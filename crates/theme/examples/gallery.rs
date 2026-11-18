use std::time::Duration;

use iced::widget::{
    button, checkbox, column, container, pick_list, progress_bar, row, rule, scrollable, slider,
    text, text_input, toggler, tooltip, Column, Space,
};
use iced::window::Screenshot;
use iced::{window, Color, Element, Length, Size, Task, Theme};
use monolith_theme::token::*;
use monolith_theme::widget::{chunk_tile, module};
use monolith_theme::{
    icon, icon_colored, style, theme, Icon, FONT_BYTES, MONO, MONO_BOLD, UI, UI_MEDIUM,
    UI_SEMIBOLD,
};

const PLATFORMS: [&str; 4] = ["Paper", "Purpur", "Fabric", "Vanilla"];

#[derive(Debug, Clone)]
enum Message {
    Noop,
    Name(String),
    Check(bool),
    Toggle(bool),
    Memory(f32),
    Platform(&'static str),
    Capture,
    Shot(Screenshot),
}

struct Gallery {
    name: String,
    check: bool,
    toggle: bool,
    memory: f32,
    platform: Option<&'static str>,
    shot: Option<String>,
}

fn main() -> iced::Result {
    let shot = std::env::args()
        .skip_while(|a| a != "--screenshot")
        .nth(1);
    let mut app = iced::application(
        move || {
            let task = if shot.is_some() {
                Task::perform(tokio::time::sleep(Duration::from_millis(900)), |_| {
                    Message::Capture
                })
            } else {
                Task::none()
            };
            (
                Gallery {
                    name: "Survival".into(),
                    check: true,
                    toggle: true,
                    memory: 4.0,
                    platform: Some("Paper"),
                    shot: shot.clone(),
                },
                task,
            )
        },
        update,
        view,
    )
    .title("Monolith design kit")
    .theme(|_: &Gallery| theme())
    .default_font(UI)
    .window_size(Size::new(1640.0, 1040.0))
    .centered();
    for bytes in FONT_BYTES {
        app = app.font(bytes);
    }
    app.run()
}

fn update(state: &mut Gallery, message: Message) -> Task<Message> {
    match message {
        Message::Noop => {}
        Message::Name(value) => state.name = value,
        Message::Check(value) => state.check = value,
        Message::Toggle(value) => state.toggle = value,
        Message::Memory(value) => state.memory = value,
        Message::Platform(value) => state.platform = Some(value),
        Message::Capture => {
            return window::oldest().then(|id| match id {
                Some(id) => window::screenshot(id).map(Message::Shot),
                None => Task::none(),
            });
        }
        Message::Shot(screenshot) => {
            if let Some(path) = &state.shot {
                let saved = image::save_buffer(
                    path,
                    &screenshot.rgba,
                    screenshot.size.width,
                    screenshot.size.height,
                    image::ColorType::Rgba8,
                );
                if let Err(error) = saved {
                    eprintln!("could not save screenshot: {error}");
                }
            }
            return iced::exit();
        }
    }
    Task::none()
}

fn caption<'a>(value: &'a str) -> Element<'a, Message> {
    text(value).size(TEXT_XS).style(style::muted).into()
}

fn heading<'a>(value: &'a str, size: f32) -> Element<'a, Message> {
    text(value).size(size).font(UI_SEMIBOLD).into()
}

fn mono<'a>(value: impl text::IntoFragment<'a>, size: f32) -> Element<'a, Message> {
    text(value).size(size).font(MONO).into()
}

const STATES: [(&str, button::Status); 4] = [
    ("Rest", button::Status::Active),
    ("Hover", button::Status::Hovered),
    ("Pressed", button::Status::Pressed),
    ("Disabled", button::Status::Disabled),
];

fn button_row<'a>(
    label: &'a str,
    glyph: Icon,
    fg: Color,
    look: fn(&Theme, button::Status) -> button::Style,
) -> Element<'a, Message> {
    let cells = STATES.iter().map(|(_, status)| {
        let status = *status;
        let ink = if status == button::Status::Disabled {
            INK_MUTED.scale_alpha(0.45)
        } else {
            fg
        };
        button(
            row![icon_colored(glyph, 16.0, ink), text(label).size(TEXT_MD).font(UI_MEDIUM)]
                .spacing(SPACE_2)
                .align_y(iced::Alignment::Center),
        )
        .padding([6, 12])
        .width(112)
        .style(move |theme, _| look(theme, status))
        .on_press(Message::Noop)
        .into()
    });
    row(cells).spacing(SPACE_2).into()
}

fn forced_input<'a>(
    value: &'a str,
    status: text_input::Status,
    look: fn(&Theme, text_input::Status) -> text_input::Style,
) -> Element<'a, Message> {
    text_input("Server name", value)
        .on_input(Message::Name)
        .size(TEXT_MD)
        .padding([8, 12])
        .style(move |theme, _| look(theme, status))
        .into()
}

fn colours<'a>() -> Element<'a, Message> {
    let swatch = |name: &'a str, hex: &'a str, color: Color| -> Element<'a, Message> {
        column![
            container(Space::new())
                .width(Length::Fill)
                .height(36)
                .style(move |_: &Theme| container::Style {
                    background: Some(color.into()),
                    border: iced::Border {
                        color: LINE,
                        width: BORDER,
                        radius: RADIUS.into(),
                    },
                    ..container::Style::default()
                }),
            text(name).size(TEXT_SM).font(UI_MEDIUM),
            text(hex).size(TEXT_XS).font(MONO).style(style::muted),
        ]
        .spacing(SPACE_1)
        .width(Length::Fill)
        .into()
    };
    module(
        "Palette",
        column![
            row![
                swatch("Deepslate", "#14171b", GROUND),
                swatch("Stratum", "#1f242a", STRATUM),
                swatch("Stratum high", "#2c333b", STRATUM_HIGH),
                swatch("Ink", "#e9e6df", INK),
                swatch("Muted ink", "#a7a39a", INK_MUTED),
            ]
            .spacing(SPACE_2),
            row![
                swatch("Grass", "#6fbf4a", GRASS),
                swatch("Sand", "#e2c36b", SAND),
                swatch("Redstone", "#d8473c", REDSTONE),
                swatch("Water", "#3f86c6", WATER),
                swatch("Stone", "#7d838a", STONE),
            ]
            .spacing(SPACE_2),
        ]
        .spacing(SPACE_3),
    )
}

fn typography<'a>() -> Element<'a, Message> {
    let sizes = [
        (TEXT_DISPLAY, "40"),
        (TEXT_2XL, "28"),
        (TEXT_XL, "20"),
        (TEXT_LG, "16"),
        (TEXT_MD, "14"),
        (TEXT_SM, "13"),
        (TEXT_XS, "12"),
    ];
    let rows = sizes.iter().map(|(size, label)| {
        row![
            container(text(*label).size(TEXT_XS).font(MONO).style(style::muted)).width(28),
            text("Start your server")
                .size(*size)
                .font(if *size >= TEXT_XL { UI_SEMIBOLD } else { UI }),
        ]
        .spacing(SPACE_3)
        .align_y(iced::Alignment::Center)
        .into()
    });
    module(
        "Type",
        column![
            Column::with_children(rows).spacing(SPACE_2),
            rule::horizontal(1).style(style::hairline),
            row![
                column![
                    caption("Join address"),
                    text("play.monolith.gg:25565").size(TEXT_XL).font(MONO_BOLD),
                ]
                .spacing(SPACE_1),
                column![
                    caption("Live readout"),
                    mono("TPS 19.98   RAM 2.41/4.00 GB   0 4 -12", TEXT_MD),
                ]
                .spacing(SPACE_1),
            ]
            .spacing(SPACE_5),
        ]
        .spacing(SPACE_3),
    )
}

fn icons<'a>() -> Element<'a, Message> {
    let cells = Icon::ALL
        .iter()
        .map(|glyph| container(icon(*glyph, 22.0)).padding(SPACE_2).into());
    module("Icons", row(cells).spacing(SPACE_1).wrap())
}

fn console<'a>() -> Element<'a, Message> {
    let lines = [
        (INK_MUTED, "[04:12:09] [Server thread/INFO]: Starting minecraft server version 1.21.4"),
        (INK_MUTED, "[04:12:11] [Server thread/INFO]: Preparing spawn area: 84%"),
        (GRASS, "[04:12:14] [Server thread/INFO]: Done (4.812s)! For help, type \"help\""),
        (INK, "[04:12:40] [Server thread/INFO]: Steve joined the game"),
        (SAND, "[04:13:02] [Server thread/WARN]: Can't keep up! Is the server overloaded?"),
        (REDSTONE, "[04:13:55] [Server thread/ERROR]: Could not pass event PlayerMoveEvent"),
        (INK, "[04:14:10] [Server thread/INFO]: Alex joined the game"),
        (INK_MUTED, "[04:14:31] [Server thread/INFO]: Saved the game"),
        (INK, "[04:15:02] [Server thread/INFO]: Steve left the game"),
        (INK_MUTED, "[04:15:30] [Server thread/INFO]: Saved the game"),
    ];
    let body = Column::with_children(lines.iter().map(|(color, line)| {
        text(*line)
            .size(TEXT_SM)
            .font(MONO)
            .style(style::colored(*color))
            .into()
    }))
    .spacing(2);
    module(
        "Console",
        container(
            scrollable(container(body).padding([SPACE_2, SPACE_3]))
                .style(style::scroll)
                .height(150),
        )
        .style(style::well)
        .width(Length::Fill),
    )
}

fn buttons<'a>() -> Element<'a, Message> {
    let header = row(STATES.iter().map(|(name, _)| {
        container(caption(name)).width(Length::Fixed(112.0)).into()
    }))
    .spacing(SPACE_2);
    let icon_states = row(STATES.iter().map(|(_, status)| {
        let status = *status;
        let ink = if status == button::Status::Disabled {
            INK_MUTED.scale_alpha(0.45)
        } else {
            INK
        };
        container(
            tooltip(
                button(icon_colored(Icon::Copy, 16.0, ink))
                    .padding(8)
                    .style(move |theme, _| style::icon_button(theme, status))
                    .on_press(Message::Noop),
                container(text("Copy join address").size(TEXT_SM))
                    .padding([SPACE_1, SPACE_2])
                    .style(style::tooltip),
                tooltip::Position::Bottom,
            )
            .gap(SPACE_1),
        )
        .width(Length::Fixed(112.0))
        .into()
    }))
    .spacing(SPACE_2);
    module(
        "Buttons",
        column![
            header,
            button_row("Start", Icon::Play, GROUND, style::primary),
            button_row("Browse", Icon::Folder, INK, style::secondary),
            button_row("Refresh", Icon::Restart, INK_MUTED, style::ghost),
            button_row("Delete", Icon::Trash, GROUND, style::danger),
            button_row("Remove", Icon::Trash, REDSTONE, style::danger_ghost),
            icon_states,
        ]
        .spacing(SPACE_2),
    )
}

fn inputs<'a>(state: &'a Gallery) -> Element<'a, Message> {
    let field_header = row![
        container(caption("Rest")).width(Length::FillPortion(1)),
        container(caption("Hover")).width(Length::FillPortion(1)),
        container(caption("Focused")).width(Length::FillPortion(1)),
    ]
    .spacing(SPACE_3);
    let rest = text_input::Status::Active;
    let hover = text_input::Status::Hovered;
    let focus = text_input::Status::Focused { is_hovered: false };
    let off = text_input::Status::Disabled;
    let switches = |toggled: bool, disabled: bool| -> Element<'a, Message> {
        let base = toggler(toggled).label("Auto-restart").size(18).text_size(TEXT_MD).style(style::switch);
        if disabled { base.into() } else { base.on_toggle(Message::Toggle).into() }
    };
    module(
        "Fields",
        column![
            field_header,
            row![
                forced_input(&state.name, rest, style::input),
                forced_input(&state.name, hover, style::input),
                forced_input(&state.name, focus, style::input),
            ]
            .spacing(SPACE_3),
            row![
                text_input("Server name", "")
                    .size(TEXT_MD)
                    .padding([8, 12])
                    .style(style::input),
                forced_input("my server!", text_input::Status::Active, style::input_error),
                forced_input("Locked", off, style::input),
            ]
            .spacing(SPACE_3),
            caption("Names use letters, numbers and dashes. Rename it before creating."),
            rule::horizontal(1).style(style::hairline),
            row![
                pick_list(PLATFORMS, state.platform, Message::Platform)
                    .placeholder("Platform")
                    .padding([8, 12])
                    .text_size(TEXT_MD)
                    .style(style::pick)
                    .menu_style(style::pick_menu)
                    .width(Length::FillPortion(1)),
                column![
                    row![
                        text(format!("{:.1} GB", state.memory)).size(TEXT_MD).font(MONO),
                        caption("memory"),
                    ]
                    .spacing(SPACE_2),
                    slider(1.0..=16.0, state.memory, Message::Memory)
                        .step(0.5_f32)
                        .style(style::range),
                ]
                .spacing(SPACE_1)
                .width(Length::FillPortion(2)),
            ]
            .spacing(SPACE_3)
            .align_y(iced::Alignment::End),
            row![
                checkbox(state.check)
                    .label("Accept the EULA")
                    .size(18)
                    .text_size(TEXT_MD)
                    .style(style::check)
                    .on_toggle(Message::Check),
                checkbox(false).label("Unchecked").size(18).text_size(TEXT_MD).style(style::check).on_toggle(Message::Check),
                checkbox(true).label("Locked").size(18).text_size(TEXT_MD).style(style::check),
            ]
            .spacing(SPACE_5),
            row![switches(state.toggle, false), switches(false, true)].spacing(SPACE_5),
        ]
        .spacing(SPACE_3),
    )
}

fn mark<'a>(color: Color, players: u32) -> Element<'a, Message> {
    let side = 8.0 + (players as f32).sqrt() * 4.0;
    container(Space::new())
        .width(side)
        .height(side)
        .style(style::state_band(color))
        .into()
}

fn tile<'a>(
    name: &'a str,
    meta: &'a str,
    coord: &'a str,
    state: Color,
    status: &'a str,
    players: u32,
    focused: bool,
) -> Element<'a, Message> {
    chunk_tile(
        state,
        focused,
        column![
            row![
                column![
                    text(name).size(TEXT_LG).font(UI_SEMIBOLD),
                    text(meta).size(TEXT_SM).style(style::muted),
                ]
                .spacing(2)
                .width(Length::Fill),
                mono(coord, TEXT_XS),
            ],
            row![
                text(status).size(TEXT_SM).font(UI_MEDIUM).style(style::colored(state)),
                Space::new().width(Length::Fill),
                icon_colored(Icon::Users, 14.0, INK_MUTED),
                mono(players.to_string(), TEXT_SM),
                mark(state, players),
            ]
            .spacing(SPACE_2)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(SPACE_5),
    )
    .width(Length::Fill)
    .into()
}

fn tiles<'a>() -> Element<'a, Message> {
    module(
        "Chunk tiles",
        column![
            row![
                container(tile("Survival", "Paper 1.21.4", "0,0", GRASS, "Online", 7, true))
                    .width(Length::FillPortion(3)),
                container(tile("Creative", "Fabric 1.21.4", "1,0", SAND, "Starting", 0, false))
                    .width(Length::FillPortion(2)),
            ]
            .spacing(SPACE_3),
            row![
                container(tile("Modded", "Fabric 1.20.1", "0,1", REDSTONE, "Crashed", 0, false))
                    .width(Length::FillPortion(2)),
                container(tile("Archive", "Vanilla 1.16.5", "1,1", STONE, "Offline", 0, false))
                    .width(Length::FillPortion(2)),
                container(tile("Test", "Purpur 1.21", "2,1", GRASS, "Online", 2, false))
                    .width(Length::FillPortion(2)),
            ]
            .spacing(SPACE_3),
        ]
        .spacing(SPACE_3),
    )
}

fn navigation<'a>() -> Element<'a, Message> {
    let nav_item = |glyph: Icon, label: &'a str, selected: bool| -> Element<'a, Message> {
        button(
            row![icon(glyph, 18.0), text(label).size(TEXT_MD).font(UI_MEDIUM)]
                .spacing(SPACE_3)
                .align_y(iced::Alignment::Center),
        )
        .width(Length::Fill)
        .padding([8, 12])
        .style(style::nav(selected))
        .on_press(Message::Noop)
        .into()
    };
    let tab_item = |label: &'a str, selected: bool| -> Element<'a, Message> {
        button(text(label).size(TEXT_SM).font(UI_MEDIUM))
            .padding([6, 12])
            .style(style::tab(selected))
            .on_press(Message::Noop)
            .into()
    };
    module(
        "Navigation",
        row![
            container(
                column![
                    nav_item(Icon::Server, "Servers", true),
                    nav_item(Icon::Plus, "Create server", false),
                    nav_item(Icon::Settings, "Settings", false),
                ]
                .spacing(SPACE_1),
            )
            .padding(SPACE_2)
            .width(Length::Fixed(156.0))
            .style(style::rail),
            column![
                row![
                    tab_item("Console", true),
                    tab_item("Files", false),
                    tab_item("Players", false),
                    tab_item("Backups", false),
                ]
                .spacing(SPACE_1),
            ]
            .spacing(SPACE_3)
            .width(Length::Fill),
        ]
        .spacing(SPACE_5),
    )
}

fn feedback<'a>() -> Element<'a, Message> {
    let bar = |value: f32, look: fn(&Theme) -> progress_bar::Style| -> Element<'a, Message> {
        progress_bar(0.0..=100.0, value).girth(6).style(look).into()
    };
    let overlay = container(
        column![
            heading("Delete Survival?", TEXT_LG),
            text("This removes the server and its world. Backups stay in the backups folder.")
                .size(TEXT_SM)
                .style(style::muted),
            row![
                button(text("Keep server").size(TEXT_MD).font(UI_MEDIUM))
                    .padding([6, 12])
                    .style(style::secondary)
                    .on_press(Message::Noop),
                button(text("Delete server").size(TEXT_MD).font(UI_MEDIUM))
                    .padding([6, 12])
                    .style(style::danger)
                    .on_press(Message::Noop),
            ]
            .spacing(SPACE_2),
        ]
        .spacing(SPACE_3),
    )
    .padding(CHUNK)
    .width(Length::Fill)
    .style(style::overlay);
    module(
        "Progress and overlay",
        column![
            row![caption("Memory"), Space::new().width(Length::Fill), mono("2.4 / 4.0 GB", TEXT_SM)],
            bar(60.0, style::progress),
            row![caption("Memory near limit"), Space::new().width(Length::Fill), mono("3.5 / 4.0 GB", TEXT_SM)],
            bar(88.0, style::progress_warn),
            row![caption("Out of memory"), Space::new().width(Length::Fill), mono("4.0 / 4.0 GB", TEXT_SM)],
            bar(100.0, style::progress_danger),
            Space::new().height(SPACE_2),
            overlay,
        ]
        .spacing(SPACE_2),
    )
}

fn view(state: &Gallery) -> Element<'_, Message> {
    let left = column![colours(), typography(), icons()].spacing(CHUNK);
    let middle = column![buttons(), inputs(state), console()].spacing(CHUNK);
    let right = column![tiles(), navigation(), feedback()].spacing(CHUNK);
    container(
        row![
            container(left).width(Length::FillPortion(5)),
            container(middle).width(Length::FillPortion(5)),
            container(right).width(Length::FillPortion(5)),
        ]
        .spacing(CHUNK),
    )
    .padding(CHUNK)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(style::ground)
    .into()
}
