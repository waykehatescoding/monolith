use iced::widget::operation::{self, AbsoluteOffset};
use monolith_theme::token::{BUTTON_MD, BUTTON_SM};
use monolith_theme::widget::sized;
use iced::widget::text::Span;
use iced::widget::{
    column, container, row, rich_text, scrollable, space, span, stack, text, text_input,
};
use iced::{clipboard, event, keyboard, Alignment, Color, Element, Length, Subscription, Task};
use monolith_core::events::Event;
use monolith_core::log_filter::filter_logs;
use monolith_core::server_runner;
use monolith_core::types::{ConsoleLine, ConsoleLineType, ServerRuntimeStatus};
use monolith_theme::style;
use monolith_theme::token::{
    CORAL, INK, INK_TERTIARY, LOG_INFO, MOSS, SPACE_1, SPACE_2, SPACE_3, SPACE_4, TEXT_MD, TEXT_SM,
};
use monolith_theme::{icon_colored, Icon, MONO, UI_MEDIUM};

use crate::ctx::{Ctx, Route, Tab};

const MAX_LINES: usize = 2000;
const HISTORY_LIMIT: usize = 100;
const LOG_ID: &str = "console-log";
const INPUT_ID: &str = "console-input";
const RENDER_LIMIT: usize = 500;
const BOTTOM_SLACK: f32 = 4.0;

#[derive(Debug)]
pub struct State {
    server_id: Option<String>,
    lines: Vec<ConsoleLine>,
    loading: bool,
    search: String,
    kinds: Vec<ConsoleLineType>,
    input: String,
    history: Vec<String>,
    history_pos: Option<usize>,
    draft: String,
    at_bottom: bool,
    sending: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            server_id: None,
            lines: Vec::new(),
            loading: false,
            search: String::new(),
            kinds: Vec::new(),
            input: String::new(),
            history: Vec::new(),
            history_pos: None,
            draft: String::new(),
            at_bottom: true,
            sending: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(String, Vec<ConsoleLine>),
    Search(String),
    ToggleKind(ConsoleLineType),
    Input(String),
    Submit,
    Sent(Result<(), String>),
    Scrolled(f32),
    JumpToLatest,
    Copy,
    Clear,
    ArrowKey(bool),
    ArrowResolved(bool, bool),
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
        Message::Search(value) => {
            state.search = value;
            Task::none()
        }
        Message::ToggleKind(kind) => {
            match state.kinds.iter().position(|k| *k == kind) {
                Some(index) => {
                    state.kinds.remove(index);
                }
                None => state.kinds.push(kind),
            }
            Task::none()
        }
        Message::Input(value) => {
            state.input = value;
            state.history_pos = None;
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
        Message::Copy => {
            let text = visible(state)
                .iter()
                .map(|line| strip_ansi(&line.text))
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                ctx.info("There is nothing to copy yet.");
                return Task::none();
            }
            ctx.success("Console output copied.");
            clipboard::write(text)
        }
        Message::Clear => {
            state.lines.clear();
            state.at_bottom = true;
            ctx.info("Console cleared. New output will keep appearing.");
            Task::none()
        }
        Message::ArrowKey(up) => {
            if ctx.route != Route::Server(Tab::Console) || state.history.is_empty() {
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

fn step_history(state: &mut State, up: bool) {
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

fn visible(state: &State) -> Vec<ConsoleLine> {
    filter_logs(&state.lines, &state.search, &state.kinds)
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let Some(id) = state.server_id.as_deref().or(ctx.selected.as_deref()) else {
        return container(text("Select a server to see its console.").style(style::muted))
            .center(Length::Fill)
            .into();
    };
    column![toolbar(state), log_area(state), command_bar(state, ctx, id)]
        .spacing(SPACE_3)
        .height(Length::Fill)
        .into()
}

fn toolbar(state: &State) -> Element<'_, Message> {
    let search = text_input("Search output", &state.search)
        .on_input(Message::Search)
        .size(TEXT_SM)
        .padding([5.0, SPACE_3])
        .style(style::input)
        .width(Length::FillPortion(2));
    let chips = [
        (ConsoleLineType::Stdout, "Output"),
        (ConsoleLineType::Stderr, "Errors"),
        (ConsoleLineType::System, "System"),
    ]
    .into_iter()
    .fold(row![].spacing(SPACE_1), |chips, (kind, label)| {
        chips.push(
            sized(text(label).size(TEXT_SM).font(UI_MEDIUM), BUTTON_SM)
                .style(style::tab(state.kinds.contains(&kind)))
                .on_press(Message::ToggleKind(kind)),
        )
    });
    let empty = state.lines.is_empty();
    let filtering = !state.search.is_empty() || !state.kinds.is_empty();
    let count = if filtering {
        text(format!("{} of {} lines", visible(state).len(), state.lines.len()))
            .size(TEXT_SM)
            .style(style::muted)
    } else {
        text("")
    };
    row![
        search,
        chips,
        count,
        space().width(Length::FillPortion(3)),
        action_button("Copy", Icon::Copy, (!empty).then_some(Message::Copy)),
        action_button("Clear", Icon::Trash, (!empty).then_some(Message::Clear)),
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center)
    .into()
}

fn action_button(
    label: &'static str,
    glyph: Icon,
    on_press: Option<Message>,
) -> Element<'static, Message> {
    let color = if on_press.is_some() { INK } else { INK_TERTIARY };
    let content = row![
        icon_colored(glyph, 14.0, color),
        text(label).size(TEXT_SM).font(UI_MEDIUM)
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center);
    let base = sized(content, BUTTON_SM).style(style::ghost);
    match on_press {
        Some(message) => base.on_press(message).into(),
        None => base.into(),
    }
}

fn log_area(state: &State) -> Element<'_, Message> {
    let lines = visible(state);
    let body: Element<'_, Message> = if lines.is_empty() {
        let note = if state.loading {
            "Loading console output"
        } else if state.lines.is_empty() {
            "No output yet. Start the server and its log will appear here."
        } else {
            "Nothing matches. Clear the search or turn off a filter."
        };
        container(text(note).size(TEXT_MD).style(style::muted))
            .center(Length::Fill)
            .into()
    } else {
        let skip = lines.len().saturating_sub(RENDER_LIMIT);
        let list = lines
            .into_iter()
            .skip(skip)
            .fold(column![].spacing(2), |list, line| list.push(line_view(line)));
        scrollable(container(list).padding([SPACE_3, SPACE_4]).width(Length::Fill))
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
        return surface.into();
    }
    let pill = container(
        sized(
            row![
                icon_colored(Icon::ChevronDown, 14.0, INK),
                text("Jump to latest").size(TEXT_SM).font(UI_MEDIUM)
            ]
            .spacing(SPACE_1)
            .align_y(Alignment::Center), BUTTON_SM)
        .style(style::secondary)
        .on_press(Message::JumpToLatest),
    )
    .padding(SPACE_4)
    .center_x(Length::Fill)
    .align_bottom(Length::Fill);
    stack![surface, pill].into()
}

fn line_view<'a>(line: ConsoleLine) -> Element<'a, Message> {
    let base = match line.kind {
        ConsoleLineType::Stdout => LOG_INFO,
        ConsoleLineType::Stderr => CORAL,
        ConsoleLineType::System => MOSS,
    };
    rich_text::<(), Message, _, _>(ansi_spans(&line.text, base))
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
    let send = sized(text(if state.sending { "Sending" } else { "Send" }).font(UI_MEDIUM), BUTTON_MD)
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
            key: keyboard::Key::Named(named),
            ..
        }) if status == event::Status::Ignored => match named {
            keyboard::key::Named::ArrowUp => Some(Message::ArrowKey(true)),
            keyboard::key::Named::ArrowDown => Some(Message::ArrowKey(false)),
            _ => None,
        },
        _ => None,
    })
}

fn strip_ansi(input: &str) -> String {
    segments(input).map(|(segment, _)| segment).collect()
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
    let mut args = codes
        .split(';')
        .map(|code| if code.is_empty() { Some(0) } else { code.parse::<u8>().ok() });
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
    fn strips_ansi_sequences() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m plain"), "red plain");
        assert_eq!(strip_ansi("no codes"), "no codes");
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
}
