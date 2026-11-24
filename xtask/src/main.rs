use anyhow::{bail, Context, Result};
use resvg::tiny_skia::{Color, Pixmap, Transform};
use resvg::usvg::{Options, Tree};
use std::fs::{self, File};
use std::path::PathBuf;

fn ground() -> Color {
    Color::from_rgba8(0x0e, 0x0f, 0x11, 255)
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn load(name: &str) -> Result<Tree> {
    let data =
        fs::read(root().join("assets/brand").join(name)).with_context(|| name.to_string())?;
    let mut options = Options::default();
    options
        .fontdb_mut()
        .load_fonts_dir(root().join("assets/fonts"));
    Ok(Tree::from_data(&data, &options)?)
}

fn canvas(w: u32, h: u32) -> Result<Pixmap> {
    let mut pixmap = Pixmap::new(w, h).context("canvas size")?;
    pixmap.fill(ground());
    Ok(pixmap)
}

fn place(pixmap: &mut Pixmap, tree: &Tree, x: f32, y: f32, scale: f32) {
    let transform = Transform::from_scale(scale, scale).post_translate(x, y);
    resvg::render(tree, transform, &mut pixmap.as_mut());
}

fn icon(mark: &Tree, size: u32) -> Result<Pixmap> {
    let mut pixmap = Pixmap::new(size, size).context("canvas size")?;
    place(&mut pixmap, mark, 0.0, 0.0, size as f32 / 128.0);
    Ok(pixmap)
}

fn save(pixmap: &Pixmap, name: &str) -> Result<()> {
    let path = root().join("assets/brand").join(name);
    pixmap.save_png(&path).with_context(|| name.to_string())?;
    println!("{}", path.display());
    Ok(())
}

fn assets() -> Result<()> {
    let mark = load("icon.svg")?;
    let wordmark = load("wordmark.svg")?;

    save(&icon(&mark, 512)?, "icon.png")?;

    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16, 24, 32, 48, 64, 128, 256] {
        let rgba = icon(&mark, size)?.take();
        dir.add_entry(ico::IconDirEntry::encode(&ico::IconImage::from_rgba_data(
            size, size, rgba,
        ))?);
    }
    dir.write(File::create(root().join("assets/brand/icon.ico"))?)?;

    let mut sidebar = canvas(164, 314)?;
    place(&mut sidebar, &mark, 50.0, 40.0, 0.5);
    save(&sidebar, "installer-sidebar.png")?;

    let mut header = canvas(500, 80)?;
    place(&mut header, &wordmark, 24.0, 16.0, 1.0);
    save(&header, "installer-header.png")?;

    let mut banner = canvas(1280, 640)?;
    place(&mut banner, &wordmark, 244.0, 224.0, 4.0);
    save(&banner, "readme-banner.png")?;
    Ok(())
}

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("assets") => assets(),
        _ => bail!("usage: cargo run -p xtask -- assets"),
    }
}
