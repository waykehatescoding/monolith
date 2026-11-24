# Changelog

## 0.1.0

- Console: Ctrl+F search, real log colours including § codes, level filter, chat-only view, Tab completion, clickable links, save to file, old log and crash report browser, crash cause explanations, and mclo.gs upload.
- Settings: visual editor for every server.properties key with a MOTD preview, form editors for Bukkit, Spigot, Paper and Purpur configs, live game rules, server icon, Java GC presets and custom JVM arguments, and a restart reminder.
- Players: operator levels, ban reasons and temporary bans, IP bans, bulk whitelist and import from another server, real UUID lookup.
- Content: Hangar, Spigot and optional CurseForge sources, dependency checks, disable without deleting, and compatibility warnings before changing platform.
- Create servers from Modrinth or CurseForge modpacks, from templates, or by duplicating an existing server.
- Worlds tab: switch, create, import, export and reset worlds, manage data packs, pre-generate with Chunky and trim barely visited chunks.
- Backups: retention, locking, excluded paths, a second copy folder, backups on stop and before changing server software, and restoring into another server.
- Automation tab: scheduled tasks, triggers on joins, chat and state changes, restart when empty, idle stop, a watchdog for frozen servers, automatic server updates, and sleep mode that starts the server when someone joins.
- Network tab: UPnP port forwarding, a reachability check with a firewall rule, Geyser and Floodgate crossplay, playit.gg tunnels and a Velocity proxy wizard.
- Discord webhook alerts, RCON consoles, spark profiling, disk usage per server, a multiplayer list preview, config file search, syntax highlighting and format checks in the file editor.

## 0.0.6

- Sharing no longer gets stuck on "Creating public address". Servers with sharing on now get their public address whenever they start, and if ngrok can't create one, the reason is shown instead.

## 0.0.5

- New server platforms: Forge, NeoForge and Quilt. Monolith runs the loader's installer for you, and the Content tab installs matching mods from Modrinth.
- Modrinth search in Content and Default plugins now updates as you type.

## 0.0.4

- The console now lives on the Overview tab and fills the page, with player, TPS, memory and uptime counts above it. The separate Console tab is gone, along with its search, filters, copy and clear.
- You can scroll back through console output again. Scrolling up no longer snaps straight back to the bottom.

## 0.0.3

- The Recent console on the Overview tab now shows log colours, keeps each entry on one line, and stays a fixed height.

## 0.0.2

- The Add plugins panel in Default plugins now opens with a list of the most popular Modrinth plugins instead of an empty search box.

## 0.0.1

First release. Native Windows Minecraft server manager: create and run servers, live console, server properties, players, backups, plugin and mod installs from Modrinth, analytics plugin, tunnelling, import and export, in-app updates, and a Windows installer.
