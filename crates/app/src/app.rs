use std::time::Duration;

use iced::widget::text::Wrapping;
use iced::widget::{button, column, container, row, scrollable, text, Space, Stack};
use iced::window::{Direction, Screenshot};
use iced::{event, window, Alignment, Element, Length, Padding, Subscription, Task};
use monolith_core::events::{self, Event};
use monolith_core::types::{AppPreferences, ServerRecord, ServerRuntimeStatus};
use monolith_core::{app_preferences, lifecycle, server_store};
use monolith_theme::token::*;
use monolith_theme::widget::{drag_region, resize_grips, server_icon, window_controls};
use monolith_theme::{app_icon, icon, style, Icon, UI_MEDIUM, UI_SEMIBOLD};

use crate::ctx::{status_color, Action, Confirm, Ctx, Route, SettingsPage, Tab, ToastKind};
use crate::screens::server::{
    analytics, backups, console, content, files, overview, players, settings as server_settings,
};
use crate::screens::settings::{defaults, general, sharing};
use crate::screens::{self, assistant, create, onboarding, palette};
use crate::ui::confirm;
use crate::ui::overlay::modal;
use crate::ui::shortcuts::{self, Shortcut};
use crate::ui::toast::{self, Toast};
use crate::Launch;

type S = screens::Message;

#[derive(Clone)]
pub enum Message {
    Screen(S),
    Core(Event),
    ServersLoaded(Vec<ServerRecord>),
    PreferencesLoaded(AppPreferences),
    Shortcut(Shortcut),
    Goto(Route),
    SelectServer(String),
    OpenPalette,
    ClosePalette,
    ToggleAssistant,
    DismissToast(u64),
    ConfirmAccepted,
    ConfirmCancelled,
    ConfirmFinished(Result<String, String>),
    CloseRequested,
    CloseChecked(bool),
    CloseCancelled,
    CloseConfirmed,
    Quit,
    Drag,
    Resize(Direction),
    Minimize,
    ToggleMaximize,
    Maximized(bool),
    Capture,
    Shot(Screenshot),
}

pub struct App {
    ctx: Ctx,
    screens: screens::Screens,
    toasts: Vec<Toast>,
    next_toast: u64,
    confirm: Option<Confirm>,
    close_dialog: bool,
    maximized: bool,
    booted: bool,
    screenshot: Option<std::path::PathBuf>,
}

impl App {
    pub fn boot(launch: Launch) -> (App, Task<Message>) {
        let mut app = App {
            ctx: Ctx::new(launch.route),
            screens: screens::Screens::default(),
            toasts: Vec::new(),
            next_toast: 0,
            confirm: None,
            close_dialog: false,
            maximized: false,
            booted: false,
            screenshot: launch.screenshot.clone(),
        };
        app.ctx.palette_open = launch.palette;
        let mut tasks = vec![
            Task::perform(server_store::get_servers(), Message::ServersLoaded),
            Task::perform(
                app_preferences::get_app_preferences(),
                Message::PreferencesLoaded,
            ),
            Task::future(lifecycle::startup()).discard(),
            Task::future(lifecycle::run_auto_backups()).discard(),
            Task::future(lifecycle::run_scheduled_restarts()).discard(),
        ];
        if launch.screenshot.is_some() {
            tasks.push(Task::perform(
                tokio::time::sleep(Duration::from_millis(1200)),
                |_| Message::Capture,
            ));
        }
        (app, Task::batch(tasks))
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let mut tasks = vec![self.handle(message)];
        loop {
            let actions = self.ctx.take_actions();
            if actions.is_empty() {
                break;
            }
            for action in actions {
                tasks.push(self.apply(action));
            }
        }
        Task::batch(tasks)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(core_events),
            window::close_requests().map(|_| Message::CloseRequested),
            event::listen_with(key_message),
            self.screens.subscription().map(Message::Screen),
        ])
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Screen(message) => self.screens.update(message, &self.ctx).map(Message::Screen),
            Message::Core(event) => self.on_core_event(event),
            Message::ServersLoaded(servers) => self.on_servers_loaded(servers),
            Message::PreferencesLoaded(preferences) => {
                self.ctx.preferences = preferences;
                Task::none()
            }
            Message::Shortcut(shortcut) => self.on_shortcut(shortcut),
            Message::Goto(route) => self.go(route),
            Message::SelectServer(id) => self.select(id),
            Message::OpenPalette => self.apply(Action::OpenPalette),
            Message::ClosePalette => self.apply(Action::ClosePalette),
            Message::ToggleAssistant => self.apply(Action::ToggleAssistant),
            Message::DismissToast(id) => {
                self.toasts.retain(|toast| toast.id != id);
                Task::none()
            }
            Message::ConfirmAccepted => match self.confirm.take() {
                Some(confirm) => Task::perform((confirm.run)(), Message::ConfirmFinished),
                None => Task::none(),
            },
            Message::ConfirmCancelled => {
                self.confirm = None;
                Task::none()
            }
            Message::ConfirmFinished(result) => match result {
                Ok(text) => {
                    if !text.is_empty() {
                        self.ctx.success(text);
                    }
                    self.ctx.refresh_servers();
                    Task::none()
                }
                Err(error) => {
                    self.ctx.error(error);
                    Task::none()
                }
            },
            Message::CloseRequested => {
                if self.close_dialog {
                    Task::none()
                } else {
                    Task::perform(lifecycle::should_confirm_close(), Message::CloseChecked)
                }
            }
            Message::CloseChecked(true) => {
                self.close_dialog = true;
                Task::none()
            }
            Message::CloseChecked(false) | Message::CloseConfirmed => quit(),
            Message::CloseCancelled => {
                self.close_dialog = false;
                Task::none()
            }
            Message::Quit => iced::exit(),
            Message::Drag => window::oldest().and_then(window::drag),
            Message::Resize(direction) => {
                window::oldest().and_then(move |id| window::drag_resize(id, direction))
            }
            Message::Minimize => window::oldest().and_then(|id| window::minimize(id, true)),
            Message::ToggleMaximize => window::oldest().and_then(|id| {
                window::toggle_maximize(id).chain(window::is_maximized(id).map(Message::Maximized))
            }),
            Message::Maximized(value) => {
                self.maximized = value;
                Task::none()
            }
            Message::Capture => window::oldest().then(|id| match id {
                Some(id) => window::screenshot(id).map(Message::Shot),
                None => Task::none(),
            }),
            Message::Shot(screenshot) => {
                if let Some(path) = &self.screenshot {
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
                iced::exit()
            }
        }
    }

    fn apply(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::Navigate(route) => self.go(route),
            Action::Toast(kind, text) => self.push_toast(kind, text),
            Action::Confirm(confirm) => {
                self.confirm = Some(confirm);
                Task::none()
            }
            Action::SelectServer(id) => self.select(id),
            Action::RefreshServers => {
                Task::perform(server_store::get_servers(), Message::ServersLoaded)
            }
            Action::RefreshPreferences => Task::perform(
                app_preferences::get_app_preferences(),
                Message::PreferencesLoaded,
            ),
            Action::OpenPalette => {
                self.ctx.palette_open = true;
                palette::on_show(&mut self.screens.palette, &self.ctx)
                    .map(S::Palette)
                    .map(Message::Screen)
            }
            Action::ClosePalette => {
                self.ctx.palette_open = false;
                Task::none()
            }
            Action::ToggleAssistant => {
                self.ctx.assistant_open = !self.ctx.assistant_open;
                if self.ctx.assistant_open {
                    assistant::on_show(&mut self.screens.assistant, &self.ctx)
                        .map(S::Assistant)
                        .map(Message::Screen)
                } else {
                    Task::none()
                }
            }
        }
    }

    fn push_toast(&mut self, kind: ToastKind, text: String) -> Task<Message> {
        let id = self.next_toast;
        self.next_toast += 1;
        self.toasts.push(Toast { id, kind, text });
        let lifetime = match kind {
            ToastKind::Error => Duration::from_secs(9),
            _ => Duration::from_secs(4),
        };
        Task::perform(tokio::time::sleep(lifetime), move |_| {
            Message::DismissToast(id)
        })
    }

    fn go(&mut self, route: Route) -> Task<Message> {
        self.ctx.route = route;
        self.ctx.palette_open = false;
        self.show()
    }

    fn select(&mut self, id: String) -> Task<Message> {
        if self.ctx.server(&id).is_none() {
            return Task::none();
        }
        self.ctx.selected = Some(id);
        self.ctx.palette_open = false;
        if !matches!(self.ctx.route, Route::Server(_)) {
            self.ctx.route = Route::Server(Tab::Overview);
        }
        self.show()
    }

    fn on_shortcut(&mut self, shortcut: Shortcut) -> Task<Message> {
        match shortcut {
            Shortcut::Palette if self.ctx.palette_open => self.apply(Action::ClosePalette),
            Shortcut::Palette => self.apply(Action::OpenPalette),
            Shortcut::NewServer => self.go(Route::Create),
            Shortcut::Settings => self.go(Route::Settings(SettingsPage::General)),
            Shortcut::Server(index) => match self.ctx.servers.get(index) {
                Some(server) => {
                    let id = server.id.clone();
                    self.select(id)
                }
                None => Task::none(),
            },
            Shortcut::Escape => {
                if self.close_dialog {
                    self.close_dialog = false;
                } else if self.confirm.is_some() {
                    self.confirm = None;
                } else if self.ctx.palette_open {
                    self.ctx.palette_open = false;
                }
                Task::none()
            }
        }
    }

    fn on_core_event(&mut self, event: Event) -> Task<Message> {
        match &event {
            Event::ServerStatus(update) => {
                self.ctx
                    .statuses
                    .insert(update.server_id.clone(), update.status);
                if update.status == ServerRuntimeStatus::Offline {
                    self.ctx.stats.remove(&update.server_id);
                }
                match self
                    .ctx
                    .servers
                    .iter_mut()
                    .find(|server| server.id == update.server_id)
                {
                    Some(server) => {
                        server.status = update.status;
                        if let Some(players) = &update.players {
                            server.players = players.clone();
                        }
                    }
                    None => self.ctx.refresh_servers(),
                }
            }
            Event::ServerStats(stats) => {
                self.ctx.stats.insert(stats.server_id.clone(), stats.clone());
            }
            _ => {}
        }
        self.screens.on_event(&event, &self.ctx).map(Message::Screen)
    }

    fn on_servers_loaded(&mut self, servers: Vec<ServerRecord>) -> Task<Message> {
        for server in &servers {
            self.ctx.statuses.insert(server.id.clone(), server.status);
        }
        self.ctx
            .statuses
            .retain(|id, _| servers.iter().any(|server| server.id == *id));
        self.ctx.servers = servers;
        let known = self
            .ctx
            .selected
            .as_deref()
            .is_some_and(|id| self.ctx.server(id).is_some());
        let mut changed = false;
        if !known {
            self.ctx.selected = self.ctx.servers.first().map(|server| server.id.clone());
            changed = true;
        }
        if changed || !self.booted {
            self.booted = true;
            self.show()
        } else {
            Task::none()
        }
    }

    fn show(&mut self) -> Task<Message> {
        let ctx = &self.ctx;
        let screens = &mut self.screens;
        let task: Task<S> = match ctx.route {
            Route::Server(_) if ctx.selected.is_none() => {
                onboarding::on_show(&mut screens.onboarding, ctx).map(S::Onboarding)
            }
            Route::Server(tab) => {
                let header = overview::on_show(&mut screens.overview, ctx).map(S::Overview);
                let body = match tab {
                    Tab::Overview => Task::none(),
                    Tab::Console => console::on_show(&mut screens.console, ctx).map(S::Console),
                    Tab::Files => files::on_show(&mut screens.files, ctx).map(S::Files),
                    Tab::Players => players::on_show(&mut screens.players, ctx).map(S::Players),
                    Tab::Content => content::on_show(&mut screens.content, ctx).map(S::Content),
                    Tab::Backups => backups::on_show(&mut screens.backups, ctx).map(S::Backups),
                    Tab::Analytics => {
                        analytics::on_show(&mut screens.analytics, ctx).map(S::Analytics)
                    }
                    Tab::Settings => server_settings::on_show(&mut screens.server_settings, ctx)
                        .map(S::ServerSettings),
                };
                Task::batch([header, body])
            }
            Route::Create => create::on_show(&mut screens.create, ctx).map(S::Create),
            Route::Settings(SettingsPage::General | SettingsPage::Updates) => {
                general::on_show(&mut screens.general, ctx).map(S::General)
            }
            Route::Settings(SettingsPage::Defaults) => {
                defaults::on_show(&mut screens.defaults, ctx).map(S::Defaults)
            }
            Route::Settings(SettingsPage::Sharing) => {
                sharing::on_show(&mut screens.sharing, ctx).map(S::Sharing)
            }
            Route::Settings(SettingsPage::Assistant) => {
                assistant::on_show(&mut screens.assistant, ctx).map(S::Assistant)
            }
            Route::Empty => onboarding::on_show(&mut screens.onboarding, ctx).map(S::Onboarding),
        };
        task.map(Message::Screen)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let ctx = &self.ctx;
        let main = container(column![self.title_bar(), self.content()])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(style::surface);
        let mut layout = row![self.sidebar(), main];
        if ctx.assistant_open {
            layout = layout.push(
                container(mapped(
                    assistant::panel(&self.screens.assistant, ctx),
                    S::Assistant,
                ))
                .width(380)
                .height(Length::Fill)
                .style(style::ground),
            );
        }
        let mut layers = Stack::new().push(layout);
        if ctx.palette_open {
            layers = layers.push(modal(
                mapped(palette::view(&self.screens.palette, ctx), S::Palette),
                Message::ClosePalette,
                Some(96.0),
            ));
        }
        if let Some(pending) = &self.confirm {
            layers = layers.push(confirm::dialog(
                &pending.title,
                &pending.body,
                "Cancel",
                &pending.confirm_label,
                pending.danger,
                Message::ConfirmCancelled,
                Message::ConfirmAccepted,
            ));
        }
        if self.close_dialog {
            let running = ctx.running_count();
            layers = layers.push(confirm::dialog(
                "Stop servers and quit?",
                if running == 1 {
                    "1 server is still running. It will be stopped safely before Monolith closes."
                } else {
                    "Running servers will be stopped safely before Monolith closes."
                },
                "Keep running",
                "Stop and quit",
                false,
                Message::CloseCancelled,
                Message::CloseConfirmed,
            ));
        }
        if !self.toasts.is_empty() {
            layers = layers.push(toast::view(&self.toasts, Message::DismissToast));
        }
        layers.push(resize_grips(Message::Resize)).into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let ctx = &self.ctx;
        let brand = row![
            app_icon(28.0),
            text("Monolith").size(TEXT_LG).font(UI_SEMIBOLD)
        ]
        .spacing(SPACE_3)
        .align_y(Alignment::Center);

        let servers: Element<'_, Message> = if ctx.servers.is_empty() {
            container(text("No servers yet").size(TEXT_SM).style(style::faint))
                .padding([SPACE_2, SPACE_3])
                .into()
        } else {
            let list = ctx
                .servers
                .iter()
                .fold(column![].spacing(SPACE_1), |list, server| {
                    let selected = matches!(ctx.route, Route::Server(_))
                        && ctx.selected.as_deref() == Some(server.id.as_str());
                    let entry = row![
                        server_icon(
                            status_color(ctx.status(&server.id)),
                            ctx.status_text(&server.id)
                        ),
                        container(
                            text(server.name.as_str())
                                .size(TEXT_MD)
                                .wrapping(Wrapping::None)
                        )
                        .width(Length::Fill)
                        .clip(true),
                    ]
                    .spacing(SPACE_3)
                    .align_y(Alignment::Center);
                    list.push(
                        button(entry)
                            .width(Length::Fill)
                            .padding([8, 12])
                            .style(style::sidebar_item(selected))
                            .on_press(Message::SelectServer(server.id.clone())),
                    )
                });
            scrollable(list)
                .style(style::scroll)
                .height(Length::Fill)
                .into()
        };

        let footer = column![
            nav_button(
                "New server",
                Icon::Plus,
                matches!(ctx.route, Route::Create),
                Message::Goto(Route::Create)
            ),
            nav_button(
                "Settings",
                Icon::Settings,
                matches!(ctx.route, Route::Settings(_)),
                Message::Goto(Route::Settings(SettingsPage::General))
            ),
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
                Space::new().height(SPACE_3),
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

    fn title_bar(&self) -> Element<'_, Message> {
        let tool = |glyph: Icon, message: Message| {
            container(
                button(icon(glyph, 16.0))
                    .width(36)
                    .height(28)
                    .padding(6)
                    .style(style::icon_button)
                    .on_press(message),
            )
            .height(TITLE_BAR_HEIGHT)
            .center_y(TITLE_BAR_HEIGHT)
        };
        row![
            drag_region(
                Space::new()
                    .width(Length::Fill)
                    .height(Length::Fixed(TITLE_BAR_HEIGHT)),
                Message::Drag,
                Message::ToggleMaximize,
            ),
            tool(Icon::Search, Message::OpenPalette),
            tool(Icon::Sparkles, Message::ToggleAssistant),
            Space::new().width(SPACE_2),
            window_controls(
                self.maximized,
                Message::Minimize,
                Message::ToggleMaximize,
                Message::CloseRequested,
            ),
        ]
        .into()
    }

    fn content(&self) -> Element<'_, Message> {
        let ctx = &self.ctx;
        let body: Element<'_, Message> = match ctx.route {
            Route::Server(tab) if ctx.selected_server().is_some() => self.server_view(tab),
            Route::Server(_) | Route::Empty => mapped(
                onboarding::view(&self.screens.onboarding, ctx),
                S::Onboarding,
            ),
            Route::Create => mapped(create::view(&self.screens.create, ctx), S::Create),
            Route::Settings(page) => self.settings_view(page),
        };
        container(container(body).max_width(1100))
            .padding(Padding {
                top: SPACE_2,
                right: 40.0,
                bottom: SPACE_6,
                left: 40.0,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn server_view(&self, tab: Tab) -> Element<'_, Message> {
        let ctx = &self.ctx;
        let screens = &self.screens;
        let header = mapped(overview::header(&screens.overview, ctx), S::Overview);
        let tabs = Tab::ALL
            .iter()
            .fold(row![].spacing(SPACE_1), |bar, &entry| {
                bar.push(
                    button(text(entry.label()).size(TEXT_MD).font(UI_MEDIUM))
                        .padding([6, 12])
                        .style(style::tab(entry == tab))
                        .on_press(Message::Goto(Route::Server(entry))),
                )
            });
        let body = match tab {
            Tab::Overview => mapped(overview::view(&screens.overview, ctx), S::Overview),
            Tab::Console => mapped(console::view(&screens.console, ctx), S::Console),
            Tab::Files => mapped(files::view(&screens.files, ctx), S::Files),
            Tab::Players => mapped(players::view(&screens.players, ctx), S::Players),
            Tab::Content => mapped(content::view(&screens.content, ctx), S::Content),
            Tab::Backups => mapped(backups::view(&screens.backups, ctx), S::Backups),
            Tab::Analytics => mapped(analytics::view(&screens.analytics, ctx), S::Analytics),
            Tab::Settings => mapped(
                server_settings::view(&screens.server_settings, ctx),
                S::ServerSettings,
            ),
        };
        column![
            header,
            tabs,
            container(body).width(Length::Fill).height(Length::Fill)
        ]
        .spacing(SPACE_5)
        .into()
    }

    fn settings_view(&self, page: SettingsPage) -> Element<'_, Message> {
        let ctx = &self.ctx;
        let screens = &self.screens;
        let nav = SettingsPage::ALL
            .iter()
            .fold(column![].spacing(SPACE_1), |nav, &entry| {
                nav.push(
                    button(text(entry.label()).size(TEXT_MD))
                        .width(Length::Fill)
                        .padding([8, 12])
                        .style(style::sidebar_item(entry == page))
                        .on_press(Message::Goto(Route::Settings(entry))),
                )
            })
            .width(180);
        let body = match page {
            SettingsPage::General | SettingsPage::Updates => {
                mapped(general::view(&screens.general, ctx), S::General)
            }
            SettingsPage::Defaults => mapped(defaults::view(&screens.defaults, ctx), S::Defaults),
            SettingsPage::Sharing => mapped(sharing::view(&screens.sharing, ctx), S::Sharing),
            SettingsPage::Assistant => {
                mapped(assistant::view(&screens.assistant, ctx), S::Assistant)
            }
        };
        column![
            text("Settings").size(TEXT_2XL).font(UI_SEMIBOLD),
            row![
                nav,
                container(body)
                    .max_width(760)
                    .width(Length::Fill)
                    .height(Length::Fill)
            ]
            .spacing(SPACE_6)
            .height(Length::Fill)
        ]
        .spacing(SPACE_5)
        .into()
    }
}

fn nav_button<'a>(
    label: &'a str,
    glyph: Icon,
    selected: bool,
    message: Message,
) -> Element<'a, Message> {
    button(
        row![icon(glyph, 16.0), text(label).size(TEXT_MD).font(UI_MEDIUM)]
            .spacing(SPACE_2)
            .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([8, 12])
    .style(style::sidebar_item(selected))
    .on_press(message)
    .into()
}

fn mapped<'a, M: 'a>(element: Element<'a, M>, wrap: fn(M) -> S) -> Element<'a, Message> {
    element.map(move |message| Message::Screen(wrap(message)))
}

fn quit() -> Task<Message> {
    Task::future(async {
        lifecycle::shutdown().await;
        Message::Quit
    })
}

fn key_message(event: iced::Event, status: event::Status, id: window::Id) -> Option<Message> {
    shortcuts::from_event(event, status, id).map(Message::Shortcut)
}

fn core_events() -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(256, async |mut output| {
        use iced::futures::SinkExt;
        let mut receiver = events::subscribe();
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if output.send(Message::Core(event)).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}
