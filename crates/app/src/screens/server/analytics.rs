use std::time::Duration;

use monolith_theme::token::BUTTON_MD;
use monolith_theme::widget::{sized, sized_icon};
use chrono::{DateTime, Local, Utc};
use iced::widget::canvas::{self, Canvas, Path, Stroke};
use iced::widget::text::Alignment;
use iced::widget::{column, container, row, scrollable, space, text};
use iced::{alignment, mouse, Color, Element, Length, Pixels, Point, Rectangle, Renderer, Size, Subscription, Task, Theme};
use monolith_core::events::Event;
use monolith_core::server_stats::get_analytics_data;
use monolith_core::server_store::update_server_settings;
use monolith_core::types::{AnalyticsDataResult, ServerRuntimeStatus, ServerSettingsUpdate};
use monolith_theme::{icon, Icon, UI, UI_MEDIUM, UI_SEMIBOLD};
use monolith_theme::style;
use monolith_theme::token::*;
use serde::Deserialize;

use crate::ctx::Ctx;

const REFRESH_EVERY: Duration = Duration::from_secs(30);
const TOP_PLAYERS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Range {
    Hour,
    #[default]
    Day,
    Week,
}

impl Range {
    const ALL: [Range; 3] = [Range::Hour, Range::Day, Range::Week];

    fn label(self) -> &'static str {
        match self {
            Range::Hour => "1 hour",
            Range::Day => "24 hours",
            Range::Week => "7 days",
        }
    }

    fn seconds(self) -> f64 {
        match self {
            Range::Hour => 3600.0,
            Range::Day => 86_400.0,
            Range::Week => 604_800.0,
        }
    }

    fn time_format(self) -> &'static str {
        match self {
            Range::Hour | Range::Day => "%H:%M",
            Range::Week => "%d %b %H:%M",
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Report {
    overview: Overview,
    players: Vec<PlayerRow>,
    tps: Vec<TpsSample>,
    memory: Vec<MemorySample>,
    timeline: Vec<PlayerSample>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Overview {
    current_online: u32,
    peak_online: u32,
    unique_players: u32,
    total_joins: u64,
    total_chat_messages: u64,
    total_commands_executed: u64,
    total_deaths: u64,
    current_tps: Option<f64>,
    #[serde(rename = "memoryUsedMB")]
    memory_used_mb: f64,
    #[serde(rename = "memoryMaxMB")]
    memory_max_mb: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct PlayerRow {
    name: String,
    join_count: u64,
    total_play_time_seconds: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TpsSample {
    timestamp: String,
    tps: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MemorySample {
    timestamp: String,
    #[serde(rename = "usedMB")]
    used_mb: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PlayerSample {
    timestamp: String,
    players: f64,
}

#[derive(Debug, Default)]
enum Load {
    #[default]
    Loading,
    NoData,
    Ready,
    Failed(String),
}

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    report: Option<Report>,
    load: Load,
    range: Range,
    enabling: bool,
    refreshing: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Range(Range),
    Reload,
    Tick,
    Loaded(String, AnalyticsDataResult),
    Enable,
    Enabled(Result<(), String>),
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Range(range) => {
            state.range = range;
            Task::none()
        }
        Message::Reload => {
            state.refreshing = true;
            fetch(state, ctx)
        }
        Message::Tick => {
            if state.server_id.is_some() {
                fetch(state, ctx)
            } else {
                Task::none()
            }
        }
        Message::Loaded(id, result) => {
            if state.server_id.as_deref() != Some(id.as_str()) {
                return Task::none();
            }
            state.refreshing = false;
            match (result.success, result.data) {
                (true, Some(value)) => match serde_json::from_value::<Report>(value) {
                    Ok(report) => {
                        state.report = Some(report);
                        state.load = Load::Ready;
                    }
                    Err(_) if state.report.is_some() => {}
                    Err(_) => state.load = Load::Failed("The analytics file is not in the expected format.".into()),
                },
                _ => match result.error.as_deref() {
                    Some("no-data") => {
                        state.load = if state.report.is_some() { Load::Ready } else { Load::NoData };
                    }
                    _ if state.report.is_some() => {}
                    other => {
                        state.load = Load::Failed(other.unwrap_or("The analytics file could not be read.").to_string());
                    }
                },
            }
            Task::none()
        }
        Message::Enable => {
            let Some(id) = ctx.selected_server().map(|server| server.id.clone()) else {
                return Task::none();
            };
            state.enabling = true;
            let settings = ServerSettingsUpdate {
                analytics_enabled: Some(true),
                ..Default::default()
            };
            Task::perform(update_server_settings(id, settings), |result| {
                Message::Enabled(if result.success {
                    Ok(())
                } else {
                    Err(result.error.unwrap_or_else(|| "Something went wrong.".into()))
                })
            })
        }
        Message::Enabled(result) => {
            state.enabling = false;
            match result {
                Ok(()) => {
                    ctx.success("Analytics enabled. Restart the server to start recording.");
                    ctx.refresh_servers();
                }
                Err(error) => ctx.error(format!("Could not enable analytics. {error}")),
            }
            Task::none()
        }
    }
}

fn fetch(state: &State, ctx: &Ctx) -> Task<Message> {
    let Some(id) = state.server_id.clone() else {
        return Task::none();
    };
    if !ctx.server(&id).is_some_and(|server| server.analytics_enabled == Some(true)) {
        return Task::none();
    }
    Task::perform(get_analytics_data(id.clone()), move |result| Message::Loaded(id.clone(), result))
}

pub fn on_show(state: &mut State, ctx: &Ctx) -> Task<Message> {
    let id = ctx.selected_server().map(|server| server.id.clone());
    if state.server_id != id {
        state.server_id = id;
        state.report = None;
        state.load = Load::Loading;
        state.enabling = false;
        state.refreshing = false;
    }
    fetch(state, ctx)
}

pub fn on_event(state: &mut State, event: &Event, ctx: &Ctx) -> Task<Message> {
    match event {
        Event::ServerStatus(update) if state.server_id.as_deref() == Some(update.server_id.as_str()) => fetch(state, ctx),
        _ => Task::none(),
    }
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    iced::time::every(REFRESH_EVERY).map(|_| Message::Tick)
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let Some(server) = ctx.selected_server() else {
        return notice("No server selected", "Pick a server from the sidebar to see its analytics.", None);
    };

    if server.framework != "Paper" && server.framework != "Purpur" {
        return notice(
            "Analytics needs Paper or Purpur",
            format!(
                "{} servers cannot run the Monolith Analytics plugin. Create a Paper or Purpur server to see TPS, memory and player history.",
                server.framework
            ),
            None,
        );
    }

    if server.analytics_enabled != Some(true) {
        let label = if state.enabling { "Enabling" } else { "Enable analytics" };
        let enable = sized(text(label).size(TEXT_MD), BUTTON_MD)
            .style(style::primary)
            .on_press_maybe((!state.enabling).then_some(Message::Enable))
            .into();
        return notice(
            "Analytics is off",
            "Turn it on to track TPS, memory use and player activity over time. The plugin is added to this server and starts recording after its next restart.",
            Some(enable),
        );
    }

    match (&state.load, &state.report) {
        (Load::Failed(error), _) => {
            let retry = sized(text("Try again").size(TEXT_MD), BUTTON_MD)
                .style(style::secondary)
                .on_press(Message::Reload)
                .into();
            notice("Could not read analytics data", error.clone(), Some(retry))
        }
        (Load::Loading, None) => container(text("Loading analytics...").size(TEXT_MD).style(style::muted))
            .center(Length::Fill)
            .into(),
        (Load::NoData, _) | (_, None) => {
            let running = ctx.status(&server.id) == ServerRuntimeStatus::Online;
            let body = if running {
                "The plugin records a sample about every minute. Charts appear here once the first one is in."
            } else {
                "Start the server and let it run for a minute. Charts appear here once the plugin has recorded data."
            };
            notice("Waiting for the first samples", body, None)
        }
        (_, Some(report)) => report_view(state, report),
    }
}

fn notice<'a>(title: &'a str, body: impl text::IntoFragment<'a>, action: Option<Element<'a, Message>>) -> Element<'a, Message> {
    let mut content = column![
        text(title).size(TEXT_XL).font(UI_SEMIBOLD),
        text(body).size(TEXT_MD).style(style::muted).align_x(alignment::Horizontal::Center),
    ]
    .spacing(SPACE_2)
    .align_x(alignment::Horizontal::Center)
    .max_width(440);
    if let Some(action) = action {
        content = content.push(space().height(SPACE_3)).push(action);
    }
    container(content).center(Length::Fill).into()
}

fn report_view<'a>(state: &'a State, report: &'a Report) -> Element<'a, Message> {
    let range = state.range;
    let now = Utc::now().timestamp() as f64;
    let cutoff = now - range.seconds();
    let windowed = |points: Vec<(f64, f64)>| -> Vec<(f64, f64)> { points.into_iter().filter(|(t, _)| *t >= cutoff).collect() };

    let tps = windowed(series(report.tps.iter().map(|s| (&s.timestamp, s.tps))));
    let memory = windowed(series(report.memory.iter().map(|s| (&s.timestamp, s.used_mb))));
    let players = windowed(series(report.timeline.iter().map(|s| (&s.timestamp, s.players))));

    let overview = &report.overview;
    let tps_now = overview.current_tps.or_else(|| tps.last().map(|p| p.1));
    let memory_max = if overview.memory_max_mb > 0.0 {
        overview.memory_max_mb
    } else {
        memory.iter().map(|p| p.1).fold(0.0, f64::max) * 1.1
    };
    let player_max = players.iter().map(|p| p.1).fold(overview.peak_online as f64, f64::max).max(4.0);

    let header = row![
        space().width(Length::Fill),
        text(if state.refreshing { "Refreshing..." } else { "" }).size(TEXT_SM).style(style::muted),
        row(Range::ALL.map(|option| {
            sized(text(option.label()).size(TEXT_MD), BUTTON_MD)
                .style(style::tab(state.range == option))
                .on_press(Message::Range(option))
                .into()
        }))
        .spacing(SPACE_1),
        sized_icon(icon(Icon::Restart, 16.0), BUTTON_MD)
            .style(style::icon_button)
            .on_press_maybe((!state.refreshing).then_some(Message::Reload)),
    ]
    .spacing(SPACE_2)
    .align_y(alignment::Vertical::Center);

    let charts = column![
        chart_panel(
            "Server speed (TPS)",
            tps_now.map(|v| format!("{v:.1} of 20")).unwrap_or_else(|| "-".into()),
            Chart::new(tps, 20.0, MOSS, range, cutoff, now, format_tps),
        ),
        chart_panel(
            "Memory",
            format!("{} of {}", format_mb(overview.memory_used_mb), format_mb(memory_max)),
            Chart::new(memory, memory_max, INK_SECONDARY, range, cutoff, now, format_mb),
        ),
        chart_panel(
            "Players online",
            format!("{} now, peak {}", overview.current_online, overview.peak_online),
            Chart::new(players, player_max, MOSS, range, cutoff, now, format_count),
        ),
    ]
    .spacing(SPACE_6);

    let totals = text(format!(
        "{} joins from {} players, {} chat messages, {} commands, {} deaths",
        overview.total_joins, overview.unique_players, overview.total_chat_messages, overview.total_commands_executed, overview.total_deaths
    ))
    .size(TEXT_SM)
    .style(style::muted);

    let content = column![header, charts, totals, top_players(&report.players)]
        .spacing(SPACE_6)
        .padding(iced::Padding::ZERO.bottom(SPACE_5));

    scrollable(content)
        .style(style::scroll)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn series<'a>(samples: impl Iterator<Item = (&'a String, f64)>) -> Vec<(f64, f64)> {
    samples
        .filter_map(|(stamp, value)| {
            let time = DateTime::parse_from_rfc3339(stamp).ok()?;
            Some((time.timestamp() as f64, value))
        })
        .collect()
}

fn chart_panel<'a>(title: &'a str, value: String, chart: Chart) -> Element<'a, Message> {
    let body: Element<'a, Message> = if chart.points.is_empty() {
        container(text(chart.empty_message()).size(TEXT_SM).style(style::muted))
            .width(Length::Fill)
            .height(CHART_HEIGHT)
            .center(Length::Fill)
            .into()
    } else {
        Canvas::new(chart).width(Length::Fill).height(CHART_HEIGHT).into()
    };
    column![
        row![
            text(title).size(TEXT_MD).font(UI_MEDIUM),
            space().width(Length::Fill),
            text(value).size(TEXT_MD).style(style::muted),
        ],
        body,
    ]
    .spacing(SPACE_3)
    .into()
}

fn top_players<'a>(players: &'a [PlayerRow]) -> Element<'a, Message> {
    let mut ranked: Vec<&PlayerRow> = players.iter().collect();
    ranked.sort_by(|a, b| b.total_play_time_seconds.total_cmp(&a.total_play_time_seconds));

    let mut list = column![text("Most playtime").size(TEXT_MD).font(UI_MEDIUM)].spacing(SPACE_3);
    if ranked.is_empty() {
        list = list.push(text("Nobody has joined yet. Share the server address to get started.").size(TEXT_SM).style(style::muted));
    }
    for player in ranked.into_iter().take(TOP_PLAYERS) {
        list = list.push(
            row![
                text(&player.name).size(TEXT_MD),
                space().width(Length::Fill),
                text(format!("{} {}", player.join_count, if player.join_count == 1 { "join" } else { "joins" })).size(TEXT_SM).style(style::muted),
                text(format_playtime(player.total_play_time_seconds))
                    .size(TEXT_MD)
                    .width(72)
                    .align_x(alignment::Horizontal::Right),
            ]
            .spacing(SPACE_4)
            .align_y(alignment::Vertical::Center),
        );
    }
    list.into()
}

fn format_playtime(seconds: f64) -> String {
    let minutes = (seconds / 60.0) as u64;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h {m}m"),
    }
}

fn format_tps(value: f64) -> String {
    format!("{value:.1}")
}

fn format_count(value: f64) -> String {
    format!("{}", value.round() as u64)
}

fn format_mb(value: f64) -> String {
    if value >= 1024.0 {
        format!("{:.1} GB", value / 1024.0)
    } else {
        format!("{value:.0} MB")
    }
}

const CHART_HEIGHT: f32 = 150.0;
const PLOT_LEFT: f32 = 52.0;
const PLOT_TOP: f32 = 8.0;
const PLOT_BOTTOM: f32 = 22.0;

struct Chart {
    points: Vec<(f64, f64)>,
    y_max: f64,
    color: Color,
    range: Range,
    x_start: f64,
    x_end: f64,
    format: fn(f64) -> String,
}

impl Chart {
    fn new(points: Vec<(f64, f64)>, y_max: f64, color: Color, range: Range, x_start: f64, x_end: f64, format: fn(f64) -> String) -> Self {
        let peak = points.iter().map(|p| p.1).fold(0.0, f64::max);
        Self {
            points,
            y_max: y_max.max(peak).max(1.0),
            color,
            range,
            x_start,
            x_end,
            format,
        }
    }

    fn plot(&self, bounds: Rectangle) -> Rectangle {
        Rectangle {
            x: PLOT_LEFT,
            y: PLOT_TOP,
            width: (bounds.width - PLOT_LEFT - SPACE_2).max(1.0),
            height: (bounds.height - PLOT_TOP - PLOT_BOTTOM).max(1.0),
        }
    }

    fn project(&self, plot: Rectangle, point: (f64, f64)) -> Point {
        let x = ((point.0 - self.x_start) / (self.x_end - self.x_start)).clamp(0.0, 1.0);
        let y = (point.1 / self.y_max).clamp(0.0, 1.0);
        Point::new(plot.x + plot.width * x as f32, plot.y + plot.height * (1.0 - y as f32))
    }

    fn empty_message(&self) -> &'static str {
        match self.range {
            Range::Week => "No samples in the last 7 days. The plugin records while the server is running.",
            _ => "No samples in this range. Try a longer range.",
        }
    }

    fn clock(&self, seconds: f64) -> String {
        DateTime::from_timestamp(seconds as i64, 0)
            .map(|time| time.with_timezone(&Local).format(self.range.time_format()).to_string())
            .unwrap_or_default()
    }
}

fn label(content: String, position: Point, align: Alignment, color: Color) -> canvas::Text {
    canvas::Text {
        content,
        position,
        color,
        size: Pixels(TEXT_XS),
        font: UI,
        align_x: align,
        align_y: alignment::Vertical::Center,
        ..Default::default()
    }
}

impl canvas::Program<Message> for Chart {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, cursor: mouse::Cursor) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let plot = self.plot(bounds);

        for step in 0..=2 {
            let fraction = step as f32 / 2.0;
            let y = plot.y + plot.height * (1.0 - fraction);
            frame.stroke(
                &Path::line(Point::new(plot.x, y), Point::new(plot.x + plot.width, y)),
                Stroke::default().with_color(HAIRLINE).with_width(1.0),
            );
            frame.fill_text(label(
                (self.format)(self.y_max * fraction as f64),
                Point::new(plot.x - SPACE_2, y),
                Alignment::Right,
                INK_TERTIARY,
            ));
        }

        let (Some(first), Some(last)) = (self.points.first(), self.points.last()) else {
            return vec![frame.into_geometry()];
        };
        let baseline = plot.y + plot.height;
        let start = self.project(plot, *first);
        let end = self.project(plot, *last);
        let projected: Vec<Point> = self.points.iter().map(|p| self.project(plot, *p)).collect();

        let line = Path::new(|path| {
            path.move_to(start);
            projected.iter().skip(1).for_each(|p| path.line_to(*p));
        });
        let area = Path::new(|path| {
            path.move_to(Point::new(start.x, baseline));
            projected.iter().for_each(|p| path.line_to(*p));
            path.line_to(Point::new(end.x, baseline));
            path.close();
        });
        frame.fill(&area, self.color.scale_alpha(0.1));
        frame.stroke(
            &line,
            Stroke::default()
                .with_color(self.color)
                .with_width(1.75)
                .with_line_join(canvas::LineJoin::Round),
        );

        if projected.len() == 1 {
            frame.fill(&Path::circle(start, 3.0), self.color);
        }

        let label_y = baseline + PLOT_BOTTOM / 2.0 + 2.0;
        frame.fill_text(label(self.clock(self.x_start), Point::new(plot.x, label_y), Alignment::Left, INK_TERTIARY));
        frame.fill_text(label(self.clock(self.x_end), Point::new(plot.x + plot.width, label_y), Alignment::Right, INK_TERTIARY));

        if let Some(position) = cursor.position_in(bounds) {
            let nearest = projected
                .iter()
                .zip(&self.points)
                .min_by(|a, b| (a.0.x - position.x).abs().total_cmp(&(b.0.x - position.x).abs()));
            if let Some((point, sample)) = nearest {
                frame.stroke(
                    &Path::line(Point::new(point.x, plot.y), Point::new(point.x, baseline)),
                    Stroke::default().with_color(HAIRLINE_STRONG).with_width(1.0),
                );
                frame.fill(&Path::circle(*point, 4.0), self.color);
                frame.stroke(&Path::circle(*point, 4.0), Stroke::default().with_color(LAYER_2).with_width(2.0));

                let size = Size::new(128.0, 40.0);
                let x = if point.x + size.width + SPACE_3 > bounds.width {
                    point.x - size.width - SPACE_3
                } else {
                    point.x + SPACE_3
                };
                let y = (point.y - size.height / 2.0).clamp(0.0, (baseline - size.height).max(0.0));
                frame.fill(&Path::rounded_rectangle(Point::new(x, y), size, RADIUS_CONTROL.into()), LAYER_3);
                frame.fill_text(label((self.format)(sample.1), Point::new(x + SPACE_3, y + 13.0), Alignment::Left, INK));
                frame.fill_text(label(self.clock(sample.0), Point::new(x + SPACE_3, y + 29.0), Alignment::Left, INK_SECONDARY));
            }
        }

        vec![frame.into_geometry()]
    }
}
