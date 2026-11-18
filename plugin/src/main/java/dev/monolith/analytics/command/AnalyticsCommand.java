package dev.monolith.analytics.command;

import dev.monolith.analytics.MonolithAnalyticsPlugin;
import org.bukkit.ChatColor;
import org.bukkit.command.Command;
import org.bukkit.command.CommandExecutor;
import org.bukkit.command.CommandSender;

public class AnalyticsCommand implements CommandExecutor {

    private final MonolithAnalyticsPlugin plugin;

    public AnalyticsCommand(MonolithAnalyticsPlugin plugin) {
        this.plugin = plugin;
    }

    @Override
    public boolean onCommand(CommandSender sender, Command command, String label, String[] args) {
        if (!sender.hasPermission("monolithanalytics.admin")) {
            sender.sendMessage(ChatColor.RED + "You don't have permission to use this command.");
            return true;
        }

        if (args.length == 0) {
            sender.sendMessage(ChatColor.AQUA + "MonolithAnalytics v" + plugin.getDescription().getVersion());
            sender.sendMessage(ChatColor.GRAY + "Usage: /analytics [reload|status]");
            return true;
        }

        switch (args[0].toLowerCase()) {
            case "reload":
                plugin.reloadConfig();
                sender.sendMessage(ChatColor.GREEN + "MonolithAnalytics config reloaded.");
                break;

            case "status":
                sender.sendMessage(ChatColor.AQUA + "=== MonolithAnalytics Status ===");
                sender.sendMessage(ChatColor.GRAY + "Unique players: " + ChatColor.WHITE + plugin.getDataManager().getUniquePlayerCount());
                sender.sendMessage(ChatColor.GRAY + "Peak online: " + ChatColor.WHITE + plugin.getDataManager().getPeakOnline());
                sender.sendMessage(ChatColor.GRAY + "Current online: " + ChatColor.WHITE + plugin.getDataManager().getCurrentOnline());
                sender.sendMessage(ChatColor.GRAY + "Total joins: " + ChatColor.WHITE + plugin.getDataManager().getTotalJoins());
                sender.sendMessage(ChatColor.GRAY + "TPS samples: " + ChatColor.WHITE + plugin.getDataManager().getTpsHistory().size());
                sender.sendMessage(ChatColor.GRAY + "Memory samples: " + ChatColor.WHITE + plugin.getDataManager().getMemoryHistory().size());
                sender.sendMessage(ChatColor.GRAY + "API port: " + ChatColor.WHITE + plugin.getConfig().getInt("api.port", 7845));
                break;

            default:
                sender.sendMessage(ChatColor.RED + "Unknown subcommand. Use: /analytics [reload|status]");
                break;
        }

        return true;
    }
}
