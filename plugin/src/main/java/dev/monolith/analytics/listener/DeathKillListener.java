package dev.monolith.analytics.listener;

import dev.monolith.analytics.MonolithAnalyticsPlugin;
import org.bukkit.entity.Player;
import org.bukkit.event.EventHandler;
import org.bukkit.event.EventPriority;
import org.bukkit.event.Listener;
import org.bukkit.event.entity.PlayerDeathEvent;

public class DeathKillListener implements Listener {

    private final MonolithAnalyticsPlugin plugin;

    public DeathKillListener(MonolithAnalyticsPlugin plugin) {
        this.plugin = plugin;
    }

    @EventHandler(priority = EventPriority.MONITOR, ignoreCancelled = true)
    public void onDeath(PlayerDeathEvent event) {
        Player victim = event.getEntity();
        plugin.getDataManager().recordDeath(victim.getUniqueId().toString());

        Player killer = victim.getKiller();
        if (killer != null) {
            plugin.getDataManager().recordKill(killer.getUniqueId().toString());
        }
    }
}
