# MonolithAnalytics

Paper/Purpur plugin that records server analytics (players, TPS, memory, deaths, blocks, chat, commands, geolocation, client brand) and serves them over a small REST API for the Monolith app.

Config is created at `plugins/MonolithAnalytics/config.yml`.

## Build

Requires JDK 17 to 21.

```
gradlew.bat build
```

Output: `build/libs/MonolithAnalytics.jar`
