package dev.monolith.analytics.collector;

import dev.monolith.analytics.MonolithAnalyticsPlugin;
import org.bukkit.Bukkit;

public class PlayerCountCollector {

    private final MonolithAnalyticsPlugin plugin;

    public PlayerCountCollector(MonolithAnalyticsPlugin plugin) {
        this.plugin = plugin;
    }

    public void start() {
        int intervalSeconds = plugin.getConfig().getInt("collection.player-count-interval", 300);
        long intervalTicks = intervalSeconds * 20L;

        Bukkit.getScheduler().runTaskTimer(plugin, () -> {
            int count = Bukkit.getOnlinePlayers().size();
            plugin.getDataManager().addPlayerCountSnapshot(count);
        }, intervalTicks, intervalTicks);
    }
}
