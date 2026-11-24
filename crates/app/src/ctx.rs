use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iced::Color;
use monolith_core::types::{AppPreferences, ServerRecord, ServerRuntimeStatus, ServerStats};
use monolith_theme::token::{AMBER, CORAL, INK_SECONDARY, MOSS, STONE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Files,
    Players,
    Content,
    Backups,
    Analytics,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 7] = [
        Tab::Overview,
        Tab::Files,
        Tab::Players,
        Tab::Content,
        Tab::Backups,
        Tab::Analytics,
        Tab::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Files => "Files",
            Tab::Players => "Players",
            Tab::Content => "Content",
            Tab::Backups => "Backups",
            Tab::Analytics => "Analytics",
            Tab::Settings => "Settings",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    General,
    Defaults,
    Sharing,
    Assistant,
    Updates,
    About,
    Templates,
}

impl SettingsPage {
    pub const ALL: [SettingsPage; 7] = [
        SettingsPage::General,
        SettingsPage::Defaults,
        SettingsPage::Templates,
        SettingsPage::Sharing,
        SettingsPage::Assistant,
        SettingsPage::Updates,
        SettingsPage::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsPage::General => "General",
            SettingsPage::Defaults => "Default plugins",
            SettingsPage::Sharing => "Sharing",
            SettingsPage::Assistant => "Assistant",
            SettingsPage::Updates => "Updates",
            SettingsPage::About => "About",
            SettingsPage::Templates => "Templates",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Server(Tab),
    Create,
    Settings(SettingsPage),
    Empty,
    Remote,
}

impl Route {
    pub fn parse(name: &str) -> Option<Route> {
        Some(match name {
            "overview" => Route::Server(Tab::Overview),
            "console" => Route::Server(Tab::Overview),
            "files" => Route::Server(Tab::Files),
            "players" => Route::Server(Tab::Players),
            "content" => Route::Server(Tab::Content),
            "backups" => Route::Server(Tab::Backups),
            "analytics" => Route::Server(Tab::Analytics),
            "server-settings" => Route::Server(Tab::Settings),
            "create" => Route::Create,
            "settings-general" => Route::Settings(SettingsPage::General),
            "settings-defaults" => Route::Settings(SettingsPage::Defaults),
            "settings-sharing" => Route::Settings(SettingsPage::Sharing),
            "settings-assistant" => Route::Settings(SettingsPage::Assistant),
            "settings-updates" => Route::Settings(SettingsPage::Updates),
            "settings-about" => Route::Settings(SettingsPage::About),
            "settings-templates" => Route::Settings(SettingsPage::Templates),
            "empty" => Route::Empty,
            "remote" => Route::Remote,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
    Info,
}

pub type ConfirmRun =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> + Send + Sync>;

#[derive(Clone)]
pub struct Confirm {
    pub title: String,
    pub body: String,
    pub confirm_label: String,
    pub busy_label: String,
    pub danger: bool,
    pub run: ConfirmRun,
}

impl Confirm {
    pub fn busy_label(mut self, label: impl Into<String>) -> Self {
        self.busy_label = label.into();
        self
    }

    pub fn new<F, Fut>(
        title: impl Into<String>,
        body: impl Into<String>,
        confirm_label: impl Into<String>,
        danger: bool,
        run: F,
    ) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, String>> + Send + 'static,
    {
        Self {
            title: title.into(),
            body: body.into(),
            confirm_label: confirm_label.into(),
            busy_label: "Working".into(),
            danger,
            run: Arc::new(move || Box::pin(run())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    Hide,
    Show,
    Quit,
}

pub enum Action {
    Navigate(Route),
    Toast(ToastKind, String),
    Confirm(Confirm),
    SelectServer(String),
    RefreshServers,
    RefreshPreferences,
    OpenPalette,
    ClosePalette,
    ToggleAssistant,
    Window(WindowAction),
    SetCloseToTray(bool),
    RestartRequired(String),
}

pub struct Ctx {
    pub servers: Vec<ServerRecord>,
    pub selected: Option<String>,
    pub stats: HashMap<String, ServerStats>,
    pub statuses: HashMap<String, ServerRuntimeStatus>,
    pub preferences: AppPreferences,
    pub route: Route,
    pub palette_open: bool,
    pub assistant_open: bool,
    pub loaded: bool,
    pub close_to_tray: bool,
    pub restart_pending: HashSet<String>,
    outbox: RefCell<Vec<Action>>,
}

impl Ctx {
    pub fn new(route: Route) -> Self {
        Self {
            servers: Vec::new(),
            selected: None,
            stats: HashMap::new(),
            statuses: HashMap::new(),
            preferences: AppPreferences::default(),
            route,
            palette_open: false,
            assistant_open: false,
            loaded: false,
            close_to_tray: false,
            restart_pending: HashSet::new(),
            outbox: RefCell::new(Vec::new()),
        }
    }

    pub fn server(&self, id: &str) -> Option<&ServerRecord> {
        self.servers.iter().find(|server| server.id == id)
    }

    pub fn selected_server(&self) -> Option<&ServerRecord> {
        self.selected.as_deref().and_then(|id| self.server(id))
    }

    pub fn status(&self, id: &str) -> ServerRuntimeStatus {
        self.statuses.get(id).copied().unwrap_or_else(|| {
            self.server(id)
                .map(|server| server.status)
                .unwrap_or_default()
        })
    }

    pub fn stats(&self, id: &str) -> Option<&ServerStats> {
        self.stats.get(id)
    }

    pub fn status_text(&self, id: &str) -> String {
        match self.status(id) {
            ServerRuntimeStatus::Online => match self.stats(id) {
                Some(stats) => format!(
                    "Online · {}/{} players",
                    stats.player_count, stats.max_players
                ),
                None => "Online".into(),
            },
            ServerRuntimeStatus::Starting => "Starting".into(),
            ServerRuntimeStatus::Stopping => "Stopping".into(),
            ServerRuntimeStatus::Idle => "Idle".into(),
            ServerRuntimeStatus::Offline => "Offline".into(),
            ServerRuntimeStatus::Crashed => "Crashed".into(),
        }
    }

    pub fn running_count(&self) -> usize {
        self.servers
            .iter()
            .filter(|server| {
                matches!(
                    self.status(&server.id),
                    ServerRuntimeStatus::Online
                        | ServerRuntimeStatus::Starting
                        | ServerRuntimeStatus::Stopping
                )
            })
            .count()
    }

    pub fn request(&self, action: Action) {
        self.outbox.borrow_mut().push(action);
    }

    pub fn take_actions(&self) -> Vec<Action> {
        std::mem::take(&mut *self.outbox.borrow_mut())
    }

    pub fn navigate(&self, route: Route) {
        self.request(Action::Navigate(route));
    }

    pub fn toast(&self, kind: ToastKind, text: impl Into<String>) {
        self.request(Action::Toast(kind, text.into()));
    }

    pub fn success(&self, text: impl Into<String>) {
        self.toast(ToastKind::Success, text);
    }

    pub fn error(&self, text: impl Into<String>) {
        self.toast(ToastKind::Error, text);
    }

    pub fn info(&self, text: impl Into<String>) {
        self.toast(ToastKind::Info, text);
    }

    pub fn confirm(&self, confirm: Confirm) {
        self.request(Action::Confirm(confirm));
    }

    pub fn select_server(&self, id: impl Into<String>) {
        self.request(Action::SelectServer(id.into()));
    }

    pub fn refresh_servers(&self) {
        self.request(Action::RefreshServers);
    }

    pub fn refresh_preferences(&self) {
        self.request(Action::RefreshPreferences);
    }

    pub fn close_palette(&self) {
        self.request(Action::ClosePalette);
    }

    pub fn toggle_assistant(&self) {
        self.request(Action::ToggleAssistant);
    }

    pub fn window(&self, action: WindowAction) {
        self.request(Action::Window(action));
    }

    pub fn set_close_to_tray(&self, enabled: bool) {
        self.request(Action::SetCloseToTray(enabled));
    }

    pub fn changed_while_running(&self, id: &str) {
        if matches!(
            self.status(id),
            ServerRuntimeStatus::Online | ServerRuntimeStatus::Starting
        ) {
            self.request(Action::RestartRequired(id.to_string()));
        }
    }

    pub fn needs_restart(&self, id: &str) -> bool {
        self.restart_pending.contains(id)
    }
}

pub fn status_color(status: ServerRuntimeStatus) -> Color {
    match status {
        ServerRuntimeStatus::Online => MOSS,
        ServerRuntimeStatus::Starting | ServerRuntimeStatus::Stopping => AMBER,
        ServerRuntimeStatus::Idle => INK_SECONDARY,
        ServerRuntimeStatus::Offline => STONE,
        ServerRuntimeStatus::Crashed => CORAL,
    }
}
