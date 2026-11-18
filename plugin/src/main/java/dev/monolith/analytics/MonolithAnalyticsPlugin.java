package dev.monolith.analytics;

import dev.monolith.analytics.collector.*;
import dev.monolith.analytics.command.AnalyticsCommand;
import dev.monolith.analytics.data.DataManager;
import dev.monolith.analytics.listener.*;
import org.bukkit.Bukkit;
import org.bukkit.plugin.java.JavaPlugin;

public class MonolithAnalyticsPlugin extends JavaPlugin {

    private static MonolithAnalyticsPlugin instance;
    private DataManager dataManager;
    private TpsCollector tpsCollector;
    private MemoryCollector memoryCollector;
    private PlayerCountCollector playerCountCollector;
    private GeoLocationService geoLocationService;

    @Override
    public void onEnable() {
        instance = this;
        saveDefaultConfig();

        dataManager = new DataManager(this);
        dataManager.loadAll();

        geoLocationService = new GeoLocationService(this);

        if (isTrackingEnabled("track-player-joins", "stats.player-join-leave")) {
            Bukkit.getPluginManager().registerEvents(new PlayerJoinLeaveListener(this), this);
        }
        if (getConfig().getBoolean("stats.chat-messages", true)) {
            Bukkit.getPluginManager().registerEvents(new ChatListener(this), this);
        }
        if (getConfig().getBoolean("stats.deaths-kills", true)) {
            Bukkit.getPluginManager().registerEvents(new DeathKillListener(this), this);
        }
        if (getConfig().getBoolean("stats.blocks", true)) {
            Bukkit.getPluginManager().registerEvents(new BlockListener(this), this);
        }
        if (getConfig().getBoolean("stats.commands", true)) {
            Bukkit.getPluginManager().registerEvents(new CommandListener(this), this);
        }

        if (isTrackingEnabled("track-player-clients", "stats.client-os")) {
            Bukkit.getPluginManager().registerEvents(new ClientBrandListener(this), this);
            try {
                Bukkit.getMessenger().registerIncomingPluginChannel(this, "minecraft:brand", new ClientBrandListener(this));
            } catch (Exception e) {
                try {
                    Bukkit.getMessenger().registerIncomingPluginChannel(this, "MC|Brand", new ClientBrandListener(this));
                } catch (Exception ignored) {
                    getLogger().fine("Could not register brand channel listener");
                }
            }
        }

        if (isTrackingEnabled("track-tps", "stats.tps-history")) {
            tpsCollector = new TpsCollector(this);
            tpsCollector.start();
        }
        if (isTrackingEnabled("track-ram", "stats.memory-history")) {
            memoryCollector = new MemoryCollector(this);
            memoryCollector.start();
        }
        if (getConfig().getBoolean("stats.player-count-timeline", true)) {
            playerCountCollector = new PlayerCountCollector(this);
            playerCountCollector.start();
        }

        int saveIntervalSeconds = getConfig().getInt("data.save-interval-seconds", 30);
        long saveIntervalTicks = saveIntervalSeconds * 20L;
        Bukkit.getScheduler().runTaskTimerAsynchronously(this, () -> dataManager.saveAll(), saveIntervalTicks, saveIntervalTicks);

        Bukkit.getScheduler().runTaskTimerAsynchronously(this, () -> dataManager.updateOnlinePlaytimes(), 600L, 600L);

        Bukkit.getScheduler().runTaskTimerAsynchronously(this, () -> dataManager.cleanupOldData(), 20L * 60, 20L * 60 * 60);

        getCommand("analytics").setExecutor(new AnalyticsCommand(this));

        getLogger().info("MonolithAnalytics v" + getDescription().getVersion() + " enabled! (file-based mode)");
    }

    @Override
    public void onDisable() {
        if (dataManager != null) {
            dataManager.saveAll();
        }

        try {
            Bukkit.getMessenger().unregisterIncomingPluginChannel(this);
        } catch (Exception ignored) {}

        getLogger().info("MonolithAnalytics disabled.");
    }

    public boolean isTrackingEnabled(String newKey, String legacyKey) {
        if (getConfig().contains("tracking." + newKey)) {
            return getConfig().getBoolean("tracking." + newKey, true);
        }
        return getConfig().getBoolean(legacyKey, true);
    }

    public static MonolithAnalyticsPlugin getInstance() {
        return instance;
    }

    public DataManager getDataManager() {
        return dataManager;
    }

    public GeoLocationService getGeoLocationService() {
        return geoLocationService;
    }
}
