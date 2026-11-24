use std::path::PathBuf;

use iced::widget::{column, container, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length, Subscription, Task};
use monolith_core::events::Event;
use monolith_core::templates::{
    delete_template, export_template, import_template, list_templates, rename_template,
    ServerTemplate,
};
use monolith_theme::token::*;
use monolith_theme::widget::sized;
use monolith_theme::{icon, style, Icon, UI_MEDIUM, UI_SEMIBOLD};

use crate::ctx::Ctx;

#[derive(Debug, Default)]
pub struct State {
    templates: Option<Vec<ServerTemplate>>,
    renaming: Option<(String, String)>,
    deleting: Option<String>,
    busy: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(Vec<ServerTemplate>),
    Rename(String, String),
    RenameInput(String),
    SaveRename,
    CancelRename,
    AskDelete(String),
    Delete(String),
    CancelDelete,
    Export(String, String),
    Import,
    Done(Result<Option<String>, String>),
}

fn reload() -> Task<Message> {
    Task::perform(list_templates(), Message::Loaded)
}

fn summary(template: &ServerTemplate) -> String {
    let mut parts = vec![
        format!("{} {}", template.framework, template.version),
        format!("{:.1} GB", template.ram_mb as f64 / 1024.0),
    ];
    match template.content.len() {
        0 => {}
        1 => parts.push("1 plugin or mod".into()),
        n => parts.push(format!("{n} plugins and mods")),
    }
    match template.files.len() {
        0 => {}
        1 => parts.push("1 config file".into()),
        n => parts.push(format!("{n} config files")),
    }
    parts.join(" · ")
}

fn file_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| if r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    format!("{}.json", safe.trim())
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Loaded(templates) => {
            state.templates = Some(templates);
            Task::none()
        }
        Message::Rename(id, name) => {
            state.renaming = Some((id, name));
            state.deleting = None;
            Task::none()
        }
        Message::RenameInput(value) => {
            if let Some((_, name)) = &mut state.renaming {
                *name = value;
            }
            Task::none()
        }
        Message::CancelRename => {
            state.renaming = None;
            Task::none()
        }
        Message::SaveRename => {
            let Some((id, name)) = state.renaming.clone() else {
                return Task::none();
            };
            if name.trim().is_empty() || state.busy {
                return Task::none();
            }
            state.busy = true;
            Task::perform(
                async move { rename_template(id, name).await.map(|()| None) },
                Message::Done,
            )
        }
        Message::AskDelete(id) => {
            state.deleting = Some(id);
            state.renaming = None;
            Task::none()
        }
        Message::CancelDelete => {
            state.deleting = None;
            Task::none()
        }
        Message::Delete(id) => {
            state.busy = true;
            Task::perform(
                async move {
                    delete_template(id)
                        .await
                        .map(|()| Some("Template deleted".to_string()))
                },
                Message::Done,
            )
        }
        Message::Export(id, name) => {
            state.busy = true;
            Task::perform(
                async move {
                    let Some(target) = rfd::AsyncFileDialog::new()
                        .set_file_name(file_name(&name))
                        .add_filter("Monolith template", &["json"])
                        .save_file()
                        .await
                    else {
                        return Ok(None);
                    };
                    export_template(id, target.path().to_path_buf()).await?;
                    Ok(Some(format!("Exported {name}")))
                },
                Message::Done,
            )
        }
        Message::Import => {
            state.busy = true;
            Task::perform(
                async {
                    let Some(file) = rfd::AsyncFileDialog::new()
                        .set_title("Choose a template")
                        .add_filter("Monolith template", &["json"])
                        .pick_file()
                        .await
                    else {
                        return Ok(None);
                    };
                    let template = import_template(PathBuf::from(file.path())).await?;
                    Ok(Some(format!("Imported {}", template.name)))
                },
                Message::Done,
            )
        }
        Message::Done(result) => {
            state.busy = false;
            match result {
                Ok(note) => {
                    state.renaming = None;
                    state.deleting = None;
                    if let Some(note) = note {
                        ctx.success(note);
                    }
                }
                Err(error) => ctx.error(error),
            }
            reload()
        }
    }
}

fn small<'a>(label: &'a str, message: Option<Message>, look: Look) -> Element<'a, Message> {
    sized(text(label).size(TEXT_SM).font(UI_MEDIUM), BUTTON_SM)
        .style(look)
        .on_press_maybe(message)
        .into()
}

type Look = fn(&iced::Theme, iced::widget::button::Status) -> iced::widget::button::Style;

fn template_row<'a>(state: &'a State, template: &'a ServerTemplate) -> Element<'a, Message> {
    let idle = !state.busy;
    if let Some((id, name)) = &state.renaming {
        if *id == template.id {
            let save = (idle && !name.trim().is_empty()).then_some(Message::SaveRename);
            return row![
                text_input("Template name", name)
                    .size(TEXT_MD)
                    .padding([8, 12])
                    .style(style::input)
                    .on_input(Message::RenameInput)
                    .on_submit_maybe(save.clone())
                    .width(Length::Fill),
                small("Cancel", Some(Message::CancelRename), style::ghost),
                small("Save", save, style::primary),
            ]
            .spacing(SPACE_2)
            .align_y(Alignment::Center)
            .into();
        }
    }
    let actions = if state.deleting.as_deref() == Some(template.id.as_str()) {
        row![
            text("Delete this template?")
                .size(TEXT_SM)
                .style(style::muted),
            small("Cancel", Some(Message::CancelDelete), style::ghost),
            small(
                "Delete",
                idle.then(|| Message::Delete(template.id.clone())),
                style::danger
            ),
        ]
    } else {
        row![
            small(
                "Rename",
                idle.then(|| Message::Rename(template.id.clone(), template.name.clone())),
                style::ghost
            ),
            small(
                "Export",
                idle.then(|| Message::Export(template.id.clone(), template.name.clone())),
                style::ghost
            ),
            small(
                "Delete",
                idle.then(|| Message::AskDelete(template.id.clone())),
                style::ghost
            ),
        ]
    };
    row![
        column![
            text(&template.name).size(TEXT_MD).font(UI_MEDIUM),
            text(summary(template)).size(TEXT_SM).style(style::muted)
        ]
        .spacing(2)
        .width(Length::Fill),
        actions.spacing(SPACE_1).align_y(Alignment::Center),
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center)
    .into()
}

pub fn view<'a>(state: &'a State, _ctx: &'a Ctx) -> Element<'a, Message> {
    let header = row![
        column![
            text("Templates").size(TEXT_XL).font(UI_SEMIBOLD),
            text("Start new servers from a saved setup. Save one from a server's settings, or import a template a friend shared.")
                .size(TEXT_MD)
                .style(style::muted),
        ]
        .spacing(SPACE_1)
        .width(Length::Fill),
        sized(
            row![icon(Icon::Download, 16.0), text("Import").size(TEXT_MD).font(UI_MEDIUM)]
                .spacing(SPACE_2)
                .align_y(Alignment::Center),
            BUTTON_MD,
        )
        .style(style::secondary)
        .on_press_maybe((!state.busy).then_some(Message::Import)),
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center);

    let body: Element<'a, Message> = match &state.templates {
        None => text("Loading templates")
            .size(TEXT_MD)
            .style(style::muted)
            .into(),
        Some(templates) if templates.is_empty() => {
            text("No templates yet. Open a server's settings and choose Save as template.")
                .size(TEXT_MD)
                .style(style::muted)
                .into()
        }
        Some(templates) => templates
            .iter()
            .fold(column![].spacing(SPACE_4), |list, template| {
                list.push(template_row(state, template))
            })
            .into(),
    };

    scrollable(container(column![header, body].spacing(SPACE_5)).padding([0.0, SPACE_4]))
        .style(style::scroll)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

pub fn on_event(_state: &mut State, _event: &Event, _ctx: &Ctx) -> Task<Message> {
    Task::none()
}

pub fn on_show(_state: &mut State, _ctx: &Ctx) -> Task<Message> {
    reload()
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::none()
}
