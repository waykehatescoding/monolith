package dev.monolith.analytics.collector;

import dev.monolith.analytics.MonolithAnalyticsPlugin;
import org.bukkit.Bukkit;

public class MemoryCollector {

    private final MonolithAnalyticsPlugin plugin;

    public MemoryCollector(MonolithAnalyticsPlugin plugin) {
        this.plugin = plugin;
    }

    public void start() {
        int intervalSeconds = plugin.getConfig().getInt("collection.memory-interval", 60);
        long intervalTicks = intervalSeconds * 20L;

        Bukkit.getScheduler().runTaskTimerAsynchronously(plugin, () -> {
            Runtime runtime = Runtime.getRuntime();
            double maxMB = runtime.maxMemory() / (1024.0 * 1024.0);
            double usedMB = (runtime.totalMemory() - runtime.freeMemory()) / (1024.0 * 1024.0);
            usedMB = Math.round(usedMB * 10.0) / 10.0;
            maxMB = Math.round(maxMB * 10.0) / 10.0;

            plugin.getDataManager().addMemorySample(usedMB, maxMB);
        }, intervalTicks, intervalTicks);
    }
}
