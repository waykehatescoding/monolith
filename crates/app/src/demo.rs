use std::fs;
use std::path::Path;

use monolith_core::paths;
use monolith_core::types::ServerRecord;

struct Seed {
    id: &'static str,
    name: &'static str,
    framework: &'static str,
    version: &'static str,
    ram_mb: u32,
    gamemode: &'static str,
    difficulty: &'static str,
    max_players: u32,
    port: u32,
    motd: &'static str,
}

const SEEDS: [Seed; 5] = [
    Seed {
        id: "demo-survival",
        name: "Survival",
        framework: "Paper",
        version: "1.21.4",
        ram_mb: 4096,
        gamemode: "survival",
        difficulty: "normal",
        max_players: 20,
        port: 25565,
        motd: "Survival with friends",
    },
    Seed {
        id: "demo-creative",
        name: "Creative Build",
        framework: "Paper",
        version: "1.21.4",
        ram_mb: 3072,
        gamemode: "creative",
        difficulty: "peaceful",
        max_players: 10,
        port: 25566,
        motd: "Build anything",
    },
    Seed {
        id: "demo-modded",
        name: "Modded SMP",
        framework: "Fabric",
        version: "1.21.1",
        ram_mb: 6144,
        gamemode: "survival",
        difficulty: "hard",
        max_players: 12,
        port: 25567,
        motd: "Modded SMP",
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
        port: 25568,
        motd: "Friends only",
    },
    Seed {
        id: "demo-skyblock",
        name: "Skyblock",
        framework: "Purpur",
        version: "1.21.4",
        ram_mb: 4096,
        gamemode: "survival",
        difficulty: "normal",
        max_players: 16,
        port: 25569,
        motd: "Island life",
    },
];

pub fn prepare() {
    let dir = std::env::temp_dir().join("monolith-demo");
    std::env::set_var("MONOLITH_DATA_DIR", &dir);
    if dir.join("servers.json").exists() {
        return;
    }
    if let Err(error) = seed(&dir) {
        eprintln!("could not seed demo data: {error}");
    }
}

fn seed(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let mut records = Vec::new();
    for seed in &SEEDS {
        let path = paths::servers_dir().join(seed.name);
        fs::create_dir_all(path.join("plugins"))?;
        fs::write(path.join("eula.txt"), "eula=true\n")?;
        fs::write(path.join("server.properties"), properties(seed))?;
        records.push(ServerRecord {
            id: seed.id.into(),
            name: seed.name.into(),
            framework: seed.framework.into(),
            version: seed.version.into(),
            ram_mb: seed.ram_mb,
            players: format!("0/{}", seed.max_players),
            created_at: "2025-01-12T18:04:00.000Z".into(),
            server_path: path.to_string_lossy().into_owned(),
            eula_accepted: Some(true),
            ..ServerRecord::default()
        });
    }
    let json = serde_json::to_string_pretty(&records).map_err(std::io::Error::other)?;
    fs::write(dir.join("servers.json"), json)
}

fn properties(seed: &Seed) -> String {
    format!(
        "motd={}\nserver-port={}\nmax-players={}\ngamemode={}\ndifficulty={}\nlevel-name=world\nonline-mode=true\nview-distance=10\nwhite-list=false\n",
        seed.motd, seed.port, seed.max_players, seed.gamemode, seed.difficulty
    )
}
