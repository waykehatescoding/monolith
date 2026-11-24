use std::time::{Duration, Instant};

use iced::widget::{column, row, text};
use iced::{clipboard, Alignment, Element, Length, Subscription, Task};
use monolith_core::events::Event;
use monolith_core::server_runner::send_command;
use monolith_core::spark::{self, HEALTH_COMMAND, PROFILE_COMMAND};
use monolith_core::types::{ServerRecord, ServerRuntimeStatus, SuccessResult};
use monolith_theme::style;
use monolith_theme::token::{BUTTON_MD, CORAL, SPACE_2, SPACE_3, TEXT_MD, TEXT_SM};
use monolith_theme::widget::sized;
use monolith_theme::{MONO, UI_MEDIUM};

use super::settings::section;
use crate::ctx::Ctx;
use crate::ui::busy_button;

const PROFILE_SECONDS: u64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    Profile,
    Health,
}

impl Job {
    fn command(self) -> &'static str {
        match self {
            Job::Profile => PROFILE_COMMAND,
            Job::Health => HEALTH_COMMAND,
        }
    }

    fn give_up_after(self) -> Duration {
        match self {
            Job::Profile => Duration::from_secs(PROFILE_SECONDS + 60),
            Job::Health => Duration::from_secs(45),
        }
    }
}

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    installed: Option<bool>,
    installing: bool,
    running: Option<(Job, Instant)>,
    report: Option<(Job, String)>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Checked(String, bool),
    Install,
    Installed(Result<(), String>),
    Run(Job),
    Sent(SuccessResult),
    Tick,
    Open,
    Copy,
}

pub fn on_show(state: &mut State, server: &ServerRecord) -> Task<Message> {
    if state.server_id.as_deref() != Some(server.id.as_str()) {
        *state = State {
            server_id: Some(server.id.clone()),
            ..State::default()
        };
    }
    if !spark::supports(&server.framework) {
        return Task::none();
    }
    let id = server.id.clone();
    Task::perform(
        spark::is_installed(server.server_path.clone()),
        move |found| Message::Checked(id.clone(), found),
    )
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    let Some(server) = ctx.selected_server() else {
        return Task::none();
    };
    match message {
        Message::Checked(id, found) => {
            if state.server_id.as_ref() == Some(&id) {
                state.installed = Some(found);
            }
        }
        Message::Install => {
            state.installing = true;
            state.error = None;
            return Task::perform(spark::install(server.clone()), Message::Installed);
        }
        Message::Installed(result) => {
            state.installing = false;
            match result {
                Ok(()) => {
                    state.installed = Some(true);
                    ctx.success("spark installed");
                    ctx.changed_while_running(&server.id);
                }
                Err(error) => state.error = Some(error),
            }
        }
        Message::Run(job) => {
            state.running = Some((job, Instant::now()));
            state.report = None;
            state.error = None;
            return Task::perform(
                send_command(server.id.clone(), job.command().to_string()),
                Message::Sent,
            );
        }
        Message::Sent(result) => {
            if !result.success {
                state.running = None;
                state.error = result.error;
            }
        }
        Message::Tick => {
            if let Some((job, started)) = state.running {
                if started.elapsed() > job.give_up_after() {
                    state.running = None;
                    state.error = Some(
                        "spark did not post a report link. Check the console for what it said."
                            .into(),
                    );
                }
            }
        }
        Message::Open => {
            if let Some((_, url)) = &state.report {
                if open::that_detached(url).is_err() {
                    ctx.error("Could not open the browser");
                }
            }
        }
        Message::Copy => {
            if let Some((_, url)) = &state.report {
                ctx.success("Link copied");
                return clipboard::write(url.clone());
            }
        }
    }
    Task::none()
}

pub fn on_event(state: &mut State, event: &Event) {
    let Event::ConsoleOutput { server_id, line } = event else {
        return;
    };
    let Some((job, _)) = state.running else {
        return;
    };
    if state.server_id.as_ref() != Some(server_id) {
        return;
    }
    if let Some(url) = spark::report_url(&line.text) {
        state.running = None;
        state.report = Some((job, url));
    }
}

pub fn subscription(state: &State) -> Subscription<Message> {
    match state.running {
        Some(_) => iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick),
        None => Subscription::none(),
    }
}

fn progress(job: Job, started: Instant) -> String {
    let elapsed = started.elapsed().as_secs();
    match job {
        Job::Profile if elapsed < PROFILE_SECONDS => {
            format!("Profiling, {} s left", PROFILE_SECONDS - elapsed)
        }
        Job::Profile => "Uploading the profile".into(),
        Job::Health => "Building the health report".into(),
    }
}

pub fn view<'a>(
    state: &'a State,
    server: &'a ServerRecord,
    ctx: &Ctx,
) -> Option<Element<'a, Message>> {
    if !spark::supports(&server.framework) {
        return None;
    }
    let online = ctx.status(&server.id) == ServerRuntimeStatus::Online;
    let mut rows: Vec<Element<'a, Message>> = vec![text(
        "spark records what the server spends its time on and uploads a report you can open in the browser or share when asking for help.",
    )
    .size(TEXT_SM)
    .style(style::muted)
    .into()];

    match state.installed {
        None => {}
        Some(false) => rows.push(
            row![
                text("spark is not installed. It comes from Modrinth and loads after a restart.")
                    .size(TEXT_MD)
                    .width(Length::Fill),
                busy_button(
                    "Install spark",
                    "Installing",
                    state.installing,
                    Some(Message::Install),
                    style::primary,
                    BUTTON_MD,
                ),
            ]
            .spacing(SPACE_3)
            .align_y(Alignment::Center)
            .into(),
        ),
        Some(true) => {
            let busy = state.running.is_some();
            let idle = online && !busy && !ctx.needs_restart(&server.id);
            let profiling = matches!(state.running, Some((Job::Profile, _)));
            let checking = matches!(state.running, Some((Job::Health, _)));
            let mut actions = row![
                busy_button(
                    "Start profiler (60 s)",
                    "Profiling",
                    profiling,
                    idle.then_some(Message::Run(Job::Profile)),
                    style::primary,
                    BUTTON_MD,
                ),
                busy_button(
                    "Health report",
                    "Checking",
                    checking,
                    idle.then_some(Message::Run(Job::Health)),
                    style::secondary,
                    BUTTON_MD,
                ),
            ]
            .spacing(SPACE_2)
            .align_y(Alignment::Center);
            let note = match state.running {
                Some((job, started)) => Some(progress(job, started)),
                None if ctx.needs_restart(&server.id) => {
                    Some("Restart the server to load spark.".into())
                }
                None if !online => Some("Start the server to use spark.".into()),
                None => None,
            };
            if let Some(note) = note {
                actions = actions.push(text(note).size(TEXT_SM).style(style::muted));
            }
            rows.push(actions.into());
        }
    }

    if let Some((job, url)) = &state.report {
        let label = match job {
            Job::Profile => "Profile",
            Job::Health => "Health report",
        };
        rows.push(
            row![
                column![
                    text(label).size(TEXT_SM).style(style::muted),
                    text(url.as_str()).size(TEXT_MD).font(MONO),
                ]
                .width(Length::Fill),
                sized(text("Open").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                    .style(style::secondary)
                    .on_press(Message::Open),
                sized(text("Copy").size(TEXT_MD).font(UI_MEDIUM), BUTTON_MD)
                    .style(style::secondary)
                    .on_press(Message::Copy),
            ]
            .spacing(SPACE_2)
            .align_y(Alignment::Center)
            .into(),
        );
    }
    if let Some(error) = &state.error {
        rows.push(text(error.as_str()).size(TEXT_SM).color(CORAL).into());
    }
    Some(section("Performance", rows))
}
