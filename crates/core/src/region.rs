use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::{GzDecoder, ZlibDecoder};
use walkdir::WalkDir;

const SECTOR: usize = 4096;
const HEADER: usize = 2 * SECTOR;
const CHUNKS: usize = 1024;
const MAX_CHUNK_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DEPTH: usize = 512;
const COMPANIONS: [&str; 2] = ["entities", "poi"];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrimReport {
    pub chunks: u64,
    pub removed: u64,
    pub bytes_freed: u64,
    pub files_deleted: u64,
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let bytes = self.data.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(bytes)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn len(&mut self, width: usize) -> Option<usize> {
        let bytes = self.take(width)?;
        let n = if width == 2 {
            u16::from_be_bytes([bytes[0], bytes[1]]) as i64
        } else {
            i32::from_be_bytes(bytes.try_into().ok()?) as i64
        };
        usize::try_from(n).ok()
    }

    fn name(&mut self) -> Option<&'a [u8]> {
        let n = self.len(2)?;
        self.take(n)
    }

    fn skip(&mut self, tag: u8, depth: usize) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        match tag {
            1 => self.take(1).map(drop),
            2 => self.take(2).map(drop),
            3 | 5 => self.take(4).map(drop),
            4 | 6 => self.take(8).map(drop),
            7 => {
                let n = self.len(4)?;
                self.take(n).map(drop)
            }
            8 => self.name().map(drop),
            9 => {
                let kind = self.u8()?;
                let n = self.len(4)?;
                for _ in 0..n {
                    self.skip(kind, depth + 1)?;
                }
                Some(())
            }
            10 => loop {
                let kind = self.u8()?;
                if kind == 0 {
                    return Some(());
                }
                self.name()?;
                self.skip(kind, depth + 1)?;
            },
            11 => {
                let n = self.len(4)?;
                self.take(n.checked_mul(4)?).map(drop)
            }
            12 => {
                let n = self.len(4)?;
                self.take(n.checked_mul(8)?).map(drop)
            }
            _ => None,
        }
    }

    fn find_inhabited(&mut self, depth: usize) -> Option<i64> {
        loop {
            let kind = self.u8()?;
            if kind == 0 {
                return None;
            }
            let name = self.name()?;
            if kind == 4 && name == b"InhabitedTime" {
                return Some(i64::from_be_bytes(self.take(8)?.try_into().ok()?));
            }
            if kind == 10 && name == b"Level" && depth == 0 {
                if let Some(time) = self.find_inhabited(depth + 1) {
                    return Some(time);
                }
                continue;
            }
            self.skip(kind, depth + 1)?;
        }
    }
}

pub fn inhabited_time(nbt: &[u8]) -> Option<i64> {
    let mut reader = Reader { data: nbt, pos: 0 };
    if reader.u8()? != 10 {
        return None;
    }
    reader.name()?;
    reader.find_inhabited(0)
}

fn location(data: &[u8], index: usize) -> (usize, usize) {
    let bytes = &data[index * 4..index * 4 + 4];
    let offset = u32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]]) as usize;
    (offset, bytes[3] as usize)
}

fn valid(data: &[u8]) -> bool {
    data.len() >= HEADER
        && (0..CHUNKS).all(|index| {
            let (offset, count) = location(data, index);
            count == 0 || (offset >= 2 && (offset + count) * SECTOR <= data.len())
        })
}

fn chunk_nbt(data: &[u8], index: usize) -> Option<Vec<u8>> {
    let (offset, count) = location(data, index);
    let sectors = data.get(offset * SECTOR..(offset + count) * SECTOR)?;
    let length = u32::from_be_bytes(sectors.get(..4)?.try_into().ok()?) as usize;
    let payload = sectors.get(5..4usize.checked_add(length)?)?;
    let mut nbt = Vec::new();
    match sectors[4] {
        1 => GzDecoder::new(payload)
            .take(MAX_CHUNK_BYTES)
            .read_to_end(&mut nbt)
            .ok()?,
        2 => ZlibDecoder::new(payload)
            .take(MAX_CHUNK_BYTES)
            .read_to_end(&mut nbt)
            .ok()?,
        3 => return Some(payload.to_vec()),
        _ => return None,
    };
    Some(nbt)
}

pub fn unvisited(data: &[u8], min_ticks: i64) -> Option<Vec<usize>> {
    if !valid(data) {
        return None;
    }
    Some(
        (0..CHUNKS)
            .filter(|&index| location(data, index).1 > 0)
            .filter(|&index| {
                chunk_nbt(data, index)
                    .and_then(|nbt| inhabited_time(&nbt))
                    .is_some_and(|time| time < min_ticks)
            })
            .collect(),
    )
}

pub fn without(data: &[u8], remove: &[usize]) -> Option<Vec<u8>> {
    let mut out = vec![0u8; HEADER];
    for index in 0..CHUNKS {
        let (offset, count) = location(data, index);
        if count == 0 || remove.contains(&index) {
            continue;
        }
        let sector = out.len() / SECTOR;
        out.extend_from_slice(&data[offset * SECTOR..(offset + count) * SECTOR]);
        out[index * 4..index * 4 + 4]
            .copy_from_slice(&((sector as u32) << 8 | count as u32).to_be_bytes());
        let stamp = SECTOR + index * 4;
        out[stamp..stamp + 4].copy_from_slice(&data[stamp..stamp + 4]);
    }
    (out.len() > HEADER).then_some(out)
}

fn region_dirs(folders: &[PathBuf]) -> Vec<PathBuf> {
    folders
        .iter()
        .flat_map(|folder| WalkDir::new(folder).max_depth(5).into_iter().flatten())
        .filter(|entry| entry.file_type().is_dir() && entry.file_name() == "region")
        .map(|entry| entry.into_path())
        .collect()
}

fn rewrite(
    path: &Path,
    data: &[u8],
    remove: &[usize],
    apply: bool,
    report: &mut TrimReport,
) -> Result<(), String> {
    let kept = without(data, remove);
    let new_len = kept.as_ref().map_or(0, Vec::len);
    report.bytes_freed += data.len().saturating_sub(new_len) as u64;
    if kept.is_none() {
        report.files_deleted += 1;
    }
    if !apply {
        return Ok(());
    }
    match kept {
        Some(bytes) => {
            let temp = path.with_extension("mca.trim");
            fs::write(&temp, bytes)
                .and_then(|_| fs::rename(&temp, path))
                .map_err(|e| format!("Could not rewrite {}: {e}", path.display()))
        }
        None => {
            fs::remove_file(path).map_err(|e| format!("Could not delete {}: {e}", path.display()))
        }
    }
}

pub fn trim(folders: &[PathBuf], min_ticks: i64, apply: bool) -> Result<TrimReport, String> {
    let mut report = TrimReport::default();
    for dir in region_dirs(folders) {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "mca") {
                continue;
            }
            let data =
                fs::read(&path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
            let Some(remove) = unvisited(&data, min_ticks) else {
                continue;
            };
            report.chunks += (0..CHUNKS).filter(|&i| location(&data, i).1 > 0).count() as u64;
            if remove.is_empty() {
                continue;
            }
            report.removed += remove.len() as u64;
            for companion in COMPANIONS {
                let Some(parent) = dir.parent() else {
                    continue;
                };
                let other = parent.join(companion).join(entry.file_name());
                let Ok(other_data) = fs::read(&other) else {
                    continue;
                };
                if valid(&other_data) {
                    rewrite(&other, &other_data, &remove, apply, &mut report)?;
                }
            }
            rewrite(&path, &data, &remove, apply, &mut report)?;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::{GzEncoder, ZlibEncoder};
    use flate2::Compression;
    use std::io::Write;

    fn name(out: &mut Vec<u8>, kind: u8, text: &str) {
        out.push(kind);
        out.extend_from_slice(&(text.len() as u16).to_be_bytes());
        out.extend_from_slice(text.as_bytes());
    }

    fn chunk(inhabited: i64, legacy: bool) -> Vec<u8> {
        let mut out = Vec::new();
        name(&mut out, 10, "");
        name(&mut out, 8, "Status");
        out.extend_from_slice(&14u16.to_be_bytes());
        out.extend_from_slice(b"minecraft:full");
        name(&mut out, 9, "sections");
        out.push(10);
        out.extend_from_slice(&2i32.to_be_bytes());
        for y in [0u8, 1] {
            name(&mut out, 1, "Y");
            out.push(y);
            name(&mut out, 12, "data");
            out.extend_from_slice(&1i32.to_be_bytes());
            out.extend_from_slice(&[0; 8]);
            out.push(0);
        }
        name(&mut out, 11, "Heights");
        out.extend_from_slice(&2i32.to_be_bytes());
        out.extend_from_slice(&[0; 8]);
        name(&mut out, 4, "LastUpdate");
        out.extend_from_slice(&99i64.to_be_bytes());
        if legacy {
            name(&mut out, 10, "Level");
        }
        name(&mut out, 4, "InhabitedTime");
        out.extend_from_slice(&inhabited.to_be_bytes());
        if legacy {
            out.push(0);
        }
        out.push(0);
        out
    }

    fn compress(kind: u8, nbt: &[u8]) -> Vec<u8> {
        match kind {
            1 => {
                let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
                encoder.write_all(nbt).unwrap();
                encoder.finish().unwrap()
            }
            2 => {
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
                encoder.write_all(nbt).unwrap();
                encoder.finish().unwrap()
            }
            _ => nbt.to_vec(),
        }
    }

    fn region(chunks: &[(usize, u8, Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![0u8; HEADER];
        for (index, kind, nbt) in chunks {
            let payload = compress(*kind, nbt);
            let sector = out.len() / SECTOR;
            out.extend_from_slice(&(payload.len() as u32 + 1).to_be_bytes());
            out.push(*kind);
            out.extend_from_slice(&payload);
            out.resize(out.len().div_ceil(SECTOR) * SECTOR, 0);
            let count = out.len() / SECTOR - sector;
            out[index * 4..index * 4 + 4]
                .copy_from_slice(&((sector as u32) << 8 | count as u32).to_be_bytes());
            let stamp = SECTOR + index * 4;
            out[stamp..stamp + 4].copy_from_slice(&(1000 + *index as u32).to_be_bytes());
        }
        out
    }

    #[test]
    fn finds_inhabited_time_in_new_and_old_chunks() {
        assert_eq!(inhabited_time(&chunk(1234, false)), Some(1234));
        assert_eq!(inhabited_time(&chunk(-5, true)), Some(-5));
        let mut no_time = chunk(1, false);
        let at = no_time
            .windows(13)
            .position(|w| w == b"InhabitedTime")
            .unwrap();
        no_time[at] = b'X';
        assert_eq!(inhabited_time(&no_time), None);
        let full = chunk(77, false);
        for cut in 0..full.len() - 1 {
            assert_eq!(inhabited_time(&full[..cut]), None, "cut at {cut}");
        }
        assert_eq!(inhabited_time(b""), None);
        assert_eq!(
            inhabited_time(&[10, 0, 0, 9, 0, 1, b'x', 1, 0x7f, 0xff, 0xff, 0xff]),
            None
        );
    }

    #[test]
    fn finds_and_removes_unvisited_chunks() {
        let data = region(&[
            (0, 2, chunk(10, false)),
            (1, 1, chunk(100_000, false)),
            (5, 3, chunk(0, true)),
            (7, 4, chunk(0, false)),
            (9, 0x82, chunk(0, false)),
            (40, 2, b"garbage".to_vec()),
        ]);
        let remove = unvisited(&data, 1000).unwrap();
        assert_eq!(remove, vec![0, 5]);

        let trimmed = without(&data, &remove).unwrap();
        assert!(valid(&trimmed));
        assert!(trimmed.len() < data.len());
        assert_eq!(location(&trimmed, 0), (0, 0));
        assert_eq!(location(&trimmed, 5), (0, 0));
        assert_eq!(&trimmed[SECTOR..SECTOR + 4], &[0; 4]);
        assert_eq!(
            inhabited_time(&chunk_nbt(&trimmed, 1).unwrap()),
            Some(100_000)
        );
        assert_eq!(&trimmed[SECTOR + 4..SECTOR + 8], &1001u32.to_be_bytes());
        for kept in [7, 9, 40] {
            let (old_offset, count) = location(&data, kept);
            let (new_offset, new_count) = location(&trimmed, kept);
            assert_eq!(count, new_count);
            assert_eq!(
                &data[old_offset * SECTOR..(old_offset + count) * SECTOR],
                &trimmed[new_offset * SECTOR..(new_offset + count) * SECTOR]
            );
        }
        assert_eq!(unvisited(&trimmed, 1000).unwrap(), Vec::<usize>::new());
        assert_eq!(without(&data, &[0, 1, 5, 7, 9, 40]), None);
    }

    #[test]
    fn skips_broken_region_files() {
        assert_eq!(unvisited(&[0; 100], 10), None);
        let mut data = region(&[(0, 2, chunk(0, false))]);
        data[3] = 50;
        assert_eq!(unvisited(&data, 10), None);
    }

    #[test]
    fn trims_a_world_with_entities_and_poi() {
        let world = std::env::temp_dir().join(format!("monolith-trim-{}", uuid::Uuid::new_v4()));
        for dir in ["region", "entities", "poi", "DIM-1/region"] {
            fs::create_dir_all(world.join(dir)).unwrap();
        }
        let mixed = region(&[(0, 2, chunk(0, false)), (1, 2, chunk(50_000, false))]);
        let empty = region(&[(0, 2, chunk(0, false)), (3, 2, chunk(5, false))]);
        let companion = region(&[(0, 2, b"entity".to_vec()), (1, 2, b"keep".to_vec())]);
        fs::write(world.join("region/r.0.0.mca"), &mixed).unwrap();
        fs::write(world.join("entities/r.0.0.mca"), &companion).unwrap();
        fs::write(world.join("poi/r.0.0.mca"), &companion).unwrap();
        fs::write(world.join("DIM-1/region/r.1.1.mca"), &empty).unwrap();
        fs::write(world.join("region/r.9.9.mca"), b"").unwrap();
        let folders = [world.clone()];

        let preview = trim(&folders, 1200, false).unwrap();
        assert_eq!(preview.chunks, 4);
        assert_eq!(preview.removed, 3);
        assert_eq!(preview.files_deleted, 1);
        assert!(preview.bytes_freed > 0);
        assert_eq!(fs::read(world.join("region/r.0.0.mca")).unwrap(), mixed);

        let applied = trim(&folders, 1200, true).unwrap();
        assert_eq!(applied, preview);
        assert!(!world.join("DIM-1/region/r.1.1.mca").exists());
        let region_after = fs::read(world.join("region/r.0.0.mca")).unwrap();
        assert_eq!(location(&region_after, 0), (0, 0));
        assert!(location(&region_after, 1).1 > 0);
        for companion in ["entities", "poi"] {
            let after = fs::read(world.join(companion).join("r.0.0.mca")).unwrap();
            assert_eq!(location(&after, 0), (0, 0));
            assert_eq!(location(&after, 1).0, 2);
        }
        assert_eq!(trim(&folders, 1200, true).unwrap().removed, 0);
        fs::remove_dir_all(&world).unwrap();
    }
}
