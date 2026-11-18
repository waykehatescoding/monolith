package dev.monolith.analytics.listener;

import dev.monolith.analytics.MonolithAnalyticsPlugin;
import dev.monolith.analytics.data.DataManager;
import org.bukkit.entity.Player;
import org.bukkit.plugin.messaging.PluginMessageListener;
import org.bukkit.event.EventHandler;
import org.bukkit.event.EventPriority;
import org.bukkit.event.Listener;
import org.bukkit.event.player.PlayerRegisterChannelEvent;

import java.nio.charset.StandardCharsets;

public class ClientBrandListener implements Listener, PluginMessageListener {

    private final MonolithAnalyticsPlugin plugin;

    public ClientBrandListener(MonolithAnalyticsPlugin plugin) {
        this.plugin = plugin;
    }

    @Override
    public void onPluginMessageReceived(String channel, Player player, byte[] message) {
        if (!channel.equals("minecraft:brand") && !channel.equals("MC|Brand")) {
            return;
        }

        try {
            String brand = decodeBrand(message);
            if (brand != null && !brand.isEmpty()) {
                String uuid = player.getUniqueId().toString();
                String normalized = normalizeBrand(brand);
                plugin.getDataManager().setPlayerClientBrand(uuid, normalized);

                String os = detectOsFromBrand(brand);
                if (os != null) {
                    plugin.getDataManager().setPlayerOs(uuid, os);
                }
            }
        } catch (Exception e) {
            plugin.getLogger().fine("Failed to decode brand for " + player.getName() + ": " + e.getMessage());
        }
    }

    @EventHandler(priority = EventPriority.MONITOR)
    public void onChannelRegister(PlayerRegisterChannelEvent event) {
        Player player = event.getPlayer();
        try {
            java.lang.reflect.Method brandMethod = player.getClass().getMethod("getClientBrandName");
            String brand = (String) brandMethod.invoke(player);
            if (brand != null && !brand.isEmpty()) {
                String uuid = player.getUniqueId().toString();
                plugin.getDataManager().setPlayerClientBrand(uuid, normalizeBrand(brand));
            }
        } catch (Exception ignored) {
        }
    }

    private String decodeBrand(byte[] data) {
        if (data == null || data.length == 0) return null;

        try {
            int index = 0;
            int length = 0;
            int shift = 0;
            byte b;
            do {
                if (index >= data.length) break;
                b = data[index++];
                length |= (b & 0x7F) << shift;
                shift += 7;
            } while ((b & 0x80) != 0 && shift < 35);

            if (length > 0 && index + length <= data.length) {
                return new String(data, index, length, StandardCharsets.UTF_8).trim();
            }
        } catch (Exception ignored) {}

        return new String(data, StandardCharsets.UTF_8).trim();
    }

    private String normalizeBrand(String brand) {
        if (brand == null) return "Unknown";
        String lower = brand.toLowerCase().trim();

        if (lower.contains("lunarclient") || lower.contains("lunar")) return "Lunar Client";
        if (lower.contains("badlion") || lower.contains("blc")) return "Badlion Client";
        if (lower.contains("labymod")) return "LabyMod";
        if (lower.contains("feather")) return "Feather Client";
        if (lower.contains("fabric")) return "Fabric";
        if (lower.contains("forge") || lower.contains("fml")) return "Forge";
        if (lower.contains("quilt")) return "Quilt";
        if (lower.contains("optifine")) return "OptiFine";
        if (lower.contains("iris")) return "Iris";
        if (lower.contains("sodium")) return "Sodium";
        if (lower.equals("vanilla") || lower.equals("minecraft") || lower.equals("brand")) return "Vanilla";

        return brand;
    }

    private String detectOsFromBrand(String brand) {
        if (brand == null) return null;
        String lower = brand.toLowerCase();
        if (lower.contains("windows") || lower.contains("win32") || lower.contains("win64")) return "Windows";
        if (lower.contains("macos") || lower.contains("mac os") || lower.contains("darwin")) return "macOS";
        if (lower.contains("linux") || lower.contains("ubuntu") || lower.contains("debian")) return "Linux";
        return null;
    }
}
