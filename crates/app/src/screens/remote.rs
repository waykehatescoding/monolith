use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::operation;
use iced::widget::{
    button, column, container, rich_text, row, rule, scrollable, text, text_input, Column, Space,
};
use iced::{event, keyboard, Alignment, Element, Length, Subscription, Task};
use monolith_core::events::Event;
use monolith_core::rcon::{self, Rcon, RconConnection, DEFAULT_PORT};
use monolith_theme::token::*;
use monolith_theme::widget::{sized, sized_icon};
use monolith_theme::{icon, style, Icon, MONO, UI_MEDIUM, UI_SEMIBOLD};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::ctx::{Ctx, Route};
use crate::screens::server::console::legacy_spans;
use crate::ui::busy_button;

const INPUT_ID: &str = "remote-input";
const HISTORY_LIMIT: usize = 100;
const MAX_LINES: usize = 1000;

type Session = Arc<Mutex<Rcon<TcpStream>>>;

#[derive(Debug, Clone)]
pub struct Target {
    label: String,
    host: String,
    port: u16,
    password: String,
}

#[derive(Debug, Default)]
struct Form {
    id: Option<String>,
    name: String,
    host: String,
    port: String,
    password: String,
}

#[derive(Debug)]
struct Line {
    text: String,
    command: bool,
}

#[derive(Debug, Default)]
pub struct State {
    connections: Vec<RconConnection>,
    local: Vec<Target>,
    form: Option<Form>,
    saving: bool,
    target: Option<Target>,
    session: Option<Session>,
    connecting: bool,
    error: Option<String>,
    lines: Vec<Line>,
    input: String,
    history: Vec<String>,
    history_pos: Option<usize>,
    draft: String,
    sending: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(Vec<RconConnection>),
    LocalLoaded(Vec<Target>),
    New,
    Edit(String),
    FormName(String),
    FormHost(String),
    FormPort(String),
    FormPassword(String),
    CancelForm,
    SaveForm,
    Delete(String),
    Saved(Result<Vec<RconConnection>, String>),
    Connect(Target),
    Connected(Result<Session, String>),
    Disconnect,
    Input(String),
    Submit,
    Output(Result<String, String>),
    ArrowKey(bool),
    ArrowResolved(bool, bool),
}

impl Form {
    fn port(&self) -> Option<u16> {
        self.port.trim().parse().ok().filter(|port| *port > 0)
    }

    fn ready(&self) -> bool {
        !self.name.trim().is_empty() && !self.host.trim().is_empty() && self.port().is_some()
    }
}

fn target(connection: &RconConnection) -> Target {
    Target {
        label: connection.name.clone(),
        host: connection.host.clone(),
        port: connection.port,
        password: connection.password.clone(),
    }
}

fn save(connections: Vec<RconConnection>) -> Task<Message> {
    Task::perform(
        async move {
            rcon::save_connections(connections.clone()).await?;
            Ok(connections)
        },
        Message::Saved,
    )
}

fn push(state: &mut State, text: String, command: bool) {
    state.lines.push(Line { text, command });
    if state.lines.len() > MAX_LINES {
        state.lines.drain(..state.lines.len() - MAX_LINES);
    }
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Loaded(connections) => {
            state.connections = connections;
            Task::none()
        }
        Message::LocalLoaded(local) => {
            state.local = local;
            Task::none()
        }
        Message::New => {
            state.form = Some(Form {
                port: DEFAULT_PORT.to_string(),
                ..Form::default()
            });
            Task::none()
        }
        Message::Edit(id) => {
            state.form = state.connections.iter().find(|c| c.id == id).map(|c| Form {
                id: Some(c.id.clone()),
                name: c.name.clone(),
                host: c.host.clone(),
                port: c.port.to_string(),
                password: c.password.clone(),
            });
            Task::none()
        }
        Message::FormName(value) => {
            if let Some(form) = &mut state.form {
                form.name = value;
            }
            Task::none()
        }
        Message::FormHost(value) => {
            if let Some(form) = &mut state.form {
                form.host = value;
            }
            Task::none()
        }
        Message::FormPort(value) => {
            if let Some(form) = &mut state.form {
                form.port = value;
            }
            Task::none()
        }
        Message::FormPassword(value) => {
            if let Some(form) = &mut state.form {
                form.password = value;
            }
            Task::none()
        }
        Message::CancelForm => {
            state.form = None;
            Task::none()
        }
        Message::SaveForm => {
            let Some(form) = state.form.as_ref().filter(|f| f.ready() && !state.saving) else {
                return Task::none();
            };
            let connection = RconConnection {
                id: form
                    .id
                    .clone()
                    .unwrap_or_else(|| chrono::Utc::now().timestamp_millis().to_string()),
                name: form.name.trim().to_string(),
                host: form.host.trim().to_string(),
                port: form.port().unwrap_or(DEFAULT_PORT),
                password: form.password.clone(),
            };
            let mut connections = state.connections.clone();
            match connections.iter_mut().find(|c| c.id == connection.id) {
                Some(existing) => *existing = connection,
                None => connections.push(connection),
            }
            state.saving = true;
            save(connections)
        }
        Message::Delete(id) => {
            let connections = state
                .connections
                .iter()
                .filter(|c| c.id != id)
                .cloned()
                .collect();
            state.saving = true;
            save(connections)
        }
        Message::Saved(result) => {
            state.saving = false;
            match result {
                Ok(connections) => {
                    state.connections = connections;
                    state.form = None;
                }
                Err(error) => ctx.error(format!("Could not save the connection: {error}")),
            }
            Task::none()
        }
        Message::Connect(target) => {
            if state.connecting {
                return Task::none();
            }
            state.form = None;
            state.session = None;
            state.connecting = true;
            state.error = None;
            state.lines.clear();
            let (host, port, password) =
                (target.host.clone(), target.port, target.password.clone());
            state.target = Some(target);
            Task::perform(
                async move {
                    rcon::connect(&host, port, &password)
                        .await
                        .map(|rcon| Arc::new(Mutex::new(rcon)))
                },
                Message::Connected,
            )
        }
        Message::Connected(result) => {
            state.connecting = false;
            match result {
                Ok(session) => {
                    state.session = Some(session);
                    operation::focus(INPUT_ID)
                }
                Err(error) => {
                    state.error = Some(error);
                    Task::none()
                }
            }
        }
        Message::Disconnect => {
            state.session = None;
            state.target = None;
            state.error = None;
            state.lines.clear();
            Task::none()
        }
        Message::Input(value) => {
            state.input = value;
            state.history_pos = None;
            Task::none()
        }
        Message::Submit => {
            let command = state.input.trim().to_string();
            let Some(session) = state.session.clone() else {
                return Task::none();
            };
            if command.is_empty() || state.sending {
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
            push(state, command.clone(), true);
            Task::perform(
                async move {
                    let command = command.trim_start_matches('/').to_string();
                    session.lock().await.command(&command).await
                },
                Message::Output,
            )
        }
        Message::Output(result) => {
            state.sending = false;
            match result {
                Ok(output) if output.trim().is_empty() => {}
                Ok(output) => {
                    for line in output.lines() {
                        push(state, line.to_string(), false);
                    }
                }
                Err(error) => {
                    state.session = None;
                    state.error = Some(format!("Disconnected. {error}"));
                }
            }
            operation::focus(INPUT_ID)
        }
        Message::ArrowKey(up) => {
            if ctx.route != Route::Remote || state.history.is_empty() || state.session.is_none() {
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

fn step_history(state: &mut State, up: bool) {
    let len = state.history.len();
    let next = match (state.history_pos, up) {
        (None, true) => {
            state.draft = std::mem::take(&mut state.input);
            Some(len - 1)
        }
        (None, false) => return,
        (Some(pos), true) => Some(pos.saturating_sub(1)),
        (Some(pos), false) if pos + 1 < len => Some(pos + 1),
        (Some(_), false) => None,
    };
    state.history_pos = next;
    state.input = match next {
        Some(pos) => state.history[pos].clone(),
        None => std::mem::take(&mut state.draft),
    };
}

pub fn view<'a>(state: &'a State, _ctx: &'a Ctx) -> Element<'a, Message> {
    let main: Element<'a, Message> = if let Some(form) = &state.form {
        form_view(form, state.saving)
    } else if let Some(target) = &state.target {
        console_view(state, target)
    } else {
        container(
            text("Pick a connection to open its console.")
                .size(TEXT_MD)
                .style(style::muted),
        )
        .center(Length::Fill)
        .into()
    };
    column![
        text("Remote consoles").size(TEXT_2XL).font(UI_SEMIBOLD),
        row![
            sidebar(state),
            container(main).width(Length::Fill).height(Length::Fill)
        ]
        .spacing(SPACE_6)
        .height(Length::Fill)
    ]
    .spacing(SPACE_5)
    .into()
}

fn heading<'a>(label: &'a str) -> Element<'a, Message> {
    text(label)
        .size(TEXT_XS)
        .font(UI_MEDIUM)
        .style(style::muted)
        .into()
}

fn entry<'a>(
    label: &'a str,
    detail: String,
    selected: bool,
    message: Option<Message>,
) -> Element<'a, Message> {
    button(
        column![
            text(label).size(TEXT_MD),
            text(detail).size(TEXT_SM).style(style::muted)
        ]
        .spacing(2),
    )
    .width(Length::Fill)
    .padding([8, 12])
    .style(style::sidebar_item(selected))
    .on_press_maybe(message)
    .into()
}

fn sidebar(state: &State) -> Element<'_, Message> {
    let active = |host: &str, port: u16| {
        state
            .target
            .as_ref()
            .is_some_and(|t| t.host == host && t.port == port)
    };
    let idle = !state.connecting;
    let mut saved = column![heading("Saved")].spacing(SPACE_1);
    if state.connections.is_empty() {
        saved = saved.push(
            text("No saved connections yet.")
                .size(TEXT_SM)
                .style(style::faint),
        );
    }
    for connection in &state.connections {
        saved = saved.push(
            row![
                entry(
                    &connection.name,
                    format!("{}:{}", connection.host, connection.port),
                    active(&connection.host, connection.port),
                    idle.then(|| Message::Connect(target(connection))),
                ),
                sized_icon(icon(Icon::Edit, 14.0), BUTTON_SM)
                    .style(style::ghost)
                    .on_press(Message::Edit(connection.id.clone())),
            ]
            .spacing(SPACE_1)
            .align_y(Alignment::Center),
        );
    }
    saved = saved.push(
        sized(
            row![
                icon(Icon::Plus, 14.0),
                text("Add connection").size(TEXT_SM).font(UI_MEDIUM)
            ]
            .spacing(SPACE_1)
            .align_y(Alignment::Center),
            BUTTON_SM,
        )
        .style(style::ghost)
        .on_press(Message::New),
    );

    let mut local = column![heading("Your servers")].spacing(SPACE_1);
    if state.local.is_empty() {
        local = local.push(
            text("Set enable-rcon to true in a server's properties to connect to it here.")
                .size(TEXT_SM)
                .style(style::faint),
        );
    }
    for server in &state.local {
        local = local.push(entry(
            &server.label,
            format!("localhost:{}", server.port),
            active(&server.host, server.port),
            idle.then(|| Message::Connect(server.clone())),
        ));
    }

    scrollable(column![saved, local].spacing(SPACE_6))
        .style(style::scroll)
        .width(260)
        .height(Length::Fill)
        .into()
}

fn labelled<'a>(label: &'a str, input: Element<'a, Message>) -> Element<'a, Message> {
    column![text(label).size(TEXT_SM).style(style::muted), input]
        .spacing(SPACE_1)
        .into()
}

fn input<'a>(
    placeholder: &str,
    value: &'a str,
    on_input: fn(String) -> Message,
) -> text_input::TextInput<'a, Message> {
    text_input(placeholder, value)
        .size(TEXT_MD)
        .padding([8, 12])
        .style(style::input)
        .on_input(on_input)
}

fn form_view(form: &Form, saving: bool) -> Element<'_, Message> {
    let ready = form.ready();
    let submit = ready.then_some(Message::SaveForm);
    let port_invalid = form.port().is_none();
    let mut actions = row![].spacing(SPACE_2).align_y(Alignment::Center);
    if let Some(id) = &form.id {
        actions = actions.push(
            sized(text("Delete").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                .style(style::danger)
                .on_press_maybe((!saving).then(|| Message::Delete(id.clone()))),
        );
    }
    actions = actions
        .push(Space::new().width(Length::Fill))
        .push(
            sized(text("Cancel").size(TEXT_MD), BUTTON_MD)
                .style(style::ghost)
                .on_press(Message::CancelForm),
        )
        .push(busy_button(
            "Save",
            "Saving",
            saving,
            submit.clone(),
            style::primary,
            BUTTON_MD,
        ));
    let fields: Column<'_, Message> = column![
        text(if form.id.is_some() {
            "Edit connection"
        } else {
            "New connection"
        })
        .size(TEXT_XL)
        .font(UI_SEMIBOLD),
        labelled(
            "Name",
            input("Friend's server", &form.name, Message::FormName)
                .on_submit_maybe(submit.clone())
                .into()
        ),
        row![
            container(labelled(
                "Host",
                input("play.example.com", &form.host, Message::FormHost)
                    .on_submit_maybe(submit.clone())
                    .into()
            ))
            .width(Length::Fill),
            container(labelled(
                "Port",
                input("25575", &form.port, Message::FormPort)
                    .style(if port_invalid {
                        style::input_error
                    } else {
                        style::input
                    })
                    .on_submit_maybe(submit.clone())
                    .into()
            ))
            .width(120),
        ]
        .spacing(SPACE_3),
        labelled(
            "Password",
            input("rcon.password", &form.password, Message::FormPassword)
                .secure(true)
                .on_submit_maybe(submit)
                .into()
        ),
        text("Saved on this PC with your Monolith settings.")
            .size(TEXT_SM)
            .style(style::faint),
        actions,
    ]
    .spacing(SPACE_4);
    container(fields).max_width(520).into()
}

fn console_view<'a>(state: &'a State, target: &'a Target) -> Element<'a, Message> {
    let status = if state.connecting {
        "Connecting".to_string()
    } else if state.session.is_some() {
        format!("Connected to {}:{}", target.host, target.port)
    } else {
        "Not connected".to_string()
    };
    let mut header = row![column![
        text(&target.label).size(TEXT_LG).font(UI_MEDIUM),
        text(status).size(TEXT_SM).style(style::muted)
    ]
    .spacing(2)
    .width(Length::Fill)]
    .spacing(SPACE_2)
    .align_y(Alignment::Center);
    if state.session.is_none() && !state.connecting {
        header = header.push(
            sized(text("Reconnect").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                .style(style::secondary)
                .on_press(Message::Connect(target.clone())),
        );
    }
    header = header.push(
        sized(text("Close").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
            .style(style::ghost)
            .on_press_maybe((!state.connecting).then_some(Message::Disconnect)),
    );

    let log: Element<'a, Message> = if state.lines.is_empty() {
        let note = if state.connecting {
            "Connecting"
        } else {
            "Run a command to see its output here."
        };
        container(text(note).size(TEXT_MD).style(style::muted))
            .center(Length::Fill)
            .into()
    } else {
        let list = state.lines.iter().fold(column![].spacing(2), |list, line| {
            let content: Element<'a, Message> = if line.command {
                text(format!("> {}", line.text))
                    .color(INK_SECONDARY)
                    .size(TEXT_SM)
                    .font(MONO)
                    .into()
            } else {
                rich_text(legacy_spans(&line.text, LOG_INFO))
                    .size(TEXT_SM)
                    .font(MONO)
                    .into()
            };
            list.push(content)
        });
        scrollable(
            container(list)
                .padding([SPACE_3, SPACE_4])
                .width(Length::Fill),
        )
        .anchor_bottom()
        .style(style::scroll)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    };

    let connected = state.session.is_some();
    let mut field = text_input(
        if connected {
            "Type a command and press Enter"
        } else {
            "Connect to send commands"
        },
        &state.input,
    )
    .id(INPUT_ID)
    .size(TEXT_MD)
    .font(MONO)
    .padding([SPACE_2, SPACE_3])
    .style(style::input)
    .width(Length::Fill);
    if connected {
        field = field.on_input(Message::Input).on_submit(Message::Submit);
    }
    let can_send = connected && !state.sending && !state.input.trim().is_empty();
    let send = busy_button(
        "Send",
        "Sending",
        state.sending,
        can_send.then_some(Message::Submit),
        style::primary,
        BUTTON_MD,
    );

    let mut body = column![header, rule::horizontal(1).style(style::hairline)].spacing(SPACE_3);
    if let Some(error) = &state.error {
        body = body.push(text(error).size(TEXT_SM).style(style::colored(CORAL)));
    }
    body.push(
        container(log)
            .style(style::console)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .push(
        row![field, send]
            .spacing(SPACE_2)
            .align_y(Alignment::Center),
    )
    .into()
}

pub fn on_event(_state: &mut State, _event: &Event, _ctx: &Ctx) -> Task<Message> {
    Task::none()
}

pub fn on_show(_state: &mut State, ctx: &Ctx) -> Task<Message> {
    let servers: Vec<(String, PathBuf)> = ctx
        .servers
        .iter()
        .map(|s| (s.name.clone(), PathBuf::from(&s.server_path)))
        .collect();
    Task::batch([
        Task::perform(rcon::list_connections(), Message::Loaded),
        Task::perform(
            async move {
                let mut local = Vec::new();
                for (name, path) in servers {
                    if let Some((port, password)) = rcon::local_rcon(&path).await {
                        local.push(Target {
                            label: name,
                            host: "127.0.0.1".into(),
                            port,
                            password,
                        });
                    }
                }
                local
            },
            Message::LocalLoaded,
        ),
    ])
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
