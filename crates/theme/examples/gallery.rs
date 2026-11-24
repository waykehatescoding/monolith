use std::time::Duration;

use iced::widget::{
    button, checkbox, column, container, pick_list, progress_bar, row, rule, stack, text,
    text_input, toggler, Space,
};
use iced::window::{Direction, Screenshot};
use iced::{window, Alignment, Color, Element, Length, Size, Task, Theme};
use monolith_theme::token::*;
use monolith_theme::widget::{drag_region, resize_grips, server_icon, window_controls};
use monolith_theme::{
    app_icon, icon, icon_colored, style, theme, Icon, FONT_BYTES, MONO, UI, UI_MEDIUM, UI_SEMIBOLD,
};

const TABS: [&str; 7] = [
    "Overview", "Console", "Files", "Players", "Content", "Backups", "Settings",
];
const SERVERS: [(&str, Color); 5] = [
    ("Survival", MOSS),
    ("Creative Build", MOSS),
    ("Modded SMP", AMBER),
    ("Friends Realm", STONE),
    ("Skyblock", CORAL),
];
const JAVA: [&str; 3] = ["Java 21", "Java 17", "Java 8"];

#[derive(Debug, Clone)]
enum Message {
    Select(usize),
    Tab(usize),
    Noop,
    Toggle(bool),
    Check(bool),
    Java(&'static str),
    Capture,
    Shot(Screenshot),
    Drag,
    Resize(Direction),
    Minimize,
    ToggleMaximize,
    Maximized(bool),
    Close,
}

struct Gallery {
    server: usize,
    tab: usize,
    toggle: bool,
    check: bool,
    java: Option<&'static str>,
    shot: Option<String>,
    maximized: bool,
}

fn main() -> iced::Result {
    let shot = std::env::args().skip_while(|a| a != "--screenshot").nth(1);
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
                    server: 0,
                    tab: 0,
                    toggle: true,
                    check: true,
                    java: Some("Java 21"),
                    shot: shot.clone(),
                    maximized: false,
                },
                task,
            )
        },
        update,
        view,
    )
    .title("Monolith")
    .theme(|_: &Gallery| theme())
    .default_font(UI)
    .window_size(Size::new(1440.0, 900.0))
    .decorations(false)
    .centered();
    for bytes in FONT_BYTES {
        app = app.font(bytes);
    }
    app.run()
}

fn update(state: &mut Gallery, message: Message) -> Task<Message> {
    match message {
        Message::Select(index) => state.server = index,
        Message::Tab(index) => state.tab = index,
        Message::Noop => {}
        Message::Toggle(value) => state.toggle = value,
        Message::Check(value) => state.check = value,
        Message::Java(value) => state.java = Some(value),
        Message::Drag => return window::oldest().and_then(window::drag),
        Message::Resize(direction) => {
            return window::oldest().and_then(move |id| window::drag_resize(id, direction));
        }
        Message::Minimize => return window::oldest().and_then(|id| window::minimize(id, true)),
        Message::ToggleMaximize => {
            return window::oldest().and_then(|id| {
                window::toggle_maximize(id).chain(window::is_maximized(id).map(Message::Maximized))
            });
        }
        Message::Maximized(value) => state.maximized = value,
        Message::Close => return iced::exit(),
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

fn label<'a>(value: impl text::IntoFragment<'a>, glyph: Icon) -> Element<'a, Message> {
    row![icon(glyph, 16.0), text(value).size(TEXT_MD).font(UI_MEDIUM)]
        .spacing(SPACE_2)
        .align_y(Alignment::Center)
        .into()
}

fn sidebar(state: &Gallery) -> Element<'_, Message> {
    let brand = row![
        app_icon(28.0),
        text("Monolith").size(TEXT_LG).font(UI_SEMIBOLD)
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center);

    let servers = SERVERS.iter().enumerate().fold(
        column![].spacing(SPACE_1),
        |list, (index, (name, color))| {
            let tip = match *color {
                c if c == MOSS => "Online · 7/20 players",
                c if c == AMBER => "Starting",
                c if c == CORAL => "Crashed",
                _ => "Offline",
            };
            let entry = row![server_icon(*color, tip), text(*name).size(TEXT_MD)]
                .spacing(SPACE_3)
                .align_y(Alignment::Center);
            list.push(
                button(entry)
                    .width(Length::Fill)
                    .padding([8, 12])
                    .style(style::sidebar_item(index == state.server))
                    .on_press(Message::Select(index)),
            )
        },
    );

    let footer = column![
        button(label("New server", Icon::Plus))
            .width(Length::Fill)
            .padding([8, 12])
            .style(style::sidebar_item(false))
            .on_press(Message::Noop),
        button(label("Settings", Icon::Settings))
            .width(Length::Fill)
            .padding([8, 12])
            .style(style::sidebar_item(false))
            .on_press(Message::Noop),
    ]
    .spacing(SPACE_1);

    container(
        column![
            drag_region(
                container(brand)
                    .padding([SPACE_2, SPACE_3])
                    .width(Length::Fill),
                Message::Drag,
                Message::ToggleMaximize,
            ),
            Space::new().height(SPACE_5),
            servers,
            Space::new().height(Length::Fill),
            footer
        ]
        .height(Length::Fill),
    )
    .padding(SPACE_3)
    .width(SIDEBAR_WIDTH)
    .height(Length::Fill)
    .style(style::ground)
    .into()
}

fn readout<'a>(name: &'a str, value: &'a str, unit: &'a str) -> Element<'a, Message> {
    column![
        text(name).size(TEXT_SM).style(style::muted),
        row![
            text(value).size(TEXT_XL).font(UI_SEMIBOLD),
            text(unit).size(TEXT_MD).style(style::muted)
        ]
        .spacing(SPACE_1)
        .align_y(Alignment::End)
    ]
    .spacing(SPACE_1)
    .into()
}

fn tabs(state: &Gallery) -> Element<'_, Message> {
    TABS.iter()
        .enumerate()
        .fold(row![].spacing(SPACE_1), |bar, (index, name)| {
            bar.push(
                button(text(*name).size(TEXT_MD).font(UI_MEDIUM))
                    .padding([6, 12])
                    .style(style::tab(index == state.tab))
                    .on_press(Message::Tab(index)),
            )
        })
        .into()
}

fn log<'a>(time: &'a str, level: &'a str, color: Color, body: &'a str) -> Element<'a, Message> {
    row![
        text(time).size(TEXT_SM).font(MONO).style(style::faint),
        text(level).size(TEXT_SM).font(MONO).color(color),
        text(body).size(TEXT_SM).font(MONO).color(color)
    ]
    .spacing(SPACE_3)
    .into()
}

fn console<'a>() -> Element<'a, Message> {
    container(
        column![
            log(
                "14:02:11",
                "INFO ",
                LOG_INFO,
                "Done (4.812s)! For help, type \"help\""
            ),
            log("14:02:36", "INFO ", LOG_INFO, "Steve joined the game"),
            log("14:02:41", "INFO ", LOG_INFO, "Alex joined the game"),
            log(
                "14:03:07",
                "INFO ",
                MOSS,
                "Steve has made the advancement [Stone Age]"
            ),
            log(
                "14:04:52",
                "WARN ",
                AMBER,
                "Can't keep up! Is the server overloaded? Running 2341ms behind"
            ),
            log(
                "14:05:18",
                "ERROR",
                CORAL,
                "Could not pass event PlayerMoveEvent to WorldGuard v7.0.12"
            ),
            log(
                "14:05:44",
                "INFO ",
                LOG_INFO,
                "Alex lost connection: Disconnected"
            ),
        ]
        .spacing(SPACE_2),
    )
    .padding(SPACE_4)
    .width(Length::Fill)
    .style(style::console)
    .into()
}

fn state_button<'a>(
    name: &'a str,
    look: fn(&Theme, button::Status) -> button::Style,
    status: button::Status,
) -> Element<'a, Message> {
    let base = button(text(name).size(TEXT_MD).font(UI_MEDIUM))
        .padding([6, 16])
        .style(move |theme: &Theme, _| look(theme, status));
    if status == button::Status::Disabled {
        base.into()
    } else {
        base.on_press(Message::Noop).into()
    }
}

fn field<'a>(
    value: &'a str,
    status: text_input::Status,
    invalid: bool,
    helper: &'a str,
) -> Element<'a, Message> {
    let input = text_input("Server name", value)
        .size(TEXT_MD)
        .padding([8, 12])
        .on_input(|_| Message::Noop)
        .style(move |theme: &Theme, _| {
            if invalid {
                style::input_error(theme, status)
            } else {
                style::input(theme, status)
            }
        });
    let tint = if invalid { CORAL } else { INK_TERTIARY };
    column![input, text(helper).size(TEXT_XS).color(tint)]
        .spacing(SPACE_1)
        .width(Length::Fill)
        .into()
}

fn states(state: &Gallery) -> Element<'_, Message> {
    let buttons = row![
        state_button("Start", style::primary, button::Status::Active),
        state_button("Hover", style::secondary, button::Status::Hovered),
        state_button("Pressed", style::secondary, button::Status::Pressed),
        state_button("Disabled", style::primary, button::Status::Disabled),
        state_button("Delete", style::danger, button::Status::Active),
    ]
    .spacing(SPACE_2);

    let inputs = row![
        field(
            "Survival",
            text_input::Status::Active,
            false,
            "Shown in the sidebar"
        ),
        field(
            "Surviv",
            text_input::Status::Focused { is_hovered: false },
            false,
            "Letters, numbers and spaces"
        ),
        field(
            "Survival/",
            text_input::Status::Active,
            true,
            "Remove the slash and try again"
        ),
    ]
    .spacing(SPACE_3)
    .width(640);

    let controls = row![
        toggler(state.toggle)
            .label("Auto-restart")
            .size(18)
            .text_size(TEXT_MD)
            .spacing(SPACE_2)
            .style(style::switch)
            .on_toggle(Message::Toggle),
        checkbox(state.check)
            .label("Backup on stop")
            .size(18)
            .text_size(TEXT_MD)
            .style(style::check)
            .on_toggle(Message::Check),
        pick_list(JAVA, state.java, Message::Java)
            .padding([6, 12])
            .text_size(TEXT_MD)
            .width(130)
            .style(style::pick)
            .menu_style(style::pick_menu),
        column![
            text("Memory 2.4 of 4 GB").size(TEXT_SM).style(style::muted),
            progress_bar(0.0..=4.0, 2.4).girth(6).style(style::progress)
        ]
        .spacing(SPACE_2)
        .width(170),
    ]
    .spacing(SPACE_5)
    .align_y(Alignment::Center);

    column![buttons, inputs, controls]
        .spacing(SPACE_5)
        .width(Length::Fill)
        .into()
}

fn dialog<'a>() -> Element<'a, Message> {
    container(
        column![
            column![
                text("Delete Skyblock?").size(TEXT_LG).font(UI_SEMIBOLD),
                text("The world, plugins and 3 backups will be removed from this computer. This cannot be undone.")
                    .size(TEXT_MD)
                    .style(style::muted)
            ]
            .spacing(SPACE_2),
            row![
                Space::new().width(Length::Fill),
                button(text("Cancel").size(TEXT_MD).font(UI_MEDIUM))
                    .padding([6, 16])
                    .style(style::ghost)
                    .on_press(Message::Noop),
                button(text("Delete server").size(TEXT_MD).font(UI_MEDIUM))
                    .padding([6, 16])
                    .style(|theme: &Theme, status| match status {
                        button::Status::Active => style::danger(theme, button::Status::Hovered),
                        other => style::danger(theme, other),
                    })
                    .on_press(Message::Noop),
            ]
            .spacing(SPACE_2)
        ]
        .spacing(SPACE_5),
    )
    .padding(SPACE_5)
    .width(400)
    .style(style::overlay)
    .into()
}

fn main_pane(state: &Gallery) -> Element<'_, Message> {
    let (name, color) = SERVERS[state.server];
    let status = match color {
        c if c == MOSS => "Online",
        c if c == AMBER => "Starting",
        c if c == CORAL => "Crashed",
        _ => "Offline",
    };

    let header = row![
        column![
            text(name).size(TEXT_2XL).font(UI_SEMIBOLD),
            row![
                text("Paper 1.21.4 · ").size(TEXT_MD).style(style::muted),
                text(status)
                    .size(TEXT_MD)
                    .font(UI_MEDIUM)
                    .color(if color == STONE { INK_SECONDARY } else { color }),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(SPACE_2),
        Space::new().width(Length::Fill),
        row![
            button(label("Restart", Icon::Restart))
                .padding([8, 16])
                .style(style::secondary)
                .on_press(Message::Noop),
            button(label("Share", Icon::Share))
                .padding([8, 16])
                .style(style::secondary)
                .on_press(Message::Noop),
            button(
                row![
                    icon_colored(Icon::Stop, 16.0, GROUND),
                    text("Stop").size(TEXT_MD).font(UI_SEMIBOLD)
                ]
                .spacing(SPACE_2)
                .align_y(Alignment::Center)
            )
            .padding([8, 20])
            .style(style::primary)
            .on_press(Message::Noop),
        ]
        .spacing(SPACE_2)
        .align_y(Alignment::Center),
    ]
    .align_y(Alignment::Start);

    let address = row![
        icon_colored(Icon::Globe, 16.0, INK_SECONDARY),
        text("192.168.1.24:25565").size(TEXT_MD).font(MONO),
        button(icon(Icon::Copy, 16.0))
            .width(28)
            .height(28)
            .padding(6)
            .style(style::icon_button)
            .on_press(Message::Noop),
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center);

    let readouts = row![
        readout("Players", "7 / 20", ""),
        readout("TPS", "19.98", ""),
        readout("Memory", "2.4 / 4", "GB"),
        readout("Uptime", "3h 12m", ""),
    ]
    .spacing(SPACE_7);

    let body = column![
        column![header, address].spacing(SPACE_3),
        readouts,
        column![tabs(state), console()].spacing(SPACE_3),
        rule::horizontal(1).style(style::hairline),
        states(state),
    ]
    .spacing(SPACE_6)
    .max_width(1000);

    container(body)
        .padding(iced::Padding {
            top: SPACE_2,
            right: 40.0,
            bottom: SPACE_6,
            left: 40.0,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::surface)
        .into()
}

fn title_bar(state: &Gallery) -> Element<'_, Message> {
    row![
        drag_region(
            Space::new()
                .width(Length::Fill)
                .height(Length::Fixed(TITLE_BAR_HEIGHT)),
            Message::Drag,
            Message::ToggleMaximize,
        ),
        window_controls(
            state.maximized,
            Message::Minimize,
            Message::ToggleMaximize,
            Message::Close,
        ),
    ]
    .into()
}

fn view(state: &Gallery) -> Element<'_, Message> {
    let app = row![
        sidebar(state),
        container(column![title_bar(state), main_pane(state)]).style(style::surface)
    ];
    stack![
        app,
        container(dialog())
            .padding([SPACE_6, 40.0])
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::End)
            .align_y(Alignment::End),
        resize_grips(Message::Resize)
    ]
    .into()
}
