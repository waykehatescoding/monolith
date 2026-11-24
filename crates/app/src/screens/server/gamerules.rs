use std::collections::HashMap;
use std::time::Duration;

use iced::widget::text;
use iced::{Element, Task};
use monolith_core::events::Event;
use monolith_core::server_runner::send_command;
use monolith_core::types::ServerRuntimeStatus;
use monolith_theme::style;
use monolith_theme::token::TEXT_MD;

use super::properties::{spec, Kind, Spec};
use super::settings::{integer, save_row, section, setting, spec_control};
use crate::ctx::{Ctx, Route, Tab};

const GROUP: &str = "Game rules";

use Kind::*;

pub const RULES: &[Spec] = &[
    spec(
        GROUP,
        "keepInventory",
        "Keep inventory",
        "Players keep their items and XP when they die.",
        Bool(false),
    ),
    spec(
        GROUP,
        "doDaylightCycle",
        "Daylight cycle",
        "Time of day moves forward.",
        Bool(true),
    ),
    spec(
        GROUP,
        "doWeatherCycle",
        "Weather cycle",
        "Weather changes on its own.",
        Bool(true),
    ),
    spec(
        GROUP,
        "mobGriefing",
        "Mob griefing",
        "Creepers, endermen and other mobs can change blocks.",
        Bool(true),
    ),
    spec(
        GROUP,
        "doFireTick",
        "Fire spread",
        "Fire spreads and burns out naturally.",
        Bool(true),
    ),
    spec(
        GROUP,
        "doMobSpawning",
        "Mob spawning",
        "Mobs spawn naturally.",
        Bool(true),
    ),
    spec(
        GROUP,
        "doInsomnia",
        "Phantoms",
        "Phantoms spawn when players skip sleep.",
        Bool(true),
    ),
    spec(
        GROUP,
        "doPatrolSpawning",
        "Pillager patrols",
        "Pillager patrols spawn.",
        Bool(true),
    ),
    spec(
        GROUP,
        "doTraderSpawning",
        "Wandering traders",
        "Wandering traders spawn.",
        Bool(true),
    ),
    spec(
        GROUP,
        "disableRaids",
        "Disable raids",
        "Bad Omen no longer starts raids.",
        Bool(false),
    ),
    spec(
        GROUP,
        "naturalRegeneration",
        "Natural regeneration",
        "Players heal when their hunger bar is full.",
        Bool(true),
    ),
    spec(
        GROUP,
        "doImmediateRespawn",
        "Instant respawn",
        "Skip the death screen.",
        Bool(false),
    ),
    spec(
        GROUP,
        "fallDamage",
        "Fall damage",
        "Players take damage from falling.",
        Bool(true),
    ),
    spec(
        GROUP,
        "fireDamage",
        "Fire damage",
        "Players take damage from fire and lava.",
        Bool(true),
    ),
    spec(
        GROUP,
        "drowningDamage",
        "Drowning damage",
        "Players take damage when out of air.",
        Bool(true),
    ),
    spec(
        GROUP,
        "freezeDamage",
        "Freeze damage",
        "Players take damage in powder snow.",
        Bool(true),
    ),
    spec(
        GROUP,
        "doMobLoot",
        "Mob drops",
        "Mobs drop items and XP.",
        Bool(true),
    ),
    spec(
        GROUP,
        "doTileDrops",
        "Block drops",
        "Broken blocks drop items.",
        Bool(true),
    ),
    spec(
        GROUP,
        "doEntityDrops",
        "Entity drops",
        "Minecarts, boats and item frames drop themselves.",
        Bool(true),
    ),
    spec(
        GROUP,
        "showDeathMessages",
        "Death messages",
        "Death messages appear in chat.",
        Bool(true),
    ),
    spec(
        GROUP,
        "announceAdvancements",
        "Announce advancements",
        "Advancements appear in chat.",
        Bool(true),
    ),
    spec(
        GROUP,
        "sendCommandFeedback",
        "Command feedback",
        "Players see the results of their commands.",
        Bool(true),
    ),
    spec(
        GROUP,
        "commandBlockOutput",
        "Command block output",
        "Command blocks report to operators in chat.",
        Bool(true),
    ),
    spec(
        GROUP,
        "logAdminCommands",
        "Log admin commands",
        "Operator commands are written to the server log.",
        Bool(true),
    ),
    spec(
        GROUP,
        "reducedDebugInfo",
        "Reduced debug info",
        "Hide coordinates and other details from the F3 screen.",
        Bool(false),
    ),
    spec(
        GROUP,
        "spectatorsGenerateChunks",
        "Spectators load chunks",
        "Players in spectator mode generate new terrain.",
        Bool(true),
    ),
    spec(
        GROUP,
        "tntExplodes",
        "TNT explodes",
        "TNT can be lit and explode.",
        Bool(true),
    ),
    spec(
        GROUP,
        "projectilesCanBreakBlocks",
        "Projectiles break blocks",
        "Projectiles can break blocks like decorated pots.",
        Bool(true),
    ),
    spec(
        GROUP,
        "universalAnger",
        "Universal anger",
        "Angered neutral mobs attack any nearby player.",
        Bool(false),
    ),
    spec(
        GROUP,
        "forgiveDeadPlayers",
        "Forgive dead players",
        "Angered neutral mobs calm down when their target dies.",
        Bool(true),
    ),
    spec(
        GROUP,
        "playersSleepingPercentage",
        "Players sleeping",
        "Percentage of players who must sleep to skip the night.",
        Int(100, 0, 100),
    ),
    spec(
        GROUP,
        "randomTickSpeed",
        "Random tick speed",
        "How fast crops grow and leaves decay. 0 stops it.",
        Int(3, 0, 4096),
    ),
    spec(
        GROUP,
        "spawnRadius",
        "Spawn radius",
        "Blocks around world spawn where new players appear.",
        Int(10, 0, 1000),
    ),
    spec(
        GROUP,
        "maxEntityCramming",
        "Entity cramming",
        "Mobs packed into one block before they take damage. 0 turns it off.",
        Int(24, 0, 1000),
    ),
    spec(
        GROUP,
        "snowAccumulationHeight",
        "Snow height",
        "Layers of snow that build up during snowfall.",
        Int(1, 0, 8),
    ),
];

#[derive(Default)]
pub struct State {
    server_id: Option<String>,
    live: HashMap<&'static str, String>,
    draft: HashMap<&'static str, String>,
    loading: bool,
    applying: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    Requested,
    Value(&'static str, String),
    Apply,
    Applied,
}

fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            chars.by_ref().find(char::is_ascii_alphabetic);
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_reply(line: &str) -> Option<(&str, &str)> {
    let rest = match line.strip_prefix("Gamerule ") {
        Some(rest) => rest,
        None => &line[line.find("]: Gamerule ")? + "]: Gamerule ".len()..],
    };
    let (name, rest) = rest.split_once(' ')?;
    let value = rest
        .strip_prefix("is currently set to: ")
        .or_else(|| rest.strip_prefix("is now set to: "))?;
    Some((name, value.trim()))
}

fn online(ctx: &Ctx, id: &str) -> bool {
    ctx.status(id) == ServerRuntimeStatus::Online
}

async fn send_all(id: String, commands: Vec<String>) {
    for command in commands {
        send_command(id.clone(), command).await;
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
}

pub fn on_show(state: &mut State, id: &str, ctx: &Ctx) -> Task<Message> {
    if state.server_id.as_deref() != Some(id) {
        *state = State {
            server_id: Some(id.to_string()),
            ..State::default()
        };
    }
    if online(ctx, id) && state.live.is_empty() && !state.loading {
        Task::done(Message::Refresh)
    } else {
        Task::none()
    }
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    let Some(id) = state.server_id.clone() else {
        return Task::none();
    };
    match message {
        Message::Refresh => {
            if state.loading || !online(ctx, &id) {
                return Task::none();
            }
            state.loading = true;
            let commands = RULES
                .iter()
                .map(|rule| format!("gamerule {}", rule.key))
                .collect();
            Task::perform(send_all(id, commands), |_| Message::Requested)
        }
        Message::Requested => {
            state.loading = false;
            Task::none()
        }
        Message::Value(key, value) => {
            let value = match RULES
                .iter()
                .find(|rule| rule.key == key)
                .map(|rule| &rule.kind)
            {
                Some(Int(..)) => integer(value),
                _ => value,
            };
            state.draft.insert(key, value);
            Task::none()
        }
        Message::Apply => {
            if state.applying || !online(ctx, &id) {
                return Task::none();
            }
            let commands: Vec<String> = changes(state)
                .map(|(rule, value)| format!("gamerule {} {value}", rule.key))
                .collect();
            if commands.is_empty() || invalid(state) {
                return Task::none();
            }
            state.applying = true;
            Task::perform(send_all(id, commands), |_| Message::Applied)
        }
        Message::Applied => {
            state.applying = false;
            if changes(state).next().is_none() {
                ctx.success("Game rules updated.");
            } else {
                ctx.error("Some game rules did not change. Check the console for the reason.");
            }
            Task::none()
        }
    }
}

fn changes(state: &State) -> impl Iterator<Item = (&'static Spec, &String)> {
    RULES.iter().filter_map(|rule| {
        let draft = state.draft.get(rule.key)?;
        (state.live.get(rule.key) != Some(draft)).then_some((rule, draft))
    })
}

fn invalid(state: &State) -> bool {
    changes(state).any(|(rule, value)| !rule.valid(value))
}

pub fn on_event(state: &mut State, event: &Event, ctx: &Ctx) -> Task<Message> {
    let Some(id) = state.server_id.clone() else {
        return Task::none();
    };
    match event {
        Event::ServerStatus(update) if update.server_id == id => {
            state.live.clear();
            state.draft.clear();
            let visible = ctx.route == Route::Server(Tab::Settings)
                && ctx.selected.as_deref() == Some(id.as_str());
            if update.status == ServerRuntimeStatus::Online && visible {
                return Task::done(Message::Refresh);
            }
        }
        Event::ConsoleOutput { server_id, line } if *server_id == id => {
            let text = plain(&line.text);
            let Some((name, value)) = parse_reply(&text) else {
                return Task::none();
            };
            if let Some(rule) = RULES.iter().find(|rule| rule.key == name) {
                let previous = state.live.insert(rule.key, value.to_string());
                let draft = state
                    .draft
                    .entry(rule.key)
                    .or_insert_with(|| value.to_string());
                if previous.as_deref() == Some(draft.as_str()) || state.applying {
                    *draft = value.to_string();
                }
            }
        }
        _ => {}
    }
    Task::none()
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    const TITLE: &str = "Game rules";
    let Some(id) = state.server_id.as_deref() else {
        return section(TITLE, vec![]);
    };
    if !online(ctx, id) {
        return section(
            TITLE,
            vec![text("Game rules live in the world, so Minecraft only reads and changes them while the server runs. Start the server to edit them here.")
                .size(TEXT_MD)
                .style(style::muted)
                .into()],
        );
    }
    let mut rows: Vec<Element<'a, Message>> = RULES
        .iter()
        .filter_map(|rule| {
            let value = state.draft.get(rule.key)?;
            Some(setting(
                rule.label,
                rule.hint,
                spec_control(rule, Some(value.as_str()), move |v| {
                    Message::Value(rule.key, v)
                }),
            ))
        })
        .collect();
    if rows.is_empty() {
        rows.push(
            text(if state.loading {
                "Asking the server for its game rules..."
            } else {
                "The server did not report any game rules. Check the console, then try again."
            })
            .size(TEXT_MD)
            .style(style::muted)
            .into(),
        );
        if !state.loading {
            rows.push(save_row(
                "Try again",
                "Reading",
                false,
                Some(Message::Refresh),
                None,
            ));
        }
        return section(TITLE, rows);
    }
    let invalid = invalid(state);
    rows.push(save_row(
        "Apply game rules",
        "Applying",
        state.applying,
        (changes(state).next().is_some() && !invalid).then_some(Message::Apply),
        invalid.then_some("Fix the values outlined in red before applying."),
    ));
    section(TITLE, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_gamerule_replies() {
        assert_eq!(
            parse_reply("[12:00:00 INFO]: Gamerule keepInventory is currently set to: false"),
            Some(("keepInventory", "false"))
        );
        assert_eq!(
            parse_reply("[12:00:01 INFO]: Gamerule randomTickSpeed is now set to: 10\r"),
            Some(("randomTickSpeed", "10"))
        );
        assert_eq!(
            parse_reply(&plain(
                "\x1b[0m[12:00:00 INFO]: Gamerule doFireTick is currently set to: \x1b[1mtrue\x1b[m"
            )),
            Some(("doFireTick", "true"))
        );
        assert_eq!(
            parse_reply("[12:00:02 INFO]: <Steve> Gamerule pvp is now set to: true"),
            None
        );
        assert_eq!(
            parse_reply("Gamerule pvp is now set to: true"),
            Some(("pvp", "true"))
        );
        assert_eq!(parse_reply("Unknown or incomplete command"), None);
    }

    #[test]
    fn rules_are_unique_with_valid_defaults() {
        for (i, rule) in RULES.iter().enumerate() {
            assert!(RULES[..i].iter().all(|r| r.key != rule.key), "{}", rule.key);
            let default = match rule.kind {
                Bool(on) => on.to_string(),
                Int(n, ..) => n.to_string(),
                _ => panic!("{}", rule.key),
            };
            assert!(rule.valid(&default), "{}", rule.key);
        }
    }
}
