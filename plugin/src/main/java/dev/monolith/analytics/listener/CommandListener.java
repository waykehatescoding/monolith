package dev.monolith.analytics.listener;

import dev.monolith.analytics.MonolithAnalyticsPlugin;
import org.bukkit.entity.Player;
import org.bukkit.event.EventHandler;
import org.bukkit.event.EventPriority;
import org.bukkit.event.Listener;
import org.bukkit.event.player.PlayerCommandPreprocessEvent;

public class CommandListener implements Listener {

    private final MonolithAnalyticsPlugin plugin;

    public CommandListener(MonolithAnalyticsPlugin plugin) {
        this.plugin = plugin;
    }

    @EventHandler(priority = EventPriority.MONITOR, ignoreCancelled = true)
    public void onCommand(PlayerCommandPreprocessEvent event) {
        Player player = event.getPlayer();
        plugin.getDataManager().recordCommand(player.getUniqueId().toString());
    }
}
