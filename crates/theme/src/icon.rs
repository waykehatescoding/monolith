use iced::widget::svg::{self, Svg};
use iced::{Color, Length, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Server,
    Play,
    Stop,
    Restart,
    Plus,
    Settings,
    Folder,
    File,
    Terminal,
    Users,
    Package,
    Archive,
    Chart,
    Globe,
    Share,
    Copy,
    Check,
    Close,
    Warning,
    Error,
    Info,
    Search,
    ChevronDown,
    ChevronUp,
    ChevronLeft,
    ChevronRight,
    Download,
    Upload,
    Trash,
    Edit,
    More,
    Minimize,
    Maximize,
    Sparkles,
    Clock,
    Cpu,
    Memory,
    Wifi,
    Link,
    ExternalLink,
    Filter,
    Keyboard,
    Bell,
    Power,
}

impl Icon {
    pub const ALL: [Icon; 44] = [
        Icon::Server,
        Icon::Play,
        Icon::Stop,
        Icon::Restart,
        Icon::Plus,
        Icon::Settings,
        Icon::Folder,
        Icon::File,
        Icon::Terminal,
        Icon::Users,
        Icon::Package,
        Icon::Archive,
        Icon::Chart,
        Icon::Globe,
        Icon::Share,
        Icon::Copy,
        Icon::Check,
        Icon::Close,
        Icon::Warning,
        Icon::Error,
        Icon::Info,
        Icon::Search,
        Icon::ChevronDown,
        Icon::ChevronUp,
        Icon::ChevronLeft,
        Icon::ChevronRight,
        Icon::Download,
        Icon::Upload,
        Icon::Trash,
        Icon::Edit,
        Icon::More,
        Icon::Minimize,
        Icon::Maximize,
        Icon::Sparkles,
        Icon::Clock,
        Icon::Cpu,
        Icon::Memory,
        Icon::Wifi,
        Icon::Link,
        Icon::ExternalLink,
        Icon::Filter,
        Icon::Keyboard,
        Icon::Bell,
        Icon::Power,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Icon::Server => "server",
            Icon::Play => "play",
            Icon::Stop => "square",
            Icon::Restart => "rotate-ccw",
            Icon::Plus => "plus",
            Icon::Settings => "settings",
            Icon::Folder => "folder",
            Icon::File => "file",
            Icon::Terminal => "terminal",
            Icon::Users => "users",
            Icon::Package => "package",
            Icon::Archive => "archive",
            Icon::Chart => "chart-column",
            Icon::Globe => "globe",
            Icon::Share => "share-2",
            Icon::Copy => "copy",
            Icon::Check => "check",
            Icon::Close => "x",
            Icon::Warning => "triangle-alert",
            Icon::Error => "circle-alert",
            Icon::Info => "info",
            Icon::Search => "search",
            Icon::ChevronDown => "chevron-down",
            Icon::ChevronUp => "chevron-up",
            Icon::ChevronLeft => "chevron-left",
            Icon::ChevronRight => "chevron-right",
            Icon::Download => "download",
            Icon::Upload => "upload",
            Icon::Trash => "trash-2",
            Icon::Edit => "pencil",
            Icon::More => "ellipsis",
            Icon::Minimize => "minus",
            Icon::Maximize => "maximize",
            Icon::Sparkles => "sparkles",
            Icon::Clock => "clock",
            Icon::Cpu => "cpu",
            Icon::Memory => "memory-stick",
            Icon::Wifi => "wifi",
            Icon::Link => "link",
            Icon::ExternalLink => "external-link",
            Icon::Filter => "funnel",
            Icon::Keyboard => "keyboard",
            Icon::Bell => "bell",
            Icon::Power => "power",
        }
    }

    fn source(self) -> &'static [u8] {
        match self {
            Icon::Server => include_bytes!("../../../assets/icons/server.svg"),
            Icon::Play => include_bytes!("../../../assets/icons/play.svg"),
            Icon::Stop => include_bytes!("../../../assets/icons/square.svg"),
            Icon::Restart => include_bytes!("../../../assets/icons/rotate-ccw.svg"),
            Icon::Plus => include_bytes!("../../../assets/icons/plus.svg"),
            Icon::Settings => include_bytes!("../../../assets/icons/settings.svg"),
            Icon::Folder => include_bytes!("../../../assets/icons/folder.svg"),
            Icon::File => include_bytes!("../../../assets/icons/file.svg"),
            Icon::Terminal => include_bytes!("../../../assets/icons/terminal.svg"),
            Icon::Users => include_bytes!("../../../assets/icons/users.svg"),
            Icon::Package => include_bytes!("../../../assets/icons/package.svg"),
            Icon::Archive => include_bytes!("../../../assets/icons/archive.svg"),
            Icon::Chart => include_bytes!("../../../assets/icons/chart-column.svg"),
            Icon::Globe => include_bytes!("../../../assets/icons/globe.svg"),
            Icon::Share => include_bytes!("../../../assets/icons/share-2.svg"),
            Icon::Copy => include_bytes!("../../../assets/icons/copy.svg"),
            Icon::Check => include_bytes!("../../../assets/icons/check.svg"),
            Icon::Close => include_bytes!("../../../assets/icons/x.svg"),
            Icon::Warning => include_bytes!("../../../assets/icons/triangle-alert.svg"),
            Icon::Error => include_bytes!("../../../assets/icons/circle-alert.svg"),
            Icon::Info => include_bytes!("../../../assets/icons/info.svg"),
            Icon::Search => include_bytes!("../../../assets/icons/search.svg"),
            Icon::ChevronDown => include_bytes!("../../../assets/icons/chevron-down.svg"),
            Icon::ChevronUp => include_bytes!("../../../assets/icons/chevron-up.svg"),
            Icon::ChevronLeft => include_bytes!("../../../assets/icons/chevron-left.svg"),
            Icon::ChevronRight => include_bytes!("../../../assets/icons/chevron-right.svg"),
            Icon::Download => include_bytes!("../../../assets/icons/download.svg"),
            Icon::Upload => include_bytes!("../../../assets/icons/upload.svg"),
            Icon::Trash => include_bytes!("../../../assets/icons/trash-2.svg"),
            Icon::Edit => include_bytes!("../../../assets/icons/pencil.svg"),
            Icon::More => include_bytes!("../../../assets/icons/ellipsis.svg"),
            Icon::Minimize => include_bytes!("../../../assets/icons/minus.svg"),
            Icon::Maximize => include_bytes!("../../../assets/icons/maximize.svg"),
            Icon::Sparkles => include_bytes!("../../../assets/icons/sparkles.svg"),
            Icon::Clock => include_bytes!("../../../assets/icons/clock.svg"),
            Icon::Cpu => include_bytes!("../../../assets/icons/cpu.svg"),
            Icon::Memory => include_bytes!("../../../assets/icons/memory-stick.svg"),
            Icon::Wifi => include_bytes!("../../../assets/icons/wifi.svg"),
            Icon::Link => include_bytes!("../../../assets/icons/link.svg"),
            Icon::ExternalLink => include_bytes!("../../../assets/icons/external-link.svg"),
            Icon::Filter => include_bytes!("../../../assets/icons/funnel.svg"),
            Icon::Keyboard => include_bytes!("../../../assets/icons/keyboard.svg"),
            Icon::Bell => include_bytes!("../../../assets/icons/bell.svg"),
            Icon::Power => include_bytes!("../../../assets/icons/power.svg"),
        }
    }
}

pub fn icon<'a>(name: Icon, size: f32) -> Svg<'a, Theme> {
    Svg::new(svg::Handle::from_memory(name.source()))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(|theme: &Theme, _| svg::Style {
            color: Some(theme.palette().text),
        })
}

pub fn icon_colored<'a>(name: Icon, size: f32, color: Color) -> Svg<'a, Theme> {
    Svg::new(svg::Handle::from_memory(name.source()))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_: &Theme, _| svg::Style { color: Some(color) })
}

const APP_ICON: &[u8] = include_bytes!("../../../assets/brand/icon.svg");
const APP_MARK: &[u8] = include_bytes!("../../../assets/brand/mark.svg");

pub fn app_icon<'a>(size: f32) -> Svg<'a, Theme> {
    Svg::new(svg::Handle::from_memory(APP_ICON))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
}

pub fn app_mark<'a>(size: f32) -> Svg<'a, Theme> {
    Svg::new(svg::Handle::from_memory(APP_MARK))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
}
