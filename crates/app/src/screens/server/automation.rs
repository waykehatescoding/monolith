use std::time::Duration;

use chrono::{Local, NaiveDateTime};
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{time, Alignment, Element, Length, Subscription, Task};
use monolith_core::automation::{self, Automation, Schedule, ScheduledTask, Step};
use monolith_core::events::Event;
use monolith_theme::token::*;
use monolith_theme::widget::{sized, sized_icon};
use monolith_theme::{icon, style, Icon, UI_MEDIUM};

use super::settings::{dropdown, field, section, switch};
use crate::ctx::Ctx;
use crate::ui::busy_button;

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Command,
    Say,
    Wait,
    Restart,
    Stop,
    Start,
    Backup,
}

impl StepKind {
    const ALL: [StepKind; 7] = [
        StepKind::Command,
        StepKind::Say,
        StepKind::Wait,
        StepKind::Restart,
        StepKind::Stop,
        StepKind::Start,
        StepKind::Backup,
    ];

    fn placeholder(self) -> Option<&'static str> {
        match self {
            StepKind::Command => Some("weather clear"),
            StepKind::Say => Some("Restarting soon"),
            StepKind::Wait => Some("Seconds"),
            _ => None,
        }
    }
}

impl std::fmt::Display for StepKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            StepKind::Command => "Run command",
            StepKind::Say => "Broadcast message",
            StepKind::Wait => "Wait",
            StepKind::Restart => "Restart",
            StepKind::Stop => "Stop",
            StepKind::Start => "Start",
            StepKind::Backup => "Back up",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Daily,
    Weekly,
    Interval,
}

impl Repeat {
    const ALL: [Repeat; 3] = [Repeat::Daily, Repeat::Weekly, Repeat::Interval];
}

impl std::fmt::Display for Repeat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Repeat::Daily => "Every day",
            Repeat::Weekly => "On weekdays",
            Repeat::Interval => "Every few minutes or hours",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Minutes,
    Hours,
}

impl std::fmt::Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Unit::Minutes => "minutes",
            Unit::Hours => "hours",
        })
    }
}

#[derive(Debug, Clone)]
struct StepForm {
    kind: StepKind,
    value: String,
}

#[derive(Debug, Clone)]
struct Editor {
    index: Option<usize>,
    name: String,
    repeat: Repeat,
    times: String,
    days: [bool; 7],
    every: String,
    unit: Unit,
    only_online: bool,
    steps: Vec<StepForm>,
    error: Option<String>,
}

impl Editor {
    fn new() -> Self {
        Self {
            index: None,
            name: String::new(),
            repeat: Repeat::Daily,
            times: String::new(),
            days: [false; 7],
            every: "60".into(),
            unit: Unit::Minutes,
            only_online: true,
            steps: vec![StepForm {
                kind: StepKind::Command,
                value: String::new(),
            }],
            error: None,
        }
    }

    fn from_task(index: usize, task: &ScheduledTask) -> Self {
        let mut editor = Self {
            index: Some(index),
            name: task.name.clone(),
            only_online: task.only_when_online,
            steps: task.steps.iter().map(step_form).collect(),
            ..Self::new()
        };
        match &task.schedule {
            Schedule::Daily { times } => editor.times = times.join(", "),
            Schedule::Weekly { days, time } => {
                editor.repeat = Repeat::Weekly;
                editor.times = time.clone();
                for day in days {
                    if let Some(slot) = editor.days.get_mut(usize::from(*day)) {
                        *slot = true;
                    }
                }
            }
            Schedule::Interval { minutes } => {
                editor.repeat = Repeat::Interval;
                if minutes % 60 == 0 {
                    editor.every = (minutes / 60).to_string();
                    editor.unit = Unit::Hours;
                } else {
                    editor.every = minutes.to_string();
                }
            }
        }
        editor
    }

    fn schedule(&self) -> Result<Schedule, String> {
        Ok(match self.repeat {
            Repeat::Daily => Schedule::Daily {
                times: self
                    .times
                    .split(',')
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(String::from)
                    .collect(),
            },
            Repeat::Weekly => Schedule::Weekly {
                days: (0..7u8).filter(|d| self.days[usize::from(*d)]).collect(),
                time: self.times.trim().to_string(),
            },
            Repeat::Interval => {
                let amount: u32 = self
                    .every
                    .trim()
                    .parse()
                    .map_err(|_| "Enter how often the task runs".to_string())?;
                let minutes = match self.unit {
                    Unit::Minutes => amount,
                    Unit::Hours => amount.saturating_mul(60),
                };
                Schedule::Interval { minutes }
            }
        })
    }

    fn steps(&self) -> Result<Vec<Step>, String> {
        self.steps.iter().map(to_step).collect()
    }
}

fn step_form(step: &Step) -> StepForm {
    let (kind, value) = match step {
        Step::Command(command) => (StepKind::Command, command.clone()),
        Step::Say(message) => (StepKind::Say, message.clone()),
        Step::Wait(seconds) => (StepKind::Wait, seconds.to_string()),
        Step::Restart => (StepKind::Restart, String::new()),
        Step::Stop => (StepKind::Stop, String::new()),
        Step::Start => (StepKind::Start, String::new()),
        Step::Backup => (StepKind::Backup, String::new()),
    };
    StepForm { kind, value }
}

fn to_step(form: &StepForm) -> Result<Step, String> {
    let value = form.value.trim().to_string();
    Ok(match form.kind {
        StepKind::Command => Step::Command(value.trim_start_matches('/').to_string()),
        StepKind::Say => Step::Say(value),
        StepKind::Wait => Step::Wait(
            value
                .parse()
                .map_err(|_| "Enter the wait in whole seconds".to_string())?,
        ),
        StepKind::Restart => Step::Restart,
        StepKind::Stop => Step::Stop,
        StepKind::Start => Step::Start,
        StepKind::Backup => Step::Backup,
    })
}

#[derive(Debug, Default)]
pub struct State {
    server_id: Option<String>,
    automation: Automation,
    editor: Option<Editor>,
    saving: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    NewTask,
    EditTask(usize),
    Name(String),
    Repeat(Repeat),
    Times(String),
    Day(usize),
    Every(String),
    Unit(Unit),
    OnlyOnline(bool),
    StepKind(usize, StepKind),
    StepValue(usize, String),
    AddStep,
    RemoveStep(usize),
    CancelEdit,
    SaveEdit,
    ToggleTask(usize, bool),
    DeleteTask(usize),
    RunTask(usize),
    TaskRan(String, Result<String, String>),
    Saved(Automation, Result<(), String>),
}

fn save(state: &mut State, automation: Automation) -> Task<Message> {
    let Some(id) = state.server_id.clone() else {
        return Task::none();
    };
    state.saving = true;
    Task::perform(automation::save(id, automation.clone()), move |result| {
        Message::Saved(
            automation.clone(),
            if result.success {
                Ok(())
            } else {
                Err(result.error.unwrap_or_else(|| "Could not save".into()))
            },
        )
    })
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    if let Some(editor) = state.editor.as_mut() {
        match &message {
            Message::Name(value) => editor.name = value.clone(),
            Message::Repeat(value) => editor.repeat = *value,
            Message::Times(value) => editor.times = value.clone(),
            Message::Day(day) => {
                if let Some(slot) = editor.days.get_mut(*day) {
                    *slot = !*slot;
                }
            }
            Message::Every(value) => {
                editor.every = value.chars().filter(char::is_ascii_digit).collect()
            }
            Message::Unit(value) => editor.unit = *value,
            Message::OnlyOnline(value) => editor.only_online = *value,
            Message::StepKind(index, kind) => {
                if let Some(step) = editor.steps.get_mut(*index) {
                    step.kind = *kind;
                    step.value.clear();
                }
            }
            Message::StepValue(index, value) => {
                if let Some(step) = editor.steps.get_mut(*index) {
                    step.value = value.clone();
                }
            }
            Message::AddStep => editor.steps.push(StepForm {
                kind: StepKind::Command,
                value: String::new(),
            }),
            Message::RemoveStep(index) if *index < editor.steps.len() => {
                editor.steps.remove(*index);
            }
            _ => {}
        }
    }
    match message {
        Message::NewTask => {
            state.editor = Some(Editor::new());
            Task::none()
        }
        Message::EditTask(index) => {
            state.editor = state
                .automation
                .tasks
                .get(index)
                .map(|task| Editor::from_task(index, task));
            Task::none()
        }
        Message::CancelEdit => {
            state.editor = None;
            Task::none()
        }
        Message::SaveEdit => {
            let Some(editor) = state.editor.as_mut() else {
                return Task::none();
            };
            let built = editor.schedule().and_then(|schedule| {
                Ok(ScheduledTask {
                    id: editor
                        .index
                        .and_then(|i| state.automation.tasks.get(i))
                        .map(|task| task.id.clone())
                        .unwrap_or_else(automation::new_id),
                    name: editor.name.trim().to_string(),
                    enabled: editor
                        .index
                        .and_then(|i| state.automation.tasks.get(i))
                        .is_none_or(|task| task.enabled),
                    schedule,
                    steps: editor.steps()?,
                    only_when_online: editor.only_online,
                })
            });
            let mut next = state.automation.clone();
            let result = built.and_then(|task| {
                match editor.index {
                    Some(index) if index < next.tasks.len() => next.tasks[index] = task,
                    _ => next.tasks.push(task),
                }
                automation::validate(&next)
            });
            match result {
                Ok(()) => {
                    editor.error = None;
                    save(state, next)
                }
                Err(error) => {
                    editor.error = Some(error);
                    Task::none()
                }
            }
        }
        Message::ToggleTask(index, on) => {
            let mut next = state.automation.clone();
            match next.tasks.get_mut(index) {
                Some(task) => task.enabled = on,
                None => return Task::none(),
            }
            save(state, next)
        }
        Message::DeleteTask(index) => {
            let mut next = state.automation.clone();
            if index >= next.tasks.len() {
                return Task::none();
            }
            next.tasks.remove(index);
            state.editor = None;
            save(state, next)
        }
        Message::RunTask(index) => {
            let (Some(id), Some(task)) = (
                state.server_id.clone(),
                state.automation.tasks.get(index).cloned(),
            ) else {
                return Task::none();
            };
            let name = task.name.clone();
            Task::perform(automation::run_task(id, task), move |result| {
                Message::TaskRan(name.clone(), result)
            })
        }
        Message::TaskRan(name, result) => {
            match result {
                Ok(text) => ctx.success(format!("{name}: {text}")),
                Err(error) => ctx.error(format!("{name} failed: {error}")),
            }
            Task::none()
        }
        Message::Saved(next, result) => {
            state.saving = false;
            match result {
                Ok(()) => {
                    state.automation = next;
                    state.editor = None;
                    ctx.refresh_servers();
                }
                Err(error) => match state.editor.as_mut() {
                    Some(editor) => editor.error = Some(error),
                    None => ctx.error(format!("Could not save: {error}")),
                },
            }
            Task::none()
        }
        _ => Task::none(),
    }
}

fn when(at: NaiveDateTime) -> String {
    let today = Local::now().date_naive();
    if at.date() == today {
        at.format("%H:%M").to_string()
    } else if at.date() == today.succ_opt().unwrap_or(today) {
        at.format("tomorrow %H:%M").to_string()
    } else {
        at.format("%a %e %b %H:%M").to_string()
    }
}

fn describe(schedule: &Schedule) -> String {
    match schedule {
        Schedule::Daily { times } => format!("Every day at {}", times.join(", ")),
        Schedule::Weekly { days, time } => {
            let names: Vec<&str> = days
                .iter()
                .filter_map(|d| WEEKDAYS.get(usize::from(*d)).copied())
                .collect();
            format!("{} at {time}", names.join(", "))
        }
        Schedule::Interval { minutes } if minutes % 60 == 0 => match minutes / 60 {
            1 => "Every hour".into(),
            hours => format!("Every {hours} hours"),
        },
        Schedule::Interval { minutes } => format!("Every {minutes} minutes"),
    }
}

fn small(label: &str) -> iced::widget::Text<'_> {
    text(label).size(TEXT_MD).font(UI_MEDIUM)
}

fn task_row<'a>(state: &'a State, index: usize, task: &'a ScheduledTask) -> Element<'a, Message> {
    let mut meta = vec![describe(&task.schedule)];
    if task.enabled {
        if let Some(at) = automation::next_run_at(&task.id) {
            meta.push(format!("next {}", when(at)));
        }
    }
    let running = automation::is_task_running(&task.id);
    let last = match automation::last_result(&task.id) {
        _ if running => Some("Running now".to_string()),
        Some((at, result)) => Some(format!("Last run {}: {result}", when(at))),
        None => None,
    };
    let mut details = column![
        text(&task.name).size(TEXT_MD).font(UI_MEDIUM),
        text(meta.join(" · ")).size(TEXT_SM).style(style::muted),
    ]
    .spacing(SPACE_1)
    .width(Length::Fill);
    if let Some(last) = last {
        details = details.push(text(last).size(TEXT_SM).style(style::faint));
    }
    let idle = !state.saving;
    row![
        details,
        switch(task.enabled, move |on| Message::ToggleTask(index, on)),
        sized(small("Run now"), BUTTON_MD)
            .style(style::ghost)
            .on_press_maybe((!running).then_some(Message::RunTask(index))),
        sized(small("Edit"), BUTTON_MD)
            .style(style::ghost)
            .on_press_maybe(idle.then_some(Message::EditTask(index))),
        sized_icon(icon(Icon::Trash, 16.0), BUTTON_MD)
            .style(style::danger)
            .on_press_maybe(idle.then_some(Message::DeleteTask(index))),
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center)
    .into()
}

fn steps_view(steps: &[StepForm]) -> Element<'_, Message> {
    let mut list = column![].spacing(SPACE_2);
    for (index, step) in steps.iter().enumerate() {
        let mut line = row![dropdown(StepKind::ALL, Some(step.kind), move |kind| {
            Message::StepKind(index, kind)
        })
        .width(200)]
        .spacing(SPACE_2)
        .align_y(Alignment::Center);
        line = match step.kind.placeholder() {
            Some(placeholder) => line.push(
                field(placeholder, &step.value, false, move |value| {
                    Message::StepValue(index, value)
                })
                .width(Length::Fill),
            ),
            None => line.push(Space::new().width(Length::Fill)),
        };
        line = line.push(
            sized(small("Remove"), BUTTON_MD)
                .style(style::ghost)
                .on_press_maybe((steps.len() > 1).then_some(Message::RemoveStep(index))),
        );
        list = list.push(line);
    }
    column![
        text("Steps, in order").size(TEXT_SM).style(style::muted),
        list,
        row![sized(small("Add step"), BUTTON_SM)
            .style(style::secondary)
            .on_press(Message::AddStep)],
    ]
    .spacing(SPACE_2)
    .into()
}

fn labelled<'a>(label: &'a str, control: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    row![
        text(label).size(TEXT_MD).width(140),
        container(control).width(Length::Fill)
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center)
    .into()
}

fn editor_view<'a>(state: &'a State, editor: &'a Editor) -> Element<'a, Message> {
    let when: Element<'a, Message> = match editor.repeat {
        Repeat::Daily => labelled(
            "At",
            field("04:00, 16:00", &editor.times, false, Message::Times),
        ),
        Repeat::Weekly => {
            let days =
                WEEKDAYS
                    .iter()
                    .enumerate()
                    .fold(row![].spacing(SPACE_1), |bar, (index, day)| {
                        bar.push(
                            button(text(*day).size(TEXT_SM).font(UI_MEDIUM))
                                .padding([6, 10])
                                .style(style::tab(editor.days[index]))
                                .on_press(Message::Day(index)),
                        )
                    });
            column![
                labelled("On", days),
                labelled("At", field("20:00", &editor.times, false, Message::Times)),
            ]
            .spacing(SPACE_3)
            .into()
        }
        Repeat::Interval => labelled(
            "Every",
            row![
                field("60", &editor.every, false, Message::Every).width(96),
                dropdown(
                    [Unit::Minutes, Unit::Hours],
                    Some(editor.unit),
                    Message::Unit
                )
                .width(140),
            ]
            .spacing(SPACE_2),
        ),
    };
    let mut body = column![
        labelled(
            "Name",
            field("Nightly backup", &editor.name, false, Message::Name)
        ),
        labelled(
            "Repeat",
            dropdown(Repeat::ALL, Some(editor.repeat), Message::Repeat)
        ),
        when,
        labelled(
            "Only when online",
            row![
                switch(editor.only_online, Message::OnlyOnline),
                text("Skip this run if the server is stopped")
                    .size(TEXT_SM)
                    .style(style::muted)
            ]
            .spacing(SPACE_3)
            .align_y(Alignment::Center)
        ),
        steps_view(&editor.steps),
    ]
    .spacing(SPACE_4);
    if let Some(error) = &editor.error {
        body = body.push(text(error.as_str()).size(TEXT_SM).color(CORAL));
    }
    body = body.push(
        row![
            Space::new().width(Length::Fill),
            sized(small("Cancel"), BUTTON_MD)
                .style(style::ghost)
                .on_press_maybe((!state.saving).then_some(Message::CancelEdit)),
            busy_button(
                "Save task",
                "Saving",
                state.saving,
                Some(Message::SaveEdit),
                style::primary,
                BUTTON_MD
            ),
        ]
        .spacing(SPACE_2),
    );
    container(body)
        .padding(SPACE_4)
        .width(Length::Fill)
        .style(style::panel)
        .into()
}

fn tasks_view(state: &State) -> Element<'_, Message> {
    let mut rows: Vec<Element<'_, Message>> = Vec::new();
    for (index, task) in state.automation.tasks.iter().enumerate() {
        match &state.editor {
            Some(editor) if editor.index == Some(index) => rows.push(editor_view(state, editor)),
            _ => rows.push(task_row(state, index, task)),
        }
    }
    if state.automation.tasks.is_empty() && state.editor.is_none() {
        rows.push(
            text("Run commands, broadcasts, restarts or backups on a schedule.")
                .size(TEXT_SM)
                .style(style::muted)
                .into(),
        );
    }
    match &state.editor {
        Some(editor) if editor.index.is_none() => rows.push(editor_view(state, editor)),
        Some(_) => {}
        None => rows.push(
            row![sized(small("New task"), BUTTON_MD)
                .style(style::secondary)
                .on_press_maybe((!state.saving).then_some(Message::NewTask))]
            .into(),
        ),
    }
    section("Scheduled tasks", rows)
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let Some(server) = ctx.selected_server() else {
        return Space::new().into();
    };
    if state.server_id.as_deref() != Some(server.id.as_str()) {
        return text("Loading automation").style(style::muted).into();
    }
    let page = column![tasks_view(state)]
        .spacing(SPACE_6)
        .padding(iced::Padding {
            right: SPACE_4,
            bottom: SPACE_7,
            ..iced::Padding::ZERO
        });
    scrollable(page)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::scroll)
        .into()
}

pub fn on_event(_state: &mut State, _event: &Event, _ctx: &Ctx) -> Task<Message> {
    Task::none()
}

pub fn on_show(state: &mut State, ctx: &Ctx) -> Task<Message> {
    let Some(server) = ctx.selected_server() else {
        return Task::none();
    };
    if state.server_id.as_deref() != Some(server.id.as_str()) {
        state.editor = None;
        state.saving = false;
    }
    state.server_id = Some(server.id.clone());
    if !state.saving {
        state.automation = automation::of(server);
    }
    Task::none()
}

pub fn subscription(state: &State) -> Subscription<Message> {
    if state.automation.tasks.is_empty() {
        Subscription::none()
    } else {
        time::every(Duration::from_secs(1)).map(|_| Message::Tick)
    }
}
