![Monolith](assets/brand/readme-banner.png)

Monolith is a native Windows Minecraft server manager written in Rust.

## Install

Download `Monolith-Setup-<version>.exe` from [Releases](https://github.com/waykehatescoding/monolith/releases) and run it.

## Build from source

Requires Rust stable and JDK 17-21 (for the plugin).

```
cargo build --release -p monolith
cd plugin && ./gradlew build
cargo run -p xtask -- assets
```

Installer:

```
$env:MONOLITH_PAYLOAD_EXE = "$PWD/target/release/monolith.exe"
$env:MONOLITH_PAYLOAD_PLUGIN = "$PWD/assets/plugins/MonolithAnalytics.jar"
cargo build --release -p monolith-setup
```

## Test

```
cargo test --workspace
```

## License

GPL-3.0-only. See [LICENSE](LICENSE).
