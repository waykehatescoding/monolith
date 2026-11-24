pub enum Kind {
    Bool(bool),
    Int(i64, i64, i64),
    Float(f64, f64, f64),
    Choice(&'static str, &'static [&'static str]),
    Text(&'static str),
    Secret,
}

pub struct Spec {
    pub key: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub kind: Kind,
}

pub const fn spec(
    group: &'static str,
    key: &'static str,
    label: &'static str,
    hint: &'static str,
    kind: Kind,
) -> Spec {
    Spec {
        key,
        group,
        label,
        hint,
        kind,
    }
}

use Kind::*;

const BASICS: &str = "Basics";
const WORLD: &str = "World";
const NETWORK: &str = "Network";
const PERFORMANCE: &str = "Performance";
const ADMIN: &str = "Admin and remote access";
const PACKS: &str = "Resource pack";

pub const CATALOG: &[Spec] = &[
    spec(BASICS, "motd", "Message of the day", "Shown under the server name in the multiplayer list. Use § codes for colour and \\n for a second line.", Text("A Minecraft Server")),
    spec(BASICS, "max-players", "Max players", "How many people can be online at once.", Int(20, 1, 100_000)),
    spec(BASICS, "difficulty", "Difficulty", "How hard mobs and hunger are.", Choice("easy", &["peaceful", "easy", "normal", "hard"])),
    spec(BASICS, "gamemode", "Game mode", "What new players start as.", Choice("survival", &["survival", "creative", "adventure", "spectator"])),
    spec(BASICS, "force-gamemode", "Force game mode", "Put players back into the default game mode every time they join.", Bool(false)),
    spec(BASICS, "hardcore", "Hardcore", "Players are banned after dying once. Difficulty is locked to hard.", Bool(false)),
    spec(BASICS, "pvp", "Player fighting", "Let players damage each other.", Bool(true)),
    spec(BASICS, "allow-flight", "Allow flight", "Stop kicking players who fly with mods or plugins.", Bool(false)),
    spec(BASICS, "spawn-protection", "Spawn protection", "Radius around spawn that only operators can change. 0 turns it off.", Int(16, 0, 100_000)),
    spec(BASICS, "white-list", "Whitelist", "Only players on the whitelist can join.", Bool(false)),
    spec(BASICS, "enforce-whitelist", "Enforce whitelist", "Kick online players who are removed from the whitelist.", Bool(false)),
    spec(BASICS, "player-idle-timeout", "Idle kick", "Minutes before idle players are kicked. 0 never kicks.", Int(0, 0, 100_000)),
    spec(BASICS, "pause-when-empty-seconds", "Pause when empty", "Seconds with nobody online before the world stops ticking. 0 never pauses.", Int(60, 0, 1_000_000)),
    spec(WORLD, "level-name", "World folder", "Folder the world is loaded from. A new name creates a new world.", Text("world")),
    spec(WORLD, "level-seed", "Seed", "Seed for new worlds. Leave empty for a random one.", Text("")),
    spec(WORLD, "level-type", "World type", "Generator used for new worlds.", Choice("minecraft:normal", &["minecraft:normal", "minecraft:flat", "minecraft:large_biomes", "minecraft:amplified", "minecraft:single_biome_surface"])),
    spec(WORLD, "generator-settings", "Generator settings", "JSON settings for flat and single biome worlds.", Text("{}")),
    spec(WORLD, "generate-structures", "Structures", "Generate villages, temples and other structures.", Bool(true)),
    spec(WORLD, "allow-nether", "Nether", "Let players travel to the Nether.", Bool(true)),
    spec(WORLD, "spawn-monsters", "Monsters", "Spawn hostile mobs.", Bool(true)),
    spec(WORLD, "max-world-size", "World border radius", "Largest possible world border, in blocks.", Int(29_999_984, 1, 29_999_984)),
    spec(WORLD, "initial-enabled-packs", "Enabled data packs", "Comma-separated data packs turned on when the world is created.", Text("vanilla")),
    spec(WORLD, "initial-disabled-packs", "Disabled data packs", "Comma-separated data packs turned off when the world is created.", Text("")),
    spec(NETWORK, "server-port", "Port", "Friends connect through this port. The default is 25565.", Int(25565, 1, 65535)),
    spec(NETWORK, "server-ip", "Bind address", "Leave empty to listen on every network adapter.", Text("")),
    spec(NETWORK, "online-mode", "Verify accounts", "Turn off only for offline setups or behind a proxy. Players without a paid account can then join.", Bool(true)),
    spec(NETWORK, "enforce-secure-profile", "Require signed chat", "Only allow players whose chat messages are signed by Mojang.", Bool(true)),
    spec(NETWORK, "prevent-proxy-connections", "Block VPNs", "Kick players whose connection comes from a different IP than their login.", Bool(false)),
    spec(NETWORK, "enable-status", "Show in server list", "Answer server list pings. Off makes the server look offline.", Bool(true)),
    spec(NETWORK, "hide-online-players", "Hide player names", "Hide the player list in the server list tooltip.", Bool(false)),
    spec(NETWORK, "accepts-transfers", "Accept transfers", "Let other servers send players here with /transfer.", Bool(false)),
    spec(NETWORK, "network-compression-threshold", "Compression threshold", "Packets larger than this many bytes are compressed. -1 turns compression off.", Int(256, -1, 1_000_000)),
    spec(NETWORK, "rate-limit", "Packet rate limit", "Packets per second before a player is kicked. 0 turns it off.", Int(0, 0, 1_000_000)),
    spec(PERFORMANCE, "view-distance", "View distance", "Chunks sent to each player, from 2 to 32. Lower is faster.", Int(10, 2, 32)),
    spec(PERFORMANCE, "simulation-distance", "Simulation distance", "Chunks around players where mobs and crops update, from 3 to 32.", Int(10, 3, 32)),
    spec(PERFORMANCE, "entity-broadcast-range-percentage", "Entity range", "How far away entities are shown, as a percentage of the default.", Int(100, 10, 1000)),
    spec(PERFORMANCE, "max-tick-time", "Watchdog timeout", "Milliseconds one tick may take before the server shuts itself down. -1 turns it off.", Int(60_000, -1, i64::MAX)),
    spec(PERFORMANCE, "sync-chunk-writes", "Safe chunk saving", "Write chunks synchronously. Safer, but slower on busy servers.", Bool(true)),
    spec(PERFORMANCE, "region-file-compression", "Region compression", "How world files are compressed on disk.", Choice("deflate", &["deflate", "lz4", "none"])),
    spec(PERFORMANCE, "max-chained-neighbor-updates", "Chained update limit", "Limits redstone chain reactions. -1 removes the limit.", Int(1_000_000, -1, i64::MAX)),
    spec(ADMIN, "op-permission-level", "Operator level", "Default permission level for /op, from 1 to 4.", Int(4, 1, 4)),
    spec(ADMIN, "function-permission-level", "Function level", "Permission level data pack functions run at, from 1 to 4.", Int(2, 1, 4)),
    spec(ADMIN, "enable-command-block", "Command blocks", "Let command blocks run.", Bool(false)),
    spec(ADMIN, "broadcast-console-to-ops", "Echo console to ops", "Show console command output to online operators.", Bool(true)),
    spec(ADMIN, "broadcast-rcon-to-ops", "Echo RCON to ops", "Show RCON command output to online operators.", Bool(true)),
    spec(ADMIN, "log-ips", "Log IP addresses", "Write player IP addresses to the server log.", Bool(true)),
    spec(ADMIN, "enable-rcon", "RCON", "Allow remote console connections. Use a strong password.", Bool(false)),
    spec(ADMIN, "rcon.port", "RCON port", "Port remote consoles connect to.", Int(25575, 1, 65535)),
    spec(ADMIN, "rcon.password", "RCON password", "Password remote consoles must send.", Secret),
    spec(ADMIN, "enable-query", "Query", "Answer GameSpy4 queries used by server list sites.", Bool(false)),
    spec(ADMIN, "query.port", "Query port", "UDP port for queries.", Int(25565, 1, 65535)),
    spec(ADMIN, "enable-jmx-monitoring", "JMX monitoring", "Expose tick timings to Java monitoring tools.", Bool(false)),
    spec(PACKS, "resource-pack", "Pack URL", "Direct download link to a resource pack zip.", Text("")),
    spec(PACKS, "resource-pack-sha1", "Pack SHA-1", "Hash of the zip so players re-download it only when it changes.", Text("")),
    spec(PACKS, "resource-pack-prompt", "Prompt", "Message shown when players are asked to download the pack.", Text("")),
    spec(PACKS, "require-resource-pack", "Require pack", "Kick players who decline the resource pack.", Bool(false)),
];

const BUKKIT: &str = "bukkit.yml";
const SPIGOT: &str = "spigot.yml";
const PAPER_GLOBAL: &str = "config/paper-global.yml";
const PAPER_WORLD: &str = "config/paper-world-defaults.yml";
const PURPUR: &str = "purpur.yml";

pub const CONFIGS: &[Spec] = &[
    spec(BUKKIT, "settings.allow-end", "The End", "Let players travel to the End.", Bool(true)),
    spec(BUKKIT, "settings.connection-throttle", "Connection throttle", "Milliseconds a player must wait before reconnecting. -1 turns it off.", Int(4000, -1, 1_000_000)),
    spec(BUKKIT, "settings.shutdown-message", "Shutdown message", "Shown to players when the server stops.", Text("Server closed")),
    spec(BUKKIT, "spawn-limits.monsters", "Monster limit", "Hostile mobs per player.", Int(70, 0, 1000)),
    spec(BUKKIT, "spawn-limits.animals", "Animal limit", "Passive animals per player.", Int(10, 0, 1000)),
    spec(BUKKIT, "spawn-limits.water-animals", "Water animal limit", "Squid and dolphins per player.", Int(5, 0, 1000)),
    spec(BUKKIT, "spawn-limits.water-ambient", "Fish limit", "Fish per player.", Int(20, 0, 1000)),
    spec(BUKKIT, "spawn-limits.ambient", "Bat limit", "Bats per player.", Int(15, 0, 1000)),
    spec(BUKKIT, "ticks-per.animal-spawns", "Animal spawn interval", "Ticks between animal spawn attempts. Higher is lighter.", Int(400, 0, 100_000)),
    spec(BUKKIT, "ticks-per.monster-spawns", "Monster spawn interval", "Ticks between monster spawn attempts. Higher is lighter.", Int(1, 0, 100_000)),
    spec(BUKKIT, "ticks-per.autosave", "Autosave interval", "Ticks between world saves. 6000 is five minutes.", Int(6000, 0, 1_000_000)),
    spec(SPIGOT, "settings.restart-on-crash", "Restart script on crash", "Run the restart script when the server crashes. Monolith already restarts crashed servers.", Bool(true)),
    spec(SPIGOT, "settings.timeout-time", "Watchdog timeout", "Seconds a frozen server waits before dumping threads and stopping.", Int(60, 1, 100_000)),
    spec(SPIGOT, "settings.bungeecord", "BungeeCord", "Accept player info forwarded by a BungeeCord proxy.", Bool(false)),
    spec(SPIGOT, "world-settings.default.view-distance", "View distance", "Chunks sent to players. default uses server.properties.", Text("default")),
    spec(SPIGOT, "world-settings.default.simulation-distance", "Simulation distance", "Chunks that tick around players. default uses server.properties.", Text("default")),
    spec(SPIGOT, "world-settings.default.mob-spawn-range", "Mob spawn range", "Chunks around players where mobs can spawn.", Int(8, 1, 32)),
    spec(SPIGOT, "world-settings.default.merge-radius.item", "Item merge radius", "Blocks within which dropped items merge.", Float(2.5, 0.0, 32.0)),
    spec(SPIGOT, "world-settings.default.merge-radius.exp", "XP merge radius", "Blocks within which experience orbs merge.", Float(3.0, -1.0, 32.0)),
    spec(SPIGOT, "world-settings.default.entity-activation-range.animals", "Animal activation range", "Animals farther than this many blocks tick less often.", Int(32, 0, 512)),
    spec(SPIGOT, "world-settings.default.entity-activation-range.monsters", "Monster activation range", "Monsters farther than this many blocks tick less often.", Int(32, 0, 512)),
    spec(SPIGOT, "world-settings.default.entity-activation-range.villagers", "Villager activation range", "Villagers farther than this many blocks tick less often.", Int(32, 0, 512)),
    spec(SPIGOT, "world-settings.default.entity-activation-range.misc", "Other activation range", "Items and other entities farther than this tick less often.", Int(16, 0, 512)),
    spec(SPIGOT, "world-settings.default.item-despawn-rate", "Item despawn time", "Ticks before dropped items disappear. 6000 is five minutes.", Int(6000, 0, 1_000_000)),
    spec(PAPER_GLOBAL, "proxies.velocity.enabled", "Velocity forwarding", "Accept modern forwarding from a Velocity proxy.", Bool(false)),
    spec(PAPER_GLOBAL, "proxies.velocity.online-mode", "Velocity online mode", "Players behind Velocity must have paid accounts.", Bool(true)),
    spec(PAPER_GLOBAL, "proxies.velocity.secret", "Velocity secret", "Must match forwarding.secret on the proxy.", Secret),
    spec(PAPER_GLOBAL, "chunk-system.io-threads", "Chunk IO threads", "Threads for reading and writing chunks. -1 picks automatically.", Int(-1, -1, 256)),
    spec(PAPER_GLOBAL, "chunk-system.worker-threads", "Chunk worker threads", "Threads for generating and loading chunks. -1 picks automatically.", Int(-1, -1, 256)),
    spec(PAPER_GLOBAL, "chunk-loading-basic.player-max-chunk-load-rate", "Chunk load rate", "Chunks loaded per second for each player. -1 removes the limit.", Float(100.0, -1.0, 100_000.0)),
    spec(PAPER_GLOBAL, "misc.max-joins-per-tick", "Joins per tick", "Players let in per tick when many join at once.", Int(5, 1, 1000)),
    spec(PAPER_GLOBAL, "unsupported-settings.allow-piston-duplication", "Piston duplication", "Allow TNT, carpet and rail duplication with pistons.", Bool(false)),
    spec(PAPER_GLOBAL, "unsupported-settings.allow-headless-pistons", "Headless pistons", "Allow headless piston exploits.", Bool(false)),
    spec(PAPER_GLOBAL, "unsupported-settings.allow-permanent-block-break-exploits", "Bedrock breaking", "Allow breaking bedrock and end portal frames with exploits.", Bool(false)),
    spec(PAPER_WORLD, "chunks.auto-save-interval", "Chunk autosave interval", "Ticks between chunk saves. default uses bukkit.yml.", Text("default")),
    spec(PAPER_WORLD, "chunks.max-auto-save-chunks-per-tick", "Chunks saved per tick", "Spreads autosaves out to avoid lag spikes.", Int(24, 1, 10_000)),
    spec(PAPER_WORLD, "collisions.max-entity-collisions", "Entity collisions", "Collisions an entity processes per tick.", Int(8, 0, 1000)),
    spec(PAPER_WORLD, "entities.spawning.despawn-ranges.monster.hard", "Monster hard despawn range", "Monsters farther than this many blocks despawn instantly.", Int(128, 0, 1024)),
    spec(PAPER_WORLD, "entities.spawning.despawn-ranges.monster.soft", "Monster soft despawn range", "Monsters farther than this many blocks may despawn.", Int(32, 0, 1024)),
    spec(PAPER_WORLD, "entities.spawning.per-player-mob-spawns", "Per-player mob spawns", "Spread mob caps fairly between players.", Bool(true)),
    spec(PAPER_WORLD, "anticheat.anti-xray.enabled", "Anti-xray", "Hide ores from x-ray clients.", Bool(false)),
    spec(PAPER_WORLD, "environment.optimize-explosions", "Faster explosions", "Use a quicker explosion algorithm.", Bool(false)),
    spec(PAPER_WORLD, "misc.redstone-implementation", "Redstone engine", "Faster engines can change how some contraptions behave.", Choice("VANILLA", &["VANILLA", "EIGENCRAFT", "ALTERNATE_CURRENT"])),
    spec(PURPUR, "settings.use-alternate-keepalive", "Alternate keepalive", "Kick fewer players on bad connections.", Bool(false)),
    spec(PURPUR, "settings.server-mod-name", "Server brand", "Name shown in the F3 screen.", Text("Purpur")),
    spec(PURPUR, "world-settings.default.gameplay-mechanics.use-better-mending", "Better mending", "Mending repairs the most damaged item first.", Bool(false)),
    spec(PURPUR, "world-settings.default.mobs.villager.lobotomize.enabled", "Lobotomize villagers", "Trapped villagers stop thinking, which saves a lot of CPU.", Bool(false)),
    spec(PURPUR, "world-settings.default.gameplay-mechanics.player.idle-timeout.kick-if-idle", "Kick idle players", "Kick players after the idle timeout instead of only marking them AFK.", Bool(true)),
];

pub fn find(key: &str) -> Option<&'static Spec> {
    CATALOG.iter().find(|spec| spec.key == key)
}

impl Spec {
    pub fn valid(&self, value: &str) -> bool {
        match self.kind {
            Int(_, min, max) => value.parse::<i64>().is_ok_and(|n| (min..=max).contains(&n)),
            Float(_, min, max) => value.parse::<f64>().is_ok_and(|n| (min..=max).contains(&n)),
            _ => true,
        }
    }

    pub fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || [self.key, self.label, self.hint, self.group]
                .iter()
                .any(|s| s.to_lowercase().contains(query))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_value(spec: &Spec) -> String {
        match spec.kind {
            Bool(on) => on.to_string(),
            Int(default, ..) => default.to_string(),
            Float(default, ..) => default.to_string(),
            Choice(default, _) | Text(default) => default.to_string(),
            Secret => String::new(),
        }
    }

    #[test]
    fn catalog_keys_are_unique_and_defaults_valid() {
        for (i, spec) in CATALOG.iter().enumerate() {
            assert!(
                CATALOG[..i].iter().all(|s| s.key != spec.key),
                "{}",
                spec.key
            );
            assert!(spec.valid(&default_value(spec)), "{}", spec.key);
        }
        assert!(find("view-distance").is_some_and(|s| !s.valid("40") && s.valid("12")));
        assert!(find("network-compression-threshold").is_some_and(|s| s.valid("-1")));
    }

    #[test]
    fn config_catalog_defaults_are_valid() {
        for (i, spec) in CONFIGS.iter().enumerate() {
            assert!(
                CONFIGS[..i]
                    .iter()
                    .all(|s| s.key != spec.key || s.group != spec.group),
                "{}",
                spec.key
            );
            assert!(spec.valid(&default_value(spec)), "{}", spec.key);
        }
    }
}
