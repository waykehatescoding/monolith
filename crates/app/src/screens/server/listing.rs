use std::path::Path;

use iced::widget::image::Handle;
use iced::widget::{column, container, image, rich_text, row, text, Space};
use iced::{Alignment, Element, Length, Task};
use monolith_core::server_ping::ping;
use monolith_core::server_props::read_properties;
use monolith_core::types::{ServerProperty, ServerRecord};
use monolith_theme::token::{
    BUTTON_SM, CORAL, INK_SECONDARY, MOSS, SPACE_1, SPACE_2, SPACE_3, SPACE_4, TEXT_MD, TEXT_SM,
};
use monolith_theme::widget::sized_icon;
use monolith_theme::{icon, style, Icon, MONO, UI_SEMIBOLD};

use super::console::legacy_spans;

const ICON_SIZE: f32 = 48.0;

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    preview: Option<Preview>,
    loading: bool,
}

#[derive(Debug, Clone)]
pub struct Preview {
    server_id: String,
    icon: Option<Handle>,
    motd: String,
    version: String,
    players: String,
    ping: Option<u32>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    Loaded(Preview),
}

fn prop<'a>(props: &'a [ServerProperty], key: &str) -> Option<&'a str> {
    props
        .iter()
        .find(|property| property.key == key)
        .map(|property| property.value.trim())
}

fn unescape(motd: &str) -> String {
    motd.replace("\\u00A7", "§")
        .replace("\\u00a7", "§")
        .replace("\\n", "\n")
}

async fn load(server: ServerRecord, online: bool) -> Preview {
    let path = Path::new(&server.server_path);
    let props = read_properties(path).await;
    let port = prop(&props, "server-port")
        .and_then(|port| port.parse().ok())
        .unwrap_or(25565);
    let host = prop(&props, "server-ip")
        .filter(|ip| !ip.is_empty())
        .unwrap_or("127.0.0.1")
        .to_string();
    let live = match online {
        true => Some(ping(host, port).await),
        false => None,
    };
    let error = match &live {
        Some(Err(e)) => Some(format!("Could not ping the server: {e}")),
        _ => None,
    };
    if let Some(Ok(status)) = live {
        return Preview {
            server_id: server.id,
            icon: status.favicon.map(Handle::from_bytes),
            motd: status.motd,
            version: status.version,
            players: format!("{} / {}", status.online, status.max),
            ping: Some(status.ping_ms),
            error: None,
        };
    }
    let icon = tokio::fs::read(path.join("server-icon.png"))
        .await
        .ok()
        .map(Handle::from_bytes);
    Preview {
        icon,
        motd: unescape(prop(&props, "motd").unwrap_or("A Minecraft Server")),
        version: format!("{} {}", server.framework, server.version),
        players: format!("0 / {}", prop(&props, "max-players").unwrap_or("20")),
        ping: None,
        error,
        server_id: server.id,
    }
}

pub fn refresh(state: &mut State, server: &ServerRecord, online: bool) -> Task<Message> {
    if state.server_id.as_deref() != Some(server.id.as_str()) {
        *state = State {
            server_id: Some(server.id.clone()),
            ..State::default()
        };
    }
    state.loading = true;
    let demo = (online && crate::demo::active()).then(|| server.players.replace('/', " / "));
    Task::perform(
        load(server.clone(), online && demo.is_none()),
        |mut preview| {
            if let Some(players) = demo {
                preview.players = players;
                preview.ping = Some(14);
            }
            Message::Loaded(preview)
        },
    )
}

pub fn update(
    state: &mut State,
    message: Message,
    server: &ServerRecord,
    online: bool,
) -> Task<Message> {
    match message {
        Message::Refresh => refresh(state, server, online),
        Message::Loaded(preview) => {
            if state.server_id.as_deref() == Some(preview.server_id.as_str()) {
                state.loading = false;
                state.preview = Some(preview);
            }
            Task::none()
        }
    }
}

pub fn view<'a>(state: &'a State, server: &'a ServerRecord) -> Element<'a, Message> {
    let Some(preview) = &state.preview else {
        return Space::new().into();
    };
    let icon_box: Element<'a, Message> = match &preview.icon {
        Some(handle) => image(handle.clone())
            .width(ICON_SIZE)
            .height(ICON_SIZE)
            .into(),
        None => container(Space::new())
            .width(ICON_SIZE)
            .height(ICON_SIZE)
            .style(style::well)
            .into(),
    };
    let ping: Element<'a, Message> = match preview.ping {
        Some(ms) => text(format!("{ms} ms"))
            .size(TEXT_SM)
            .font(MONO)
            .style(style::colored(MOSS))
            .into(),
        None => text("Offline").size(TEXT_SM).style(style::muted).into(),
    };
    let top = row![
        text(server.name.as_str()).size(TEXT_MD).font(UI_SEMIBOLD),
        Space::new().width(Length::Fill),
        text(preview.version.as_str())
            .size(TEXT_SM)
            .style(style::muted),
        text(preview.players.as_str()).size(TEXT_SM).font(MONO),
        ping,
        sized_icon(icon(Icon::Restart, 14.0), BUTTON_SM)
            .style(style::icon_button)
            .on_press_maybe((!state.loading).then_some(Message::Refresh)),
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center);
    let motd = preview
        .motd
        .lines()
        .take(2)
        .fold(column![].spacing(2), |lines, line| {
            lines.push(
                rich_text::<(), Message, _, _>(legacy_spans(line, INK_SECONDARY))
                    .size(TEXT_MD)
                    .font(MONO),
            )
        });
    let mut details = column![top, motd].spacing(SPACE_1).width(Length::Fill);
    if let Some(error) = &preview.error {
        details = details.push(text(error.as_str()).size(TEXT_SM).color(CORAL));
    }
    container(
        row![icon_box, details]
            .spacing(SPACE_3)
            .align_y(Alignment::Start),
    )
    .padding([SPACE_2, SPACE_4])
    .width(Length::Fill)
    .style(style::panel)
    .into()
}
