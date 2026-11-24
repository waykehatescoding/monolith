use std::path::PathBuf;

use iced::{Element, Task};
use monolith_core::config_files::{yaml_get, yaml_set};
use monolith_core::types::ServerRecord;

use super::properties::{Kind, Spec, CONFIGS};
use super::settings::{integer, save_row, section, setting, spec_control, subheading};
use crate::ctx::Ctx;

#[derive(Default)]
pub struct State {
    server_id: Option<String>,
    root: PathBuf,
    files: Vec<File>,
    saving: Option<&'static str>,
}

struct File {
    path: &'static str,
    values: Vec<(&'static Spec, String)>,
    saved: Vec<String>,
}

impl File {
    fn dirty(&self) -> bool {
        self.values
            .iter()
            .zip(&self.saved)
            .any(|((_, now), before)| now != before)
    }

    fn invalid(&self) -> bool {
        self.values.iter().any(|(spec, value)| !spec.valid(value))
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(String, Vec<(&'static str, String)>),
    Value(&'static str, &'static str, String),
    Save(&'static str),
    Saved(&'static str, Result<(), String>),
}

fn paths() -> Vec<&'static str> {
    let mut paths: Vec<&'static str> = CONFIGS.iter().map(|spec| spec.group).collect();
    paths.dedup();
    paths
}

async fn save(
    root: PathBuf,
    path: &'static str,
    changes: Vec<(&'static str, String)>,
) -> Result<(), String> {
    let target = root.join(path);
    let mut text = tokio::fs::read_to_string(&target)
        .await
        .map_err(|e| e.to_string())?;
    for (key, value) in changes {
        text = yaml_set(&text, key, &value).ok_or(format!("{key} is no longer in the file"))?;
    }
    tokio::fs::write(&target, text)
        .await
        .map_err(|e| e.to_string())
}

pub fn on_show(state: &mut State, server: &ServerRecord) -> Task<Message> {
    if state.server_id.as_deref() != Some(server.id.as_str()) {
        *state = State {
            server_id: Some(server.id.clone()),
            root: PathBuf::from(&server.server_path),
            ..State::default()
        };
    }
    let id = server.id.clone();
    let root = state.root.clone();
    Task::perform(
        async move {
            let mut files = Vec::new();
            for path in paths() {
                if let Ok(text) = tokio::fs::read_to_string(root.join(path)).await {
                    files.push((path, text));
                }
            }
            (id, files)
        },
        |(id, files)| Message::Loaded(id, files),
    )
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    let Some(id) = state.server_id.clone() else {
        return Task::none();
    };
    match message {
        Message::Loaded(loaded_id, files) => {
            if loaded_id != id || state.saving.is_some() {
                return Task::none();
            }
            let previous = std::mem::take(&mut state.files);
            for (path, text) in files {
                if let Some(file) = previous.iter().find(|f| f.path == path && f.dirty()) {
                    state.files.push(File {
                        path,
                        values: file.values.clone(),
                        saved: file.saved.clone(),
                    });
                    continue;
                }
                let values: Vec<_> = CONFIGS
                    .iter()
                    .filter(|spec| spec.group == path)
                    .filter_map(|spec| Some((spec, yaml_get(&text, spec.key)?)))
                    .filter(|(spec, value)| spec.valid(value))
                    .collect();
                if !values.is_empty() {
                    state.files.push(File {
                        path,
                        saved: values.iter().map(|(_, v)| v.clone()).collect(),
                        values,
                    });
                }
            }
            Task::none()
        }
        Message::Value(path, key, value) => {
            if let Some((spec, current)) = state
                .files
                .iter_mut()
                .filter(|f| f.path == path)
                .flat_map(|f| f.values.iter_mut())
                .find(|(spec, _)| spec.key == key)
            {
                *current = match spec.kind {
                    Kind::Int(..) => integer(value),
                    _ => value,
                };
            }
            Task::none()
        }
        Message::Save(path) => {
            let Some(file) = state.files.iter().find(|f| f.path == path) else {
                return Task::none();
            };
            if state.saving.is_some() || file.invalid() {
                return Task::none();
            }
            let changes = file
                .values
                .iter()
                .zip(&file.saved)
                .filter(|((_, now), before)| now != *before)
                .map(|((spec, value), _)| (spec.key, value.clone()))
                .collect();
            state.saving = Some(path);
            Task::perform(save(state.root.clone(), path, changes), move |result| {
                Message::Saved(path, result)
            })
        }
        Message::Saved(path, result) => {
            state.saving = None;
            match result {
                Ok(()) => {
                    if let Some(file) = state.files.iter_mut().find(|f| f.path == path) {
                        file.saved = file.values.iter().map(|(_, v)| v.clone()).collect();
                    }
                    ctx.success(format!("Saved {path}. Restart the server to apply it."));
                    ctx.changed_while_running(&id);
                }
                Err(error) => ctx.error(format!("Could not save {path}: {error}")),
            }
            Task::none()
        }
    }
}

pub fn view(state: &State) -> Option<Element<'_, Message>> {
    if state.files.is_empty() {
        return None;
    }
    let mut rows = Vec::new();
    for file in &state.files {
        let path = file.path;
        rows.push(subheading(path));
        for (spec, value) in &file.values {
            let key = spec.key;
            rows.push(setting(
                spec.label,
                spec.hint,
                spec_control(spec, Some(value.as_str()), move |v| {
                    Message::Value(path, key, v)
                }),
            ));
        }
        let invalid = file.invalid();
        rows.push(save_row(
            "Save changes",
            "Saving",
            state.saving == Some(path),
            (file.dirty() && !invalid && state.saving.is_none()).then_some(Message::Save(path)),
            invalid.then_some("Fix the values outlined in red before saving."),
        ));
    }
    Some(section("Server software config", rows))
}
