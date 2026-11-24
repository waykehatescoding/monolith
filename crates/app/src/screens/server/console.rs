use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::text::Span;
use iced::widget::{
    button, column, container, rich_text, row, scrollable, span, stack, text, text_input, tooltip,
    Space,
};
use iced::{event, keyboard, Alignment, Color, Element, Length, Subscription, Task};
use monolith_core::events::Event;
use monolith_core::log_filter::{filter_logs, line_level, LogLevel, LogView};
use monolith_core::server_runner;
use monolith_core::types::{ConsoleLine, ConsoleLineType, ServerRuntimeStatus};
use monolith_theme::style;
use monolith_theme::token::{
    AMBER, CORAL, INK, INK_SECONDARY, LOG_INFO, MOSS, SPACE_1, SPACE_2, SPACE_3, SPACE_4, TEXT_MD,
    TEXT_SM, TEXT_XS,
};
use monolith_theme::token::{BUTTON_MD, BUTTON_SM};
use monolith_theme::widget::{sized, sized_icon};
use monolith_theme::{icon_colored, Icon, MONO, UI_MEDIUM};

use crate::ctx::{Ctx, Route, Tab};

const MAX_LINES: usize = 2000;
const HISTORY_LIMIT: usize = 100;
const LOG_ID: &str = "console-log";
const INPUT_ID: &str = "console-input";
const SEARCH_ID: &str = "console-search";
const MATCH_BG: Color = Color::from_rgba(0.91, 0.71, 0.38, 0.35);
const RENDER_LIMIT: usize = 500;
const BOTTOM_SLACK: f32 = 4.0;
const COMMANDS: &[&str] = &[
    "advancement",
    "attribute",
    "ban",
    "ban-ip",
    "banlist",
    "bossbar",
    "clear",
    "clone",
    "damage",
    "data",
    "datapack",
    "debug",
    "defaultgamemode",
    "deop",
    "difficulty",
    "effect",
    "enchant",
    "execute",
    "experience",
    "fill",
    "fillbiome",
    "forceload",
    "function",
    "gamemode",
    "gamerule",
    "give",
    "help",
    "item",
    "jfr",
    "kick",
    "kill",
    "list",
    "locate",
    "loot",
    "me",
    "msg",
    "op",
    "pardon",
    "pardon-ip",
    "particle",
    "perf",
    "place",
    "playsound",
    "publish",
    "random",
    "recipe",
    "reload",
    "return",
    "ride",
    "rotate",
    "save-all",
    "save-off",
    "save-on",
    "say",
    "schedule",
    "scoreboard",
    "seed",
    "setblock",
    "setidletimeout",
    "setworldspawn",
    "spawnpoint",
    "spectate",
    "spreadplayers",
    "stop",
    "stopsound",
    "summon",
    "tag",
    "team",
    "teammsg",
    "teleport",
    "tell",
    "tellraw",
    "tick",
    "time",
    "title",
    "tm",
    "tp",
    "transfer",
    "trigger",
    "w",
    "weather",
    "whitelist",
    "worldborder",
    "xp",
];

#[derive(Debug)]
pub struct State {
    server_id: Option<String>,
    lines: Vec<ConsoleLine>,
    loading: bool,
    input: String,
    history: Vec<String>,
    history_pos: Option<usize>,
    draft: String,
    at_bottom: bool,
    sending: bool,
    search: Option<String>,
    view: LogView,
    completion: Option<Completion>,
}

#[derive(Debug)]
struct Completion {
    base: String,
    options: Vec<String>,
    index: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            server_id: None,
            lines: Vec::new(),
            loading: false,
            input: String::new(),
            history: Vec::new(),
            history_pos: None,
            draft: String::new(),
            at_bottom: true,
            sending: false,
            search: None,
            view: LogView::All,
            completion: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(String, Vec<ConsoleLine>),
    Input(String),
    Submit,
    Sent(Result<(), String>),
    Scrolled(f32),
    JumpToLatest,
    ArrowKey(bool),
    ArrowResolved(bool, bool),
    TabKey,
    TabResolved(bool),
    Find,
    Search(String),
    CloseSearch,
    View(LogView),
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Loaded(id, lines) => {
            if state.server_id.as_deref() == Some(id.as_str()) {
                let pending = std::mem::take(&mut state.lines);
                state.lines = lines;
                let seen = state.lines.len().saturating_sub(pending.len() + 50);
                for line in pending {
                    let duplicate = state.lines[seen..]
                        .iter()
                        .any(|old| old.timestamp == line.timestamp && old.text == line.text);
                    if !duplicate {
                        state.lines.push(line);
                    }
                }
                trim(&mut state.lines);
                state.loading = false;
            }
            Task::none()
        }
        Message::Input(value) => {
            state.input = value;
            state.history_pos = None;
            state.completion = None;
            Task::none()
        }
        Message::Submit => submit(state, ctx),
        Message::Sent(result) => {
            state.sending = false;
            if let Err(reason) = result {
                ctx.error(format!("The command was not sent. {reason}"));
            }
            operation::focus(INPUT_ID)
        }
        Message::Scrolled(distance_from_bottom) => {
            state.at_bottom = distance_from_bottom <= BOTTOM_SLACK;
            Task::none()
        }
        Message::JumpToLatest => {
            state.at_bottom = true;
            jump_to_latest()
        }
        Message::ArrowKey(up) => {
            if ctx.route != Route::Server(Tab::Overview) || state.history.is_empty() {
                return Task::none();
            }
            operation::is_focused(INPUT_ID).map(move |focused| Message::ArrowResolved(up, focused))
        }
        Message::ArrowResolved(up, focused) => {
            if !focused {
                return Task::none();
            }
            step_history(state, up);
            operation::move_cursor_to_end(INPUT_ID)
        }
        Message::TabKey => {
            if ctx.route != Route::Server(Tab::Overview) || state.server_id.is_none() {
                return Task::none();
            }
            operation::is_focused(INPUT_ID).map(Message::TabResolved)
        }
        Message::TabResolved(focused) => {
            if !focused {
                return Task::none();
            }
            let players = state
                .server_id
                .as_deref()
                .and_then(|id| ctx.stats(id))
                .map(|stats| stats.online_players.as_slice())
                .unwrap_or_default();
            complete(state, players);
            operation::move_cursor_to_end(INPUT_ID)
        }
        Message::Find => {
            if ctx.route != Route::Server(Tab::Overview) || state.server_id.is_none() {
                return Task::none();
            }
            state.search.get_or_insert_with(String::new);
            Task::batch([
                operation::focus(SEARCH_ID),
                operation::select_all(SEARCH_ID),
            ])
        }
        Message::Search(query) => {
            state.search = Some(query);
            Task::none()
        }
        Message::CloseSearch => {
            if state.search.take().is_none() {
                return Task::none();
            }
            state.at_bottom = true;
            Task::batch([operation::focus(INPUT_ID), jump_to_latest()])
        }
        Message::View(view) => {
            state.view = view;
            state.at_bottom = true;
            jump_to_latest()
        }
    }
}

fn submit(state: &mut State, ctx: &Ctx) -> Task<Message> {
    let command = state.input.trim().to_string();
    let Some(id) = state.server_id.clone() else {
        return Task::none();
    };
    if command.is_empty() || state.sending || !accepts_commands(ctx, &id) {
        return Task::none();
    }
    if state.history.last() != Some(&command) {
        state.history.push(command.clone());
        if state.history.len() > HISTORY_LIMIT {
            state.history.remove(0);
        }
    }
    state.history_pos = None;
    state.completion = None;
    state.input.clear();
    state.sending = true;
    state.at_bottom = true;
    Task::batch([
        Task::perform(server_runner::send_command(id, command), |result| {
            Message::Sent(match result.error {
                Some(reason) if !result.success => Err(reason),
                _ if !result.success => Err("The server did not accept it.".into()),
                _ => Ok(()),
            })
        }),
        jump_to_latest(),
    ])
}

fn complete(state: &mut State, players: &[String]) {
    if let Some(completion) = &mut state.completion {
        completion.index = (completion.index + 1) % completion.options.len();
        state.input = format!(
            "{}{}",
            completion.base, completion.options[completion.index]
        );
        return;
    }
    let (base, mut options) = candidates(&state.input, players);
    match options.len() {
        0 => {}
        1 => state.input = format!("{base}{} ", options.remove(0)),
        _ => {
            state.input = format!("{base}{}", options[0]);
            state.completion = Some(Completion {
                base,
                options,
                index: 0,
            });
        }
    }
}

fn candidates(input: &str, players: &[String]) -> (String, Vec<String>) {
    let split = input.rfind(' ').map_or(0, |i| i + 1);
    let (base, token) = input.split_at(split);
    let token = token.to_lowercase();
    let options = if split == 0 {
        let slash = if token.starts_with('/') { "/" } else { "" };
        let token = token.trim_start_matches('/');
        COMMANDS
            .iter()
            .filter(|command| command.starts_with(token))
            .map(|command| format!("{slash}{command}"))
            .collect()
    } else {
        let mut names: Vec<String> = players
            .iter()
            .filter(|name| name.to_lowercase().starts_with(&token))
            .cloned()
            .collect();
        names.sort_by_key(|name| name.to_lowercase());
        names
    };
    (base.to_string(), options)
}

fn step_history(state: &mut State, up: bool) {
    state.completion = None;
    let len = state.history.len();
    let next = match (state.history_pos, up) {
        (None, true) => {
            state.draft = std::mem::take(&mut state.input);
            Some(len - 1)
        }
        (None, false) => return,
        (Some(0), true) => Some(0),
        (Some(pos), true) => Some(pos - 1),
        (Some(pos), false) if pos + 1 < len => Some(pos + 1),
        (Some(_), false) => None,
    };
    state.history_pos = next;
    state.input = match next {
        Some(pos) => state.history[pos].clone(),
        None => std::mem::take(&mut state.draft),
    };
}

fn jump_to_latest() -> Task<Message> {
    operation::scroll_to(
        LOG_ID,
        AbsoluteOffset {
            x: None,
            y: Some(0.0),
        },
    )
}

fn trim(lines: &mut Vec<ConsoleLine>) {
    if lines.len() > MAX_LINES {
        lines.drain(..lines.len() - MAX_LINES);
    }
}

fn accepts_commands(ctx: &Ctx, id: &str) -> bool {
    matches!(
        ctx.status(id),
        ServerRuntimeStatus::Online | ServerRuntimeStatus::Starting
    )
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let Some(id) = state.server_id.as_deref().or(ctx.selected.as_deref()) else {
        return container(text("Select a server to see its console.").style(style::muted))
            .center(Length::Fill)
            .into();
    };
    let mut layout = column![toolbar(state)]
        .spacing(SPACE_3)
        .height(Length::Fill);
    if let Some(query) = &state.search {
        layout = layout.push(search_bar(state, query));
    }
    layout
        .push(log_area(state))
        .push(command_bar(state, ctx, id))
        .into()
}

const VIEWS: [(LogView, &str); 4] = [
    (LogView::All, "All"),
    (LogView::Warnings, "Warnings"),
    (LogView::Errors, "Errors"),
    (LogView::Chat, "Chat"),
];

fn toolbar(state: &State) -> Element<'_, Message> {
    let views = VIEWS
        .iter()
        .fold(row![].spacing(SPACE_1), |bar, &(view, label)| {
            bar.push(
                button(text(label).size(TEXT_SM).font(UI_MEDIUM))
                    .padding([4, 10])
                    .style(style::tab(state.view == view))
                    .on_press(Message::View(view)),
            )
        });
    row![
        views,
        Space::new().width(Length::Fill),
        tool(Icon::Search, "Find (Ctrl+F)", Some(Message::Find)),
    ]
    .spacing(SPACE_1)
    .align_y(Alignment::Center)
    .into()
}

fn tool<'a>(icon: Icon, label: &'a str, on_press: Option<Message>) -> Element<'a, Message> {
    tooltip(
        sized_icon(icon_colored(icon, 16.0, INK_SECONDARY), BUTTON_SM)
            .style(style::ghost)
            .on_press_maybe(on_press),
        container(text(label).size(TEXT_XS).color(INK))
            .padding([4, 8])
            .style(style::tooltip),
        tooltip::Position::Bottom,
    )
    .into()
}

fn query(state: &State) -> &str {
    state.search.as_deref().map(str::trim).unwrap_or("")
}

fn search_bar<'a>(state: &'a State, query_text: &'a str) -> Element<'a, Message> {
    let q = query(state);
    let count = if q.is_empty() {
        String::new()
    } else {
        match filter_logs(&state.lines, q, state.view).len() {
            1 => "1 line".to_string(),
            n => format!("{n} lines"),
        }
    };
    let field = text_input("Find in console", query_text)
        .id(SEARCH_ID)
        .on_input(Message::Search)
        .on_submit(Message::CloseSearch)
        .size(TEXT_MD)
        .padding([SPACE_2, SPACE_3])
        .style(style::input)
        .width(Length::Fill);
    let close = sized(icon_colored(Icon::Close, 14.0, INK_SECONDARY), BUTTON_SM)
        .style(style::ghost)
        .on_press(Message::CloseSearch);
    row![
        icon_colored(Icon::Search, 16.0, INK_SECONDARY),
        field,
        text(count).size(TEXT_SM).style(style::muted).font(MONO),
        close
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center)
    .into()
}

fn log_area(state: &State) -> Element<'_, Message> {
    let q = query(state);
    let matches;
    let lines = if q.is_empty() && state.view == LogView::All {
        &state.lines
    } else {
        matches = filter_logs(&state.lines, q, state.view);
        &matches
    };
    let body: Element<'_, Message> = if lines.is_empty() {
        let note = if state.loading {
            "Loading console output"
        } else if !q.is_empty() {
            "No lines match your search."
        } else if state.view == LogView::Chat {
            "No chat messages yet."
        } else if state.view != LogView::All {
            "No lines at this level."
        } else {
            "No output yet. Start the server and its log will appear here."
        };
        container(text(note).size(TEXT_MD).style(style::muted))
            .center(Length::Fill)
            .into()
    } else {
        let skip = lines.len().saturating_sub(RENDER_LIMIT);
        let list = lines
            .iter()
            .skip(skip)
            .fold(column![].spacing(2), |list, line| {
                list.push(line_view(line, q))
            });
        scrollable(
            container(list)
                .padding([SPACE_3, SPACE_4])
                .width(Length::Fill),
        )
        .id(LOG_ID)
        .anchor_bottom()
        .on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y))
        .style(style::scroll)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    };
    let surface = container(body)
        .style(style::console)
        .width(Length::Fill)
        .height(Length::Fill);
    if state.at_bottom {
        return stack![surface].into();
    }
    let pill = container(
        sized(
            row![
                icon_colored(Icon::ChevronDown, 14.0, INK),
                text("Jump to latest").size(TEXT_SM).font(UI_MEDIUM)
            ]
            .spacing(SPACE_1)
            .align_y(Alignment::Center),
            BUTTON_SM,
        )
        .style(style::secondary)
        .on_press(Message::JumpToLatest),
    )
    .padding(SPACE_4)
    .center_x(Length::Fill)
    .align_bottom(Length::Fill);
    stack![surface, pill].into()
}

fn line_view<'a>(line: &ConsoleLine, query: &str) -> Element<'a, Message> {
    let base = match line.kind {
        ConsoleLineType::Stdout => level_color(&line.text),
        ConsoleLineType::Stderr => CORAL,
        ConsoleLineType::System => MOSS,
    };
    let text = legacy_to_ansi(&line.text);
    rich_text::<(), Message, _, _>(highlight(ansi_spans(&text, base), query))
        .size(TEXT_SM)
        .font(MONO)
        .width(Length::Fill)
        .into()
}

fn command_bar<'a>(state: &'a State, ctx: &'a Ctx, id: &str) -> Element<'a, Message> {
    let enabled = accepts_commands(ctx, id);
    let reason = match ctx.status(id) {
        ServerRuntimeStatus::Stopping => "The server is stopping. Start it again to send commands.",
        ServerRuntimeStatus::Crashed => "The server crashed. Start it again to send commands.",
        _ => "The server is offline. Start it to send commands.",
    };
    let placeholder = if enabled {
        "Type a command and press Enter"
    } else {
        reason
    };
    let mut field = text_input(placeholder, &state.input)
        .id(INPUT_ID)
        .size(TEXT_MD)
        .font(MONO)
        .padding([SPACE_2, SPACE_3])
        .style(style::input)
        .width(Length::Fill);
    if enabled {
        field = field.on_input(Message::Input).on_submit(Message::Submit);
    }
    let can_send = enabled && !state.sending && !state.input.trim().is_empty();
    let send = sized(
        text(if state.sending { "Sending" } else { "Send" }).font(UI_MEDIUM),
        BUTTON_MD,
    )
    .style(style::primary)
    .on_press_maybe(can_send.then_some(Message::Submit));
    row![field, send]
        .spacing(SPACE_2)
        .align_y(Alignment::Center)
        .into()
}

pub fn on_event(state: &mut State, event: &Event, _ctx: &Ctx) -> Task<Message> {
    if let Event::ConsoleOutput { server_id, line } = event {
        if state.server_id.as_deref() == Some(server_id.as_str()) {
            state.lines.push(line.clone());
            trim(&mut state.lines);
        }
    }
    Task::none()
}

pub fn on_show(state: &mut State, ctx: &Ctx) -> Task<Message> {
    let Some(id) = ctx.selected.clone() else {
        return Task::none();
    };
    if state.server_id.as_deref() == Some(id.as_str()) {
        return Task::none();
    }
    *state = State {
        server_id: Some(id.clone()),
        loading: true,
        ..State::default()
    };
    Task::perform(server_runner::get_server_logs(id.clone()), move |lines| {
        Message::Loaded(id.clone(), lines)
    })
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    event::listen_with(|event, status, _| match event {
        iced::Event::Keyboard(keyboard::Event::KeyPressed {
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::KeyF),
            modifiers,
            ..
        }) if modifiers.command() && !modifiers.alt() => Some(Message::Find),
        iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Escape),
            ..
        }) => Some(Message::CloseSearch),
        iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(named),
            ..
        }) if status == event::Status::Ignored => match named {
            keyboard::key::Named::ArrowUp => Some(Message::ArrowKey(true)),
            keyboard::key::Named::ArrowDown => Some(Message::ArrowKey(false)),
            keyboard::key::Named::Tab => Some(Message::TabKey),
            _ => None,
        },
        _ => None,
    })
}

const LEGACY: [char; 16] = [
    '0', '4', '2', '6', '1', '5', '3', '7', '8', 'c', 'a', 'e', '9', 'd', 'b', 'f',
];

pub fn legacy_spans(text: &str, base: Color) -> Vec<Span<'static, (), iced::Font>> {
    ansi_spans(&legacy_to_ansi(text), base)
}

fn level_color(text: &str) -> Color {
    match line_level(text) {
        LogLevel::Error => CORAL,
        LogLevel::Warn => AMBER,
        LogLevel::Info => LOG_INFO,
    }
}

fn legacy_to_ansi(input: &str) -> std::borrow::Cow<'_, str> {
    if !input.contains('§') {
        return input.into();
    }
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '§' {
            out.push(c);
            continue;
        }
        let Some(code) = chars.next() else { break };
        let sgr = match code.to_ascii_lowercase() {
            'x' => {
                let hex: String = (0..6)
                    .filter_map(|_| {
                        chars.next_if_eq(&'§')?;
                        chars.next()
                    })
                    .collect();
                match u32::from_str_radix(&hex, 16) {
                    Ok(rgb) if hex.len() == 6 => {
                        format!("38;2;{};{};{}", rgb >> 16, rgb >> 8 & 0xff, rgb & 0xff)
                    }
                    _ => continue,
                }
            }
            'r' => "0".into(),
            c => match LEGACY.iter().position(|&l| l == c) {
                Some(i) if i < 8 => (30 + i).to_string(),
                Some(i) => (82 + i).to_string(),
                None => continue,
            },
        };
        out.push_str("[");
        out.push_str(&sgr);
        out.push('m');
    }
    out.into()
}

fn highlight(
    spans: Vec<Span<'static, (), iced::Font>>,
    query: &str,
) -> Vec<Span<'static, (), iced::Font>> {
    if query.is_empty() {
        return spans;
    }
    let needle = query.to_ascii_lowercase();
    let mut out = Vec::with_capacity(spans.len());
    for piece in spans {
        let text = piece.text.to_string();
        let lower = text.to_ascii_lowercase();
        let mut last = 0;
        for (start, _) in lower.match_indices(&needle) {
            if start < last {
                continue;
            }
            let end = start + needle.len();
            if start > last {
                out.push(Span {
                    text: text[last..start].to_owned().into(),
                    ..piece.clone()
                });
            }
            out.push(
                Span {
                    text: text[start..end].to_owned().into(),
                    ..piece.clone()
                }
                .background(MATCH_BG),
            );
            last = end;
        }
        if last == 0 {
            out.push(piece);
        } else if last < text.len() {
            out.push(Span {
                text: text[last..].to_owned().into(),
                ..piece
            });
        }
    }
    out
}

fn ansi_spans(input: &str, base: Color) -> Vec<Span<'static, (), iced::Font>> {
    let mut color = base;
    let mut spans = Vec::new();
    for (segment, codes) in segments(input) {
        if let Some(codes) = codes {
            color = apply_sgr(codes, color, base);
        }
        if !segment.is_empty() {
            spans.push(span(segment.to_owned()).color(color));
        }
    }
    spans
}

fn segments(input: &str) -> impl Iterator<Item = (&str, Option<&str>)> {
    let mut rest = input;
    let mut pending: Option<&str> = None;
    let mut done = false;
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        let codes = pending.take();
        match rest.find("\x1b[") {
            Some(start) => {
                let text = &rest[..start];
                let after = &rest[start + 2..];
                match after.find(|c: char| c.is_ascii_alphabetic()) {
                    Some(end) => {
                        pending = (after.as_bytes()[end] == b'm').then_some(&after[..end]);
                        rest = &after[end + 1..];
                    }
                    None => {
                        rest = "";
                        done = true;
                    }
                }
                Some((text, codes))
            }
            None => {
                done = true;
                Some((rest, codes))
            }
        }
    })
}

fn apply_sgr(codes: &str, current: Color, base: Color) -> Color {
    let mut color = current;
    let mut args = codes.split(';').map(|code| {
        if code.is_empty() {
            Some(0)
        } else {
            code.parse::<u8>().ok()
        }
    });
    while let Some(code) = args.next() {
        let Some(value) = code else { continue };
        match value {
            0 | 39 => color = base,
            30..=37 => color = palette(value - 30, false),
            90..=97 => color = palette(value - 90, true),
            38 | 48 => {
                let parsed = match args.next() {
                    Some(Some(5)) => args.next().flatten().map(indexed),
                    Some(Some(2)) => {
                        let mut channel = || args.next().flatten();
                        match (channel(), channel(), channel()) {
                            (Some(r), Some(g), Some(b)) => Some(Color::from_rgb8(r, g, b)),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let (38, Some(parsed)) = (value, parsed) {
                    color = parsed;
                }
            }
            _ => {}
        }
    }
    color
}

fn indexed(n: u8) -> Color {
    match n {
        0..=7 => palette(n, false),
        8..=15 => palette(n - 8, true),
        16..=231 => {
            let n = n - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color::from_rgb8(level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        _ => {
            let gray = 8 + (n - 232) * 10;
            Color::from_rgb8(gray, gray, gray)
        }
    }
}

fn palette(index: u8, bright: bool) -> Color {
    let (r, g, b) = match (index, bright) {
        (0, false) => (0x4a, 0x50, 0x57),
        (0, true) => (0x6b, 0x71, 0x78),
        (1, false) => (0xd9, 0x5f, 0x55),
        (1, true) => (0xef, 0x6a, 0x5e),
        (2, false) => (0x4b, 0xb8, 0x75),
        (2, true) => (0x5f, 0xd3, 0x8d),
        (3, false) => (0xd0, 0xa0, 0x4f),
        (3, true) => (0xe8, 0xb5, 0x62),
        (4, false) => (0x5a, 0x8f, 0xd6),
        (4, true) => (0x7a, 0xa8, 0xe8),
        (5, false) => (0xb0, 0x7a, 0xd0),
        (5, true) => (0xc7, 0x95, 0xe4),
        (6, false) => (0x4f, 0xb5, 0xc0),
        (6, true) => (0x6b, 0xcf, 0xd9),
        (_, false) => (0xc4, 0xc9, 0xcf),
        (_, true) => (0xec, 0xee, 0xf0),
    };
    Color::from_rgb8(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completes_commands_and_players() {
        let players = vec!["Steve".to_string(), "stan".to_string(), "Alex".to_string()];
        let (base, options) = candidates("gam", &players);
        assert_eq!(base, "");
        assert_eq!(options, ["gamemode", "gamerule"]);
        assert_eq!(candidates("/wh", &players).1, ["/whitelist"]);
        let (base, options) = candidates("tp st", &players);
        assert_eq!(base, "tp ");
        assert_eq!(options, ["stan", "Steve"]);

        let mut state = State {
            input: "op S".into(),
            ..State::default()
        };
        complete(&mut state, &players);
        assert_eq!(state.input, "op stan");
        complete(&mut state, &players);
        assert_eq!(state.input, "op Steve");
        complete(&mut state, &players);
        assert_eq!(state.input, "op stan");
        state.completion = None;
        state.input = "kick al".into();
        complete(&mut state, &players);
        assert_eq!(state.input, "kick Alex ");
    }

    #[test]
    fn extended_colours_do_not_leak_arguments() {
        let base = Color::WHITE;
        assert_eq!(apply_sgr("38;5;1", base, base), indexed(1));
        assert_eq!(
            apply_sgr("38;2;10;20;30", base, base),
            Color::from_rgb8(10, 20, 30)
        );
        assert_eq!(apply_sgr("1", base, base), base);
    }

    #[test]
    fn legacy_codes_become_ansi() {
        assert_eq!(legacy_to_ansi("§cHi§r"), "[91mHi[0m");
        assert_eq!(legacy_to_ansi("§4a§lb"), "[31mab");
        assert_eq!(legacy_to_ansi("§x§f§f§0§0§8§0X"), "[38;2;255;0;128mX");
        assert_eq!(legacy_to_ansi("plain"), "plain");
    }

    #[test]
    fn levels_pick_colours() {
        assert_eq!(level_color("[12:00:00 WARN]: Can't keep up!"), AMBER);
        assert_eq!(level_color("[12:00:00] [Server thread/ERROR]: boom"), CORAL);
        assert_eq!(level_color("[12:00:00 INFO]: Done"), LOG_INFO);
    }

    #[test]
    fn highlight_splits_matches() {
        let spans = highlight(ansi_spans("Steve joined steve", LOG_INFO), "steve");
        let texts: Vec<_> = spans.iter().map(|s| s.text.to_string()).collect();
        assert_eq!(texts, ["Steve", " joined ", "steve"]);
        assert!(spans[0].highlight.is_some() && spans[1].highlight.is_none());
    }
}
