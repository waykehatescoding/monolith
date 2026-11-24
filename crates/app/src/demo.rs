use std::collections::HashMap;
use std::fs::{self, File, FileTimes};
use std::io;
use std::os::windows::fs::FileTimesExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Local, SecondsFormat, Timelike, Utc};
use monolith_core::events::{self, Event};
use monolith_core::paths;
use monolith_core::server_runner;
use monolith_core::types::{
    BackupConfig, ConsoleLine, ConsoleLineType, ModrinthInstallEntry, ModrinthProjectType,
    ServerRecord, ServerRuntimeStatus, ServerStats,
};
use serde_json::json;

static ACTIVE: AtomicBool = AtomicBool::new(false);

const SURVIVAL: &str = "demo-survival";
const CREATIVE: &str = "demo-creative";
const MODDED: &str = "demo-modded";
const SKYBLOCK: &str = "demo-skyblock";

const SURVIVAL_PLAYERS: [&str; 7] = [
    "Steve",
    "Alex",
    "Notch_fan",
    "BuilderBee",
    "CreeperSlayer",
    "Mina_K",
    "RedstoneRob",
];
const CREATIVE_PLAYERS: [&str; 2] = ["PixelPaula", "Architect_Jo"];

struct Seed {
    id: &'static str,
    name: &'static str,
    framework: &'static str,
    version: &'static str,
    ram_mb: u32,
    gamemode: &'static str,
    difficulty: &'static str,
    max_players: u32,
    online: u32,
    port: u32,
    motd: &'static str,
    status: ServerRuntimeStatus,
}

const SEEDS: [Seed; 5] = [
    Seed {
        id: SURVIVAL,
        name: "Survival",
        framework: "Paper",
        version: "1.21.4",
        ram_mb: 4096,
        gamemode: "survival",
        difficulty: "normal",
        max_players: 20,
        online: 7,
        port: 25565,
        motd: "Survival with friends",
        status: ServerRuntimeStatus::Online,
    },
    Seed {
        id: CREATIVE,
        name: "Creative Build",
        framework: "Paper",
        version: "1.21.4",
        ram_mb: 3072,
        gamemode: "creative",
        difficulty: "peaceful",
        max_players: 10,
        online: 2,
        port: 25566,
        motd: "Build anything",
        status: ServerRuntimeStatus::Online,
    },
    Seed {
        id: MODDED,
        name: "Modded SMP",
        framework: "Fabric",
        version: "1.21.1",
        ram_mb: 6144,
        gamemode: "survival",
        difficulty: "hard",
        max_players: 12,
        online: 0,
        port: 25567,
        motd: "Modded SMP",
        status: ServerRuntimeStatus::Starting,
    },
    Seed {
        id: "demo-friends",
        name: "Friends Realm",
        framework: "Vanilla",
        version: "1.21.4",
        ram_mb: 2048,
        gamemode: "survival",
        difficulty: "easy",
        max_players: 8,
        online: 0,
        port: 25568,
        motd: "Friends only",
        status: ServerRuntimeStatus::Offline,
    },
    Seed {
        id: SKYBLOCK,
        name: "Skyblock",
        framework: "Purpur",
        version: "1.21.4",
        ram_mb: 4096,
        gamemode: "survival",
        difficulty: "normal",
        max_players: 16,
        online: 0,
        port: 25569,
        motd: "Island life",
        status: ServerRuntimeStatus::Crashed,
    },
];

struct Plugin {
    project_id: &'static str,
    title: &'static str,
    slug: &'static str,
    file: &'static str,
}

const PLUGINS: [Plugin; 4] = [
    Plugin {
        project_id: "hXiIvTyT",
        title: "EssentialsX",
        slug: "essentialsx",
        file: "EssentialsX-2.21.0.jar",
    },
    Plugin {
        project_id: "Vebnzrzj",
        title: "LuckPerms",
        slug: "luckperms",
        file: "LuckPerms-Bukkit-5.4.150.jar",
    },
    Plugin {
        project_id: "1u6JkXh5",
        title: "WorldEdit",
        slug: "worldedit",
        file: "worldedit-bukkit-7.3.9.jar",
    },
    Plugin {
        project_id: "l6YH9Als",
        title: "spark",
        slug: "spark",
        file: "spark-1.10.119-bukkit.jar",
    },
];

const OUTDATED_PROJECT: &str = "Vebnzrzj";

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

pub fn uptime(id: &str) -> Duration {
    if !active() {
        return Duration::ZERO;
    }
    match id {
        SURVIVAL => Duration::from_secs(3 * 3600 + 12 * 60 + 40),
        CREATIVE => Duration::from_secs(41 * 60 + 8),
        MODDED => Duration::from_secs(14),
        _ => Duration::ZERO,
    }
}

pub fn latest_versions(entries: &[ModrinthInstallEntry]) -> Option<HashMap<String, String>> {
    active().then(|| {
        entries
            .iter()
            .map(|entry| {
                let latest = if entry.project_id == OUTDATED_PROJECT {
                    format!("{}-newer", entry.version_id)
                } else {
                    entry.version_id.clone()
                };
                (entry.project_id.clone(), latest)
            })
            .collect()
    })
}

pub fn prepare() {
    let dir = std::env::temp_dir().join("monolith-demo");
    std::env::set_var("MONOLITH_DATA_DIR", &dir);
    ACTIVE.store(true, Ordering::Relaxed);
    if let Err(error) = fs::remove_dir_all(&dir) {
        if error.kind() != io::ErrorKind::NotFound {
            eprintln!("could not reset demo data: {error}");
        }
    }
    if let Err(error) = seed(&dir) {
        eprintln!("could not seed demo data: {error}");
        return;
    }
    seed_logs();
}

fn seed(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let mut records = Vec::new();
    for seed in &SEEDS {
        let path = paths::servers_dir().join(seed.name);
        fs::create_dir_all(path.join("plugins"))?;
        fs::write(path.join("eula.txt"), "eula=true\n")?;
        fs::write(path.join("server.properties"), properties(seed))?;
        let mut record = ServerRecord {
            id: seed.id.into(),
            name: seed.name.into(),
            framework: seed.framework.into(),
            version: seed.version.into(),
            ram_mb: seed.ram_mb,
            status: seed.status,
            players: format!("{}/{}", seed.online, seed.max_players),
            created_at: "2025-01-12T18:04:00.000Z".into(),
            server_path: path.to_string_lossy().into_owned(),
            eula_accepted: Some(true),
            analytics_enabled: (seed.framework == "Paper").then_some(true),
            ..ServerRecord::default()
        };
        if seed.id == SURVIVAL {
            record.backup_config = Some(BackupConfig {
                enabled: true,
                interval_hours: 6.0,
                last_backup_at: Some(iso(ago(2 * 3600))),
                ..BackupConfig::default()
            });
            seed_survival(&path)?;
        }
        if seed.id == CREATIVE {
            seed_analytics(&path, 5, 2)?;
        }
        records.push(record);
    }
    let json = serde_json::to_string_pretty(&records).map_err(io::Error::other)?;
    fs::write(dir.join("servers.json"), json)?;
    seed_default_plugins(dir)
}

fn properties(seed: &Seed) -> String {
    format!(
        "motd={}\nserver-port={}\nmax-players={}\ngamemode={}\ndifficulty={}\nlevel-name=world\nonline-mode=true\nview-distance=10\nwhite-list={}\n",
        seed.motd,
        seed.port,
        seed.max_players,
        seed.gamemode,
        seed.difficulty,
        seed.id == SURVIVAL
    )
}

fn name_list(names: &[&str]) -> io::Result<String> {
    let entries: Vec<_> = names
        .iter()
        .map(|name| json!({ "uuid": "", "name": name }))
        .collect();
    serde_json::to_string_pretty(&entries).map_err(io::Error::other)
}

fn seed_survival(path: &Path) -> io::Result<()> {
    fs::write(
        path.join("whitelist.json"),
        name_list(&[
            "Steve",
            "Alex",
            "Notch_fan",
            "BuilderBee",
            "CreeperSlayer",
            "Mina_K",
            "RedstoneRob",
            "Dana_D",
            "Wolfgang",
        ])?,
    )?;
    fs::write(
        path.join("banned-players.json"),
        name_list(&["xX_Griefer_Xx", "TNT_Tom"])?,
    )?;

    let plugins = path.join("plugins");
    let installed = iso(ago(9 * 86_400));
    let mut manifest = Vec::new();
    for plugin in &PLUGINS {
        fs::write(plugins.join(plugin.file), b"PK")?;
        manifest.push(ModrinthInstallEntry {
            source: Default::default(),
            disabled: false,
            project_id: plugin.project_id.into(),
            version_id: format!("demo-{}", plugin.slug),
            file_name: plugin.file.into(),
            title: plugin.title.into(),
            slug: Some(plugin.slug.into()),
            icon_url: None,
            project_type: ModrinthProjectType::Plugin,
            loader: Some("paper".into()),
            game_version: Some("1.21.4".into()),
            installed_at: installed.clone(),
        });
    }
    let meta = path.join(".monolith");
    fs::create_dir_all(&meta)?;
    fs::write(
        meta.join("modrinth.json"),
        serde_json::to_string_pretty(&manifest).map_err(io::Error::other)?,
    )?;

    let backups = path.join("backups");
    fs::create_dir_all(&backups)?;
    for (seconds_ago, label, megabytes) in [
        (2 * 3600, "auto", 48),
        (26 * 3600, "Before_1.21.4_upgrade", 47),
        (3 * 86_400 + 3600, "auto", 31),
    ] {
        let at = ago(seconds_ago);
        let stamp = DateTime::<Utc>::from(at).format("%Y-%m-%dT%H-%M-%S-000Z");
        backup(
            &backups.join(format!("backup-{stamp}-{label}.zip")),
            at,
            megabytes,
        )?;
    }

    seed_analytics(path, 12, 7)
}

fn backup(path: &Path, at: SystemTime, megabytes: u64) -> io::Result<()> {
    let file = File::create(path)?;
    file.set_len(megabytes * 1024 * 1024)?;
    file.set_times(FileTimes::new().set_modified(at).set_created(at))
}

fn seed_analytics(path: &Path, peak: u32, current: u32) -> io::Result<()> {
    let samples: u32 = 144;
    let top = f64::from(peak);
    let mut tps = Vec::new();
    let mut memory = Vec::new();
    let mut timeline = Vec::new();
    for index in (0..samples).rev() {
        let at: DateTime<Local> = ago(u64::from(index) * 600).into();
        let stamp = at
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let step = f64::from(samples - index);
        let hour = f64::from(at.hour()) + f64::from(at.minute()) / 60.0;
        let daily = 0.5 + 0.5 * ((hour - 15.0) / 24.0 * std::f64::consts::TAU).cos();
        let online = (top * daily + (step * 1.7).sin()).round().clamp(0.0, top);
        let strain = ((step * 0.37).sin() * (step * 0.11).cos()).max(0.0);
        tps.push(json!({
            "timestamp": stamp,
            "tps": (20.0 - strain * 2.6 * online / top).clamp(15.0, 20.0),
        }));
        memory.push(json!({
            "timestamp": stamp,
            "usedMB": 1500.0 + 1000.0 * ((step % 22.0) / 22.0) + 120.0 * (step * 0.9).sin() + online * 18.0,
        }));
        timeline.push(json!({ "timestamp": stamp, "players": online }));
    }
    let names = [
        "Steve",
        "Alex",
        "BuilderBee",
        "Notch_fan",
        "Mina_K",
        "CreeperSlayer",
        "RedstoneRob",
        "Dana_D",
    ];
    let players: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let rank = (names.len() - index) as u64;
            json!({
                "name": name,
                "joinCount": 9 + rank * 6,
                "totalPlayTimeSeconds": 3600 * rank * 5 + 1260,
            })
        })
        .collect();
    let report = json!({
        "overview": {
            "currentOnline": current,
            "peakOnline": peak,
            "uniquePlayers": 23,
            "totalJoins": 214,
            "totalChatMessages": 1873,
            "totalCommandsExecuted": 642,
            "totalDeaths": 58,
            "currentTps": 19.9,
            "memoryUsedMB": 2457,
            "memoryMaxMB": 4096,
        },
        "players": players,
        "tps": tps,
        "memory": memory,
        "timeline": timeline,
    });
    let data = path.join("plugins").join("MonolithAnalytics").join("data");
    fs::create_dir_all(&data)?;
    fs::write(
        data.join("analytics.json"),
        serde_json::to_string(&report).map_err(io::Error::other)?,
    )
}

fn seed_default_plugins(dir: &Path) -> io::Result<()> {
    let added = iso(ago(20 * 86_400));
    let entry = |project_id: &str, title: &str, slug: &str, group: &str| {
        json!({
            "source": "modrinth",
            "projectId": project_id,
            "title": title,
            "slug": slug,
            "addedAt": added,
            "group": group,
        })
    };
    let plugins = [
        entry("Vebnzrzj", "LuckPerms", "luckperms", "General"),
        entry("hXiIvTyT", "EssentialsX", "essentialsx", "General"),
        entry("l6YH9Als", "spark", "spark", "Performance"),
        entry("fALzjamp", "Chunky", "chunky", "Performance"),
    ];
    fs::write(
        dir.join("default-plugins.json"),
        serde_json::to_string_pretty(&plugins).map_err(io::Error::other)?,
    )
}

fn ago(seconds: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(seconds)
}

fn iso(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn console_line(seconds_ago: u64, level: &str, message: &str) -> ConsoleLine {
    let at = ago(seconds_ago);
    let clock = DateTime::<Local>::from(at).format("%H:%M:%S");
    let (text, kind) = match level {
        "RAW" => (message.to_string(), ConsoleLineType::Stderr),
        "WARN" => (
            format!("\x1b[33m[{clock} WARN]: {message}\x1b[m"),
            ConsoleLineType::Stdout,
        ),
        "ERROR" => (
            format!("\x1b[31m[{clock} ERROR]: {message}\x1b[m"),
            ConsoleLineType::Stderr,
        ),
        _ => (
            format!("[{clock} {level}]: {message}"),
            ConsoleLineType::Stdout,
        ),
    };
    ConsoleLine {
        timestamp: iso(at),
        text,
        kind,
    }
}

const SURVIVAL_LOG: &[(u64, &str, &str)] = &[
    (11520, "INFO", "Starting minecraft server version 1.21.4"),
    (11520, "INFO", "Loading properties"),
    (11519, "INFO", "This server is running Paper version 1.21.4-232-main@a1b2c3d (Implementing API version 1.21.4-R0.1-SNAPSHOT)"),
    (11519, "INFO", "Server Ping Player Sample Count: 12"),
    (11518, "INFO", "Using 4 threads for Netty based IO"),
    (11516, "INFO", "Preparing level \"world\""),
    (11515, "INFO", "[LuckPerms] Enabling LuckPerms v5.4.150"),
    (11514, "INFO", "[Essentials] Enabling Essentials v2.21.0"),
    (11514, "INFO", "[WorldEdit] Enabling WorldEdit v7.3.9+6926a4d"),
    (11513, "INFO", "[spark] Enabling spark v1.10.119"),
    (11512, "INFO", "[MonolithAnalytics] Sampling TPS and memory every 60 seconds"),
    (11511, "INFO", "Preparing spawn area: 100%"),
    (11510, "INFO", "Time elapsed: 1843 ms"),
    (11509, "INFO", "Done (6.412s)! For help, type \"help\""),
    (10830, "INFO", "Steve[/192.168.1.24:51822] logged in with entity id 112 at ([world]102.5, 70.0, -310.2)"),
    (10829, "INFO", "Steve joined the game"),
    (10790, "INFO", "<Steve> morning everyone"),
    (9960, "INFO", "Alex[/192.168.1.31:60418] logged in with entity id 187 at ([world]88.1, 64.0, -271.9)"),
    (9959, "INFO", "Alex joined the game"),
    (9930, "INFO", "<Alex> o/ finished the iron farm last night"),
    (9100, "INFO", "BuilderBee has made the advancement [Stone Age]"),
    (8460, "INFO", "Notch_fan[/10.0.0.12:49233] logged in with entity id 244 at ([world]-12.5, 72.0, 40.5)"),
    (8459, "INFO", "Notch_fan joined the game"),
    (7240, "WARN", "Can't keep up! Is the server overloaded? Running 2312ms or 46 ticks behind"),
    (6020, "INFO", "CreeperSlayer[/172.16.4.8:55102] logged in with entity id 301 at ([world]410.0, 66.0, 118.7)"),
    (6019, "INFO", "CreeperSlayer joined the game"),
    (5400, "INFO", "<Notch_fan> anyone up for a nether trip?"),
    (5380, "INFO", "<Steve> give me 10 min, restocking"),
    (4310, "INFO", "Mina_K has made the advancement [Getting an Upgrade]"),
    (3900, "INFO", "Dana_D left the game"),
    (3300, "INFO", "RedstoneRob[/192.168.1.55:50871] logged in with entity id 366 at ([world]-201.3, 63.0, -44.0)"),
    (3299, "INFO", "RedstoneRob joined the game"),
    (2400, "INFO", "<RedstoneRob> who broke my hopper line"),
    (2380, "INFO", "<Alex> not me, check the logs"),
    (1500, "INFO", "BuilderBee has made the advancement [Monster Hunter]"),
    (900, "INFO", "Mina_K[/192.168.1.77:52044] logged in with entity id 412 at ([world]22.0, 68.0, 5.5)"),
    (899, "INFO", "Mina_K joined the game"),
    (860, "INFO", "<Mina_K> hi all"),
    (420, "INFO", "[Essentials] CONSOLE issued server command: /save-all"),
    (419, "INFO", "Saving the game (this may take a moment!)"),
    (417, "INFO", "Saved the game"),
    (180, "INFO", "<CreeperSlayer> the sunrise from the new tower is unreal"),
    (45, "INFO", "BuilderBee has made the advancement [Local Brewery]"),
];

const SURVIVAL_LIVE: &[(&str, &str)] = &[
    ("INFO", "<Steve> found diamonds at -310 -54 88"),
    ("INFO", "Alex has made the advancement [Diamonds!]"),
    ("INFO", "<Mina_K> nice, sharing?"),
    ("INFO", "<BuilderBee> the bridge needs more lanterns"),
    ("INFO", "RedstoneRob was slain by Zombie"),
    ("INFO", "<RedstoneRob> ok that one was my fault"),
    (
        "INFO",
        "Notch_fan has made the advancement [Cover Me with Diamonds]",
    ),
    ("INFO", "<CreeperSlayer> who wants to raid the stronghold"),
    ("INFO", "Saving the game (this may take a moment!)"),
    ("INFO", "Saved the game"),
    (
        "WARN",
        "Can't keep up! Is the server overloaded? Running 1840ms or 36 ticks behind",
    ),
];

const CREATIVE_LOG: &[(u64, &str, &str)] = &[
    (2468, "INFO", "Starting minecraft server version 1.21.4"),
    (2466, "INFO", "This server is running Paper version 1.21.4-232-main@a1b2c3d (Implementing API version 1.21.4-R0.1-SNAPSHOT)"),
    (2462, "INFO", "[WorldEdit] Enabling WorldEdit v7.3.9+6926a4d"),
    (2459, "INFO", "Done (4.907s)! For help, type \"help\""),
    (1800, "INFO", "PixelPaula joined the game"),
    (1790, "INFO", "<PixelPaula> starting on the castle walls"),
    (900, "INFO", "Architect_Jo joined the game"),
    (860, "INFO", "<Architect_Jo> //set stone_bricks done"),
];

const CREATIVE_LIVE: &[(&str, &str)] = &[
    ("INFO", "<PixelPaula> that tower looks huge"),
    ("INFO", "<Architect_Jo> //replace stone mossy_stone_bricks"),
    ("INFO", "Architect_Jo issued server command: /time set day"),
];

const MODDED_LOG: &[(u64, &str, &str)] = &[
    (
        13,
        "INFO",
        "Loading Minecraft 1.21.1 with Fabric Loader 0.16.9",
    ),
    (12, "INFO", "Loading 41 mods:"),
    (11, "INFO", "\t- fabric-api 0.105.0+1.21.1"),
    (11, "INFO", "\t- lithium 0.13.1"),
    (10, "INFO", "\t- ferritecore 7.0.1"),
    (8, "INFO", "Environment: Minecraft 1.21.1 [SERVER]"),
];

const MODDED_LIVE: &[(&str, &str)] = &[
    ("INFO", "Loading Fabric API modules"),
    ("INFO", "Applying mixins for 38 mods"),
    ("INFO", "Reloading ResourceManager: vanilla, fabric"),
    ("INFO", "Loaded 1290 recipes"),
    ("INFO", "Preparing level \"world\""),
    (
        "INFO",
        "Preparing start region for dimension minecraft:overworld",
    ),
];

const SKYBLOCK_LOG: &[(u64, &str, &str)] = &[
    (2700, "INFO", "Starting minecraft server version 1.21.4"),
    (
        2690,
        "INFO",
        "This server is running Purpur version 1.21.4-2306-HEAD@d4e5f6a",
    ),
    (2682, "INFO", "Done (7.318s)! For help, type \"help\""),
    (
        1805,
        "WARN",
        "Can't keep up! Is the server overloaded? Running 5120ms or 102 ticks behind",
    ),
    (
        1790,
        "WARN",
        "Can't keep up! Is the server overloaded? Running 8344ms or 166 ticks behind",
    ),
    (1783, "ERROR", "Encountered an unexpected exception"),
    (1783, "RAW", "java.lang.OutOfMemoryError: Java heap space"),
    (
        1783,
        "RAW",
        "\tat java.base/java.util.Arrays.copyOf(Arrays.java:3481)",
    ),
    (
        1783,
        "RAW",
        "\tat net.minecraft.world.level.chunk.storage.RegionFile.read(RegionFile.java:204)",
    ),
    (
        1783,
        "RAW",
        "\tat net.minecraft.server.level.ChunkMap.lambda$scheduleChunkLoad$22(ChunkMap.java:611)",
    ),
    (1782, "INFO", "Stopping server"),
    (1781, "INFO", "Saving players"),
];

fn seed_logs() {
    for (id, log) in [
        (SURVIVAL, SURVIVAL_LOG),
        (CREATIVE, CREATIVE_LOG),
        (MODDED, MODDED_LOG),
        (SKYBLOCK, SKYBLOCK_LOG),
    ] {
        for (seconds_ago, level, message) in log {
            server_runner::push_console_line(id, console_line(*seconds_ago, level, message));
        }
    }
}

fn wave(tick: u64, speed: f64) -> f64 {
    (tick as f64 * speed).sin()
}

fn stats(id: &str, tick: u64) -> ServerStats {
    let (names, max_players, tps, memory, max_memory): (&[&str], u32, f64, f64, f64) =
        if id == SURVIVAL {
            (
                &SURVIVAL_PLAYERS,
                20,
                19.9 - 0.2 * wave(tick, 1.3).abs() - 0.1 * wave(tick, 0.4).max(0.0),
                2410.0 + 70.0 * wave(tick, 0.21) + 25.0 * wave(tick, 1.7),
                4096.0,
            )
        } else {
            (
                &CREATIVE_PLAYERS,
                10,
                20.0 - 0.05 * wave(tick, 0.9).abs(),
                1180.0 + 40.0 * wave(tick, 0.17),
                3072.0,
            )
        };
    ServerStats {
        server_id: id.into(),
        tps: Some(tps.min(20.0)),
        memory_used_mb: Some(memory),
        memory_max_mb: Some(max_memory),
        online_players: names.iter().map(|name| name.to_string()).collect(),
        player_count: names.len() as u32,
        max_players,
    }
}

fn trickle(id: &str, list: &[(&str, &str)], count: u64) {
    if let Some((level, message)) = list.get(count as usize % list.len()) {
        server_runner::push_console_line(id, console_line(0, level, message));
    }
}

pub async fn simulate() {
    let mut interval = tokio::time::interval(Duration::from_millis(1500));
    let mut tick: u64 = 0;
    let mut loading: u64 = 0;
    loop {
        interval.tick().await;
        for id in [SURVIVAL, CREATIVE] {
            events::emit(Event::ServerStats(stats(id, tick)));
        }
        if tick % 3 == 1 {
            trickle(SURVIVAL, SURVIVAL_LIVE, tick / 3);
        }
        if tick % 8 == 5 {
            trickle(CREATIVE, CREATIVE_LIVE, tick / 8);
        }
        if tick % 4 == 2 && loading < MODDED_LIVE.len() as u64 {
            trickle(MODDED, MODDED_LIVE, loading);
            loading += 1;
        }
        tick += 1;
    }
}
