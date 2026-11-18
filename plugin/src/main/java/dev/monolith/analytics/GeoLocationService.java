package dev.monolith.analytics;

import com.google.gson.Gson;
import dev.monolith.analytics.data.DataManager;
import org.bukkit.Bukkit;

import java.io.InputStreamReader;
import java.net.HttpURLConnection;
import java.net.InetAddress;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.time.Instant;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.logging.Level;

public class GeoLocationService {

    private final MonolithAnalyticsPlugin plugin;
    private final Gson gson = new Gson();
    private final AtomicInteger requestsThisMinute = new AtomicInteger(0);
    private volatile long minuteStart = System.currentTimeMillis();

    public GeoLocationService(MonolithAnalyticsPlugin plugin) {
        this.plugin = plugin;
    }

    public void lookupAsync(String playerUuid, String ipAddress) {
        if (!isPublicAddress(ipAddress)) {
            return;
        }

        DataManager dm = plugin.getDataManager();
        DataManager.GeoData cached = dm.getCachedGeo(ipAddress);
        int cacheDuration = plugin.getConfig().getInt("geolocation.cache-minutes", 1440);
        if (cached != null && (Instant.now().getEpochSecond() - cached.cachedAt) < cacheDuration * 60L) {
            dm.setPlayerGeo(playerUuid, cached);
            return;
        }

        Bukkit.getScheduler().runTaskAsynchronously(plugin, () -> {
            try {
                long now = System.currentTimeMillis();
                if (now - minuteStart > 60000) {
                    minuteStart = now;
                    requestsThisMinute.set(0);
                }
                int rateLimit = plugin.getConfig().getInt("geolocation.rate-limit", 40);
                if (requestsThisMinute.incrementAndGet() > rateLimit) {
                    return;
                }

                String apiUrl = plugin.getConfig().getString("geolocation.api-url",
                        "https://ipapi.co/{ip}/json/");
                String url = apiUrl.replace("{ip}", ipAddress);

                HttpURLConnection conn = (HttpURLConnection) new URL(url).openConnection();
                conn.setRequestMethod("GET");
                conn.setConnectTimeout(5000);
                conn.setReadTimeout(5000);
                conn.setRequestProperty("User-Agent", "MonolithAnalytics/1.0");

                if (conn.getResponseCode() == 200) {
                    try (InputStreamReader reader = new InputStreamReader(conn.getInputStream(), StandardCharsets.UTF_8)) {
                        DataManager.GeoData geo = gson.fromJson(reader, DataManager.GeoData.class);
                        if (geo != null && geo.isSuccessful()) {
                            geo.normalizeProviderFields();
                            geo.cachedAt = Instant.now().getEpochSecond();
                            dm.cacheGeo(ipAddress, geo);
                            dm.setPlayerGeo(playerUuid, geo);
                        }
                    }
                }
                conn.disconnect();
            } catch (Exception e) {
                plugin.getLogger().log(Level.FINE, "Geo lookup failed for " + ipAddress, e);
            }
        });
    }

    private boolean isPublicAddress(String ipAddress) {
        if (ipAddress == null || ipAddress.isEmpty()) {
            return false;
        }

        try {
            InetAddress address = InetAddress.getByName(ipAddress);
            if (
                    address.isAnyLocalAddress() ||
                    address.isLoopbackAddress() ||
                    address.isLinkLocalAddress() ||
                    address.isSiteLocalAddress() ||
                    address.isMulticastAddress()
            ) {
                return false;
            }

            String normalized = address.getHostAddress().toLowerCase();
            return !(normalized.startsWith("fc") || normalized.startsWith("fd"));
        } catch (Exception ignored) {
            return false;
        }
    }
}
