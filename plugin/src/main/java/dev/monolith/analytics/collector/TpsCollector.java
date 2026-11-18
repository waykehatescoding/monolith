package dev.monolith.analytics.collector;

import dev.monolith.analytics.MonolithAnalyticsPlugin;
import org.bukkit.Bukkit;

public class TpsCollector {

    private final MonolithAnalyticsPlugin plugin;
    private long lastPollTime;

    public TpsCollector(MonolithAnalyticsPlugin plugin) {
        this.plugin = plugin;
    }

    public void start() {
        int intervalSeconds = plugin.getConfig().getInt("collection.tps-interval", 60);
        long intervalTicks = intervalSeconds * 20L;

        lastPollTime = System.nanoTime();

        Bukkit.getScheduler().runTaskTimer(plugin, () -> {
            long now = System.nanoTime();
            long elapsed = now - lastPollTime;
            double seconds = elapsed / 1_000_000_000.0;
            double tps = intervalTicks / seconds;
            tps = Math.min(tps, 20.0);
            tps = Math.round(tps * 100.0) / 100.0;

            double mspt = (elapsed / 1_000_000.0) / intervalTicks;
            mspt = Math.round(mspt * 100.0) / 100.0;

            plugin.getDataManager().addTpsSample(tps);
            plugin.getDataManager().addMsptSample(mspt);
            lastPollTime = now;
        }, intervalTicks, intervalTicks);

        tryPaperMspt();
    }

    private void tryPaperMspt() {
        try {
            Bukkit.getServer().getClass().getMethod("getAverageTickTime");
            plugin.getLogger().fine("Paper MSPT API available");
        } catch (Exception ignored) {
        }
    }
}
