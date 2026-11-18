package dev.monolith.analytics.listener;

import dev.monolith.analytics.MonolithAnalyticsPlugin;
import dev.monolith.analytics.data.DataManager;
import org.bukkit.Bukkit;
import org.bukkit.entity.Player;
import org.bukkit.event.EventHandler;
import org.bukkit.event.EventPriority;
import org.bukkit.event.Listener;
import org.bukkit.event.player.PlayerJoinEvent;
import org.bukkit.event.player.PlayerQuitEvent;

import java.net.InetSocketAddress;

public class PlayerJoinLeaveListener implements Listener {

    private final MonolithAnalyticsPlugin plugin;

    public PlayerJoinLeaveListener(MonolithAnalyticsPlugin plugin) {
        this.plugin = plugin;
    }

    @EventHandler(priority = EventPriority.MONITOR, ignoreCancelled = true)
    public void onJoin(PlayerJoinEvent event) {
        Player player = event.getPlayer();
        String uuid = player.getUniqueId().toString();
        String name = player.getName();

        DataManager dm = plugin.getDataManager();
        dm.recordJoin(uuid, name);

        if (plugin.isTrackingEnabled("track-player-versions", "stats.client-version")) {
            Bukkit.getScheduler().runTaskLaterAsynchronously(plugin, () -> {
                detectClientVersion(player, uuid, dm);
            }, 20L);
        }

        if (plugin.isTrackingEnabled("track-player-clients", "stats.client-os")) {
            Bukkit.getScheduler().runTaskLater(plugin, () -> {
                try {
                    java.lang.reflect.Method brandMethod = player.getClass().getMethod("getClientBrandName");
                    String brand = (String) brandMethod.invoke(player);
                    if (brand != null) {
                        dm.setPlayerClientBrand(uuid, brand);
                    }
                } catch (Exception ignored) {
                }
            }, 40L);
        }

        if (plugin.isTrackingEnabled("track-geolocation", "stats.geolocation")) {
            InetSocketAddress addr = player.getAddress();
            if (addr != null && addr.getAddress() != null) {
                String ip = addr.getAddress().getHostAddress();
                plugin.getGeoLocationService().lookupAsync(uuid, ip);
            }
        }
    }

    @EventHandler(priority = EventPriority.MONITOR)
    public void onQuit(PlayerQuitEvent event) {
        String uuid = event.getPlayer().getUniqueId().toString();
        plugin.getDataManager().recordLeave(uuid);
    }

    private void detectClientVersion(Player player, String uuid, DataManager dm) {
        try {
            try {
                Class<?> viaClass = Class.forName("com.viaversion.viaversion.api.Via");
                Object api = viaClass.getMethod("getAPI").invoke(null);
                Object playerVersion = api.getClass().getMethod("getPlayerVersion", Object.class).invoke(api, player);
                if (playerVersion instanceof Integer) {
                    int protocolId = (Integer) playerVersion;
                    if (protocolId > 0) {
                        try {
                            Class<?> protocolVersionClass = Class.forName("com.viaversion.viaversion.api.protocol.version.ProtocolVersion");
                            Object protocolVersion2 = protocolVersionClass.getMethod("getProtocol", int.class).invoke(null, protocolId);
                            if (protocolVersion2 != null) {
                                String versionName = protocolVersion2.toString();
                                if (versionName != null && !versionName.isEmpty() && !versionName.equals("UNKNOWN")) {
                                    dm.setPlayerClientVersionString(uuid, versionName);
                                    return;
                                }
                            }
                        } catch (Exception ignored) {
                        }

                        try {
                            Class<?> pvClass = Class.forName("com.viaversion.viaversion.api.protocol.version.ProtocolVersion");
                            java.lang.reflect.Method getNameMethod = pvClass.getMethod("getName");
                            java.lang.reflect.Method getClosest = pvClass.getMethod("getClosest", int.class);
                            Object closest = getClosest.invoke(null, protocolId);
                            if (closest != null) {
                                String name = (String) getNameMethod.invoke(closest);
                                if (name != null && !name.isEmpty()) {
                                    dm.setPlayerClientVersionString(uuid, name);
                                    return;
                                }
                            }
                        } catch (Exception ignored) {}

                    }
                }
            } catch (ClassNotFoundException ignored) {
            } catch (Exception ignored) {
            }

            try {
                Object value = Bukkit.class.getMethod("getMinecraftVersion").invoke(null);
                String serverVersion = value instanceof String ? (String) value : null;
                if (serverVersion != null && !serverVersion.isEmpty()) {
                    dm.setPlayerClientVersionString(uuid, serverVersion);
                    return;
                }
            } catch (Exception ignored) {
            }

            try {
                String bukkitVersion = Bukkit.getVersion();
                if (bukkitVersion != null) {
                    int mcStart = bukkitVersion.indexOf("MC: ");
                    if (mcStart >= 0) {
                        int mcEnd = bukkitVersion.indexOf(")", mcStart);
                        if (mcEnd > mcStart) {
                            String mcVersion = bukkitVersion.substring(mcStart + 4, mcEnd).trim();
                            if (!mcVersion.isEmpty()) {
                                dm.setPlayerClientVersionString(uuid, mcVersion);
                                return;
                            }
                        }
                    }
                }
            } catch (Exception ignored) {}

        } catch (Exception e) {
            plugin.getLogger().fine("Could not detect version for " + player.getName());
        }
    }
}
