use std::path::{Path, PathBuf};
use std::time::Duration;

use iced::widget::{
    button, column, container, row, scrollable, text, text_editor, text_input, tooltip, Space,
};
use iced::{Alignment, Background, Border, Element, Length, Subscription, Task};
use monolith_core::events::Event;
use monolith_core::safety::{resolve_safe_server_path, sanitize_download_file_name};
use monolith_core::server_files;
use monolith_core::types::{FileEntry, SuccessResult};
use monolith_theme::token::{
    AMBER, CORAL, FOCUS_WIDTH, GROUND, INK, INK_SECONDARY, INK_TERTIARY, MOSS, RADIUS_PANEL, SPACE_1, SPACE_2,
    SPACE_3, SPACE_4, SPACE_5, SPACE_6, TEXT_LG, TEXT_MD, TEXT_SM, TEXT_XS,
};
use monolith_theme::{icon_colored, style, Icon, MONO, UI_MEDIUM};

use crate::ctx::{Confirm, Ctx};

const MAX_EDIT_BYTES: u64 = 2 * 1024 * 1024;
const PROMPT_ID: &str = "file-prompt";

#[derive(Default)]
pub struct State {
    server_id: Option<String>,
    path: Vec<String>,
    entries: Vec<FileEntry>,
    loaded: bool,
    failure: Option<String>,
    editor: Option<Editor>,
    prompt: Option<Prompt>,
    notice: Option<String>,
    busy: bool,
    watch: u8,
}

struct Editor {
    name: String,
    content: text_editor::Content,
    original: String,
    pending: String,
    dirty: bool,
    saving: bool,
    leaving: bool,
}

#[derive(Clone)]
pub enum PromptKind {
    NewFile,
    NewFolder,
    Rename(String),
    Duplicate(String),
}

struct Prompt {
    kind: PromptKind,
    input: String,
    error: Option<String>,
}

#[derive(Clone)]
pub enum Message {
    Loaded(String, Vec<String>, Vec<FileEntry>, Option<String>),
    Tick,
    Enter(String),
    Crumb(usize),
    Open(String),
    Opened(String, Vec<String>, Result<(String, String), String>),
    Edit(text_editor::Action),
    Save,
    Saved(Result<(), String>),
    Retry,
    Revert,
    Close,
    Discard,
    KeepEditing,
    Ask(PromptKind),
    PromptInput(String),
    PromptSubmit,
    PromptCancel,
    PromptDone(Result<String, String>),
    Delete(String),
    Upload,
    Uploaded(Result<String, String>),
    Explorer,
}

impl State {
    fn dir(&self) -> String {
        self.path.join("/")
    }

    fn rel(&self, name: &str) -> String {
        if self.path.is_empty() {
            name.to_string()
        } else {
            format!("{}/{name}", self.dir())
        }
    }
}

fn check(result: SuccessResult) -> Result<(), String> {
    if result.success {
        Ok(())
    } else {
        Err(result
            .error
            .unwrap_or_else(|| "Something went wrong.".into()))
    }
}

fn root_of(ctx: &Ctx, id: &str) -> Option<String> {
    ctx.server(id).map(|server| server.server_path.clone())
}

fn reload(state: &State, ctx: &Ctx) -> Task<Message> {
    let Some(id) = state.server_id.clone() else {
        return Task::none();
    };
    let Some(root) = root_of(ctx, &id) else {
        return Task::none();
    };
    let path = state.path.clone();
    let rel = state.dir();
    Task::perform(
        async move {
            let entries = server_files::list_server_files(id.clone(), rel.clone()).await;
            let failure = if entries.is_empty() {
                match resolve_safe_server_path(Path::new(&root), &rel, true) {
                    Ok(dir) => tokio::fs::read_dir(&dir).await.err().map(|e| e.to_string()),
                    Err(error) => Some(error),
                }
            } else {
                None
            };
            (id, path, entries, failure)
        },
        |(id, path, entries, failure)| Message::Loaded(id, path, entries, failure),
    )
}

fn is_binary_name(name: &str) -> bool {
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "jar" | "zip" | "gz" | "7z" | "rar" | "png" | "jpg" | "jpeg" | "gif" | "ico" | "dat"
            | "dat_old" | "mca" | "mcr" | "nbt" | "db" | "sqlite" | "exe" | "dll" | "class" | "bin"
    )
}

fn copy_name(name: &str) -> String {
    match name.rfind('.') {
        Some(at) if at > 0 => format!("{} copy{}", &name[..at], &name[at..]),
        _ => format!("{name} copy"),
    }
}

fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let value = bytes as f64;
    if value < KB {
        format!("{bytes} B")
    } else if value < KB * KB {
        format!("{:.1} KB", value / KB)
    } else if value < KB * KB * KB {
        format!("{:.1} MB", value / (KB * KB))
    } else {
        format!("{:.1} GB", value / (KB * KB * KB))
    }
}

fn format_modified(stamp: &str) -> String {
    match (stamp.get(..10), stamp.get(11..16)) {
        (Some(date), Some(time)) => format!("{date} {time}"),
        _ => String::new(),
    }
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Loaded(id, path, entries, failure) => {
            if state.server_id.as_deref() == Some(id.as_str()) && state.path == path {
                let changed = state
                    .entries
                    .iter()
                    .map(|entry| &entry.name)
                    .ne(entries.iter().map(|entry| &entry.name));
                if changed {
                    state.watch = 0;
                }
                state.entries = entries;
                state.failure = failure;
                state.loaded = true;
            }
            Task::none()
        }
        Message::Retry => {
            state.loaded = false;
            reload(state, ctx)
        }
        Message::Tick => {
            state.watch = state.watch.saturating_sub(1);
            reload(state, ctx)
        }
        Message::Enter(name) => {
            state.path.push(name);
            state.entries.clear();
            state.loaded = false;
            state.prompt = None;
            state.notice = None;
            reload(state, ctx)
        }
        Message::Crumb(depth) => {
            state.path.truncate(depth);
            state.entries.clear();
            state.loaded = false;
            state.prompt = None;
            state.notice = None;
            reload(state, ctx)
        }
        Message::Open(name) => {
            let Some(id) = state.server_id.clone() else {
                return Task::none();
            };
            let size = state
                .entries
                .iter()
                .find(|entry| entry.name == name)
                .map_or(0, |entry| entry.size);
            if size > MAX_EDIT_BYTES {
                state.notice = Some(format!(
                    "{name} is larger than 2 MB, so it can't be edited here. Open the folder and use an editor on your PC."
                ));
                return Task::none();
            }
            if is_binary_name(&name) {
                state.notice = Some(format!(
                    "{name} is a binary file, so it can't be edited as text. Use Open folder to work with it on your PC."
                ));
                return Task::none();
            }
            state.notice = None;
            let rel = state.rel(&name);
            let path = state.path.clone();
            let key = id.clone();
            Task::perform(
                async move {
                    let result = server_files::read_server_file(id, rel).await;
                    match (result.success, result.content) {
                        (true, Some(content)) if content.contains('\0') => Err(format!(
                            "{name} looks like a binary file, so it can't be edited as text."
                        )),
                        (true, Some(content)) => Ok((name, content)),
                        _ => Err(result
                            .error
                            .unwrap_or_else(|| format!("Couldn't read {name}."))),
                    }
                },
                move |result| Message::Opened(key.clone(), path.clone(), result),
            )
        }
        Message::Opened(id, path, result) => {
            if state.server_id.as_deref() != Some(id.as_str()) || state.path != path {
                return Task::none();
            }
            match result {
                Ok((name, content)) => {
                    state.editor = Some(Editor {
                        name,
                        content: text_editor::Content::with_text(&content),
                        original: content,
                        pending: String::new(),
                        dirty: false,
                        saving: false,
                        leaving: false,
                    });
                }
                Err(error) => state.notice = Some(error),
            }
            Task::none()
        }
        Message::Edit(action) => {
            if let Some(editor) = &mut state.editor {
                if editor.saving && action.is_edit() {
                    return Task::none();
                }
                let edit = action.is_edit();
                editor.content.perform(action);
                if edit {
                    editor.dirty = editor.content.text() != editor.original;
                }
            }
            Task::none()
        }
        Message::Save => {
            let (Some(id), Some(name)) = (
                state.server_id.clone(),
                state.editor.as_ref().map(|editor| editor.name.clone()),
            ) else {
                return Task::none();
            };
            let rel = state.rel(&name);
            let Some(editor) = state.editor.as_mut() else {
                return Task::none();
            };
            if editor.saving {
                return Task::none();
            }
            editor.saving = true;
            let text = editor.content.text();
            editor.pending = text.clone();
            Task::perform(
                async move { check(server_files::write_server_file(id, rel, text).await) },
                Message::Saved,
            )
        }
        Message::Saved(result) => {
            let Some(editor) = &mut state.editor else {
                return Task::none();
            };
            editor.saving = false;
            match result {
                Ok(()) => {
                    editor.original = std::mem::take(&mut editor.pending);
                    editor.dirty = false;
                    ctx.success(format!("Saved {}", editor.name));
                    if editor.leaving {
                        state.editor = None;
                    }
                    reload(state, ctx)
                }
                Err(error) => {
                    editor.leaving = false;
                    ctx.error(format!("Couldn't save {}. {error}", editor.name));
                    Task::none()
                }
            }
        }
        Message::Revert => {
            if let Some(editor) = &mut state.editor {
                editor.content = text_editor::Content::with_text(&editor.original);
                editor.dirty = false;
            }
            Task::none()
        }
        Message::Close => {
            match &mut state.editor {
                Some(editor) if editor.dirty => editor.leaving = true,
                _ => state.editor = None,
            }
            Task::none()
        }
        Message::Discard => {
            state.editor = None;
            Task::none()
        }
        Message::KeepEditing => {
            if let Some(editor) = &mut state.editor {
                editor.leaving = false;
            }
            Task::none()
        }
        Message::Ask(kind) => {
            let input = match &kind {
                PromptKind::NewFile | PromptKind::NewFolder => String::new(),
                PromptKind::Rename(name) => name.clone(),
                PromptKind::Duplicate(name) => copy_name(name),
            };
            state.prompt = Some(Prompt {
                kind,
                input,
                error: None,
            });
            iced::widget::operation::focus(PROMPT_ID)
        }
        Message::PromptInput(value) => {
            if let Some(prompt) = &mut state.prompt {
                prompt.input = value;
                prompt.error = None;
            }
            Task::none()
        }
        Message::PromptCancel => {
            state.prompt = None;
            Task::none()
        }
        Message::PromptSubmit => submit(state, ctx),
        Message::PromptDone(result) => {
            state.busy = false;
            match result {
                Ok(done) => {
                    state.prompt = None;
                    ctx.success(done);
                    reload(state, ctx)
                }
                Err(error) => {
                    if let Some(prompt) = &mut state.prompt {
                        prompt.error = Some(error);
                    }
                    Task::none()
                }
            }
        }
        Message::Delete(name) => {
            let Some(id) = state.server_id.clone() else {
                return Task::none();
            };
            let is_dir = state
                .entries
                .iter()
                .any(|entry| entry.name == name && entry.is_directory);
            let rel = state.rel(&name);
            let body = if is_dir {
                format!("{name} and everything inside it will be permanently deleted. This can't be undone.")
            } else {
                format!("{name} will be permanently deleted. This can't be undone.")
            };
            state.watch = 10;
            ctx.confirm(
                Confirm::new(format!("Delete {name}?"), body, "Delete", true, move || {
                    let (id, rel, name) = (id.clone(), rel.clone(), name.clone());
                    async move {
                        check(server_files::delete_server_file(id, rel).await)
                            .map(|()| format!("Deleted {name}"))
                    }
                })
                .busy_label("Deleting"),
            );
            Task::none()
        }
        Message::Upload => {
            let (Some(id), true) = (state.server_id.clone(), !state.busy) else {
                return Task::none();
            };
            let Some(root) = root_of(ctx, &id) else {
                return Task::none();
            };
            state.busy = true;
            let rel = state.dir();
            Task::perform(upload(root, rel), Message::Uploaded)
        }
        Message::Uploaded(result) => {
            state.busy = false;
            match result {
                Ok(done) if done.is_empty() => Task::none(),
                Ok(done) => {
                    ctx.success(done);
                    reload(state, ctx)
                }
                Err(error) => {
                    ctx.error(error);
                    Task::none()
                }
            }
        }
        Message::Explorer => {
            let Some(root) = state.server_id.as_deref().and_then(|id| root_of(ctx, id)) else {
                return Task::none();
            };
            let mut target = PathBuf::from(root);
            target.extend(&state.path);
            if let Err(error) = open::that_detached(&target) {
                ctx.error(format!("Couldn't open the folder in Explorer. {error}"));
            }
            Task::none()
        }
    }
}

fn submit(state: &mut State, ctx: &Ctx) -> Task<Message> {
    if state.busy {
        return Task::none();
    }
    let (Some(id), Some(prompt)) = (state.server_id.clone(), state.prompt.as_ref()) else {
        return Task::none();
    };
    let kind = prompt.kind.clone();
    let name = match sanitize_download_file_name(prompt.input.trim()) {
        Ok(name) => name,
        Err(_) => {
            if let Some(prompt) = &mut state.prompt {
                prompt.error = Some(
                    "That name can't be used. Enter a name without slashes.".into(),
                );
            }
            return Task::none();
        }
    };
    if name.ends_with(['.', ' ']) || name.contains(['<', '>', ':', '"', '|', '?', '*']) {
        if let Some(prompt) = &mut state.prompt {
            prompt.error = Some("Names can't contain < > : \" | ? * or end with a dot or space.".into());
        }
        return Task::none();
    }
    let unchanged = matches!(&kind, PromptKind::Rename(old) if *old == name);
    if unchanged {
        state.prompt = None;
        return Task::none();
    }
    let exists = state
        .entries
        .iter()
        .any(|entry| entry.name.eq_ignore_ascii_case(&name));
    if exists {
        if let Some(prompt) = &mut state.prompt {
            prompt.error = Some(format!("{name} already exists here. Choose a different name."));
        }
        return Task::none();
    }
    let Some(root) = root_of(ctx, &id) else {
        return Task::none();
    };
    state.busy = true;
    let dir = state.dir();
    let source = match &kind {
        PromptKind::Rename(old) | PromptKind::Duplicate(old) => state.rel(old),
        _ => String::new(),
    };
    Task::perform(
        async move {
            match kind {
                PromptKind::NewFile => {
                    let target = child_path(&root, &dir, &name)?;
                    tokio::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&target)
                        .await
                        .map(|_| format!("Created {name}"))
                        .map_err(|e| match e.kind() {
                            std::io::ErrorKind::AlreadyExists => {
                                format!("{name} already exists here. Choose a different name.")
                            }
                            _ => format!("Couldn't create {name}. {e}"),
                        })
                }
                PromptKind::NewFolder => {
                    let target = child_path(&root, &dir, &name)?;
                    tokio::fs::create_dir(&target)
                        .await
                        .map(|()| format!("Created folder {name}"))
                        .map_err(|e| format!("Couldn't create {name}. {e}"))
                }
                PromptKind::Rename(_) => check(
                    server_files::rename_server_file(id, source, name.clone()).await,
                )
                .map(|()| format!("Renamed to {name}")),
                PromptKind::Duplicate(_) => {
                    check(server_files::copy_server_file(id, source, name.clone()).await)
                        .map(|()| format!("Created {name}"))
                }
            }
        },
        Message::PromptDone,
    )
}

fn child_path(root: &str, dir: &str, name: &str) -> Result<PathBuf, String> {
    resolve_safe_server_path(Path::new(root), dir, true).map(|base| base.join(name))
}

async fn copy_new(source: &Path, target: &Path) -> std::io::Result<()> {
    let mut from = tokio::fs::File::open(source).await?;
    let mut to = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .await?;
    if let Err(error) = tokio::io::copy(&mut from, &mut to).await {
        drop(to);
        let _ = tokio::fs::remove_file(target).await;
        return Err(error);
    }
    Ok(())
}

async fn upload(root: String, dir: String) -> Result<String, String> {
    let Some(files) = rfd::AsyncFileDialog::new()
        .set_title("Add files to this folder")
        .pick_files()
        .await
    else {
        return Ok(String::new());
    };
    let base = resolve_safe_server_path(Path::new(&root), &dir, true)?;
    let targets: Vec<(PathBuf, PathBuf)> = files
        .iter()
        .map(|file| {
            let name = file.file_name();
            (file.path().to_path_buf(), base.join(name))
        })
        .collect();
    if let Some((_, existing)) = targets.iter().find(|(_, target)| target.exists()) {
        let name = existing
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        return Err(format!(
            "{name} already exists in this folder. Rename or delete it, then add it again."
        ));
    }
    for (index, (source, target)) in targets.iter().enumerate() {
        if let Err(error) = copy_new(source, target).await {
            for (_, done) in &targets[..index] {
                let _ = tokio::fs::remove_file(done).await;
            }
            return Err(format!(
                "Couldn't add {}. {error} Nothing was added.",
                source.display()
            ));
        }
    }
    Ok(match targets.len() {
        1 => "Added 1 file".to_string(),
        n => format!("Added {n} files"),
    })
}

pub fn view<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    if ctx.selected_server().is_none() {
        return notice_view("Select a server to browse its files.");
    }
    match &state.editor {
        Some(editor) => editor_view(editor),
        None => browser_view(state),
    }
}

fn notice_view<'a>(message: &'a str) -> Element<'a, Message> {
    container(text(message).size(TEXT_MD).style(style::muted))
        .center(Length::Fill)
        .into()
}

fn hint<'a>(
    content: impl Into<Element<'a, Message>>,
    label: &'a str,
) -> Element<'a, Message> {
    tooltip(
        content,
        container(text(label).size(TEXT_XS).color(INK))
            .padding([4, 8])
            .style(style::tooltip),
        tooltip::Position::Top,
    )
    .gap(SPACE_2)
    .into()
}

fn icon_action<'a>(
    glyph: Icon,
    label: &'a str,
    message: Message,
    look: fn(&iced::Theme, button::Status) -> button::Style,
) -> Element<'a, Message> {
    hint(
        button(icon_colored(glyph, 16.0, INK_SECONDARY))
            .padding(6)
            .style(look)
            .on_press(message),
        label,
    )
}

fn labeled<'a>(
    glyph: Icon,
    label: &'a str,
    message: Option<Message>,
    look: fn(&iced::Theme, button::Status) -> button::Style,
) -> Element<'a, Message> {
    let body = row![
        icon_colored(glyph, 16.0, INK_SECONDARY),
        text(label).size(TEXT_MD).font(UI_MEDIUM)
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center);
    let base = button(body).padding([6, 12]).style(look);
    match message {
        Some(message) => base.on_press(message).into(),
        None => base.into(),
    }
}

fn browser_view<'a>(state: &'a State) -> Element<'a, Message> {
    let mut crumbs = row![crumb("Server folder", 0, state.path.is_empty())]
        .spacing(SPACE_1)
        .align_y(Alignment::Center);
    for (index, name) in state.path.iter().enumerate() {
        crumbs = crumbs
            .push(icon_colored(Icon::ChevronRight, 14.0, INK_TERTIARY))
            .push(crumb(name, index + 1, index + 1 == state.path.len()));
    }

    let busy = state.busy;
    let toolbar = row![
        crumbs,
        Space::new().width(Length::Fill),
        labeled(
            Icon::File,
            "New file",
            Some(Message::Ask(PromptKind::NewFile)),
            style::secondary
        ),
        labeled(
            Icon::Folder,
            "New folder",
            Some(Message::Ask(PromptKind::NewFolder)),
            style::secondary
        ),
        labeled(
            Icon::Upload,
            if busy { "Adding" } else { "Add files" },
            (!busy).then_some(Message::Upload),
            style::secondary
        ),
        labeled(
            Icon::ExternalLink,
            "Open folder",
            Some(Message::Explorer),
            style::ghost
        ),
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center);

    let mut page = column![toolbar].spacing(SPACE_3);
    if let Some(prompt) = &state.prompt {
        page = page.push(prompt_view(prompt, state.busy));
    }
    if let Some(notice) = &state.notice {
        page = page.push(text(notice).size(TEXT_SM).color(AMBER));
    }
    page.push(list_view(state)).into()
}

fn crumb<'a>(label: &'a str, depth: usize, current: bool) -> Element<'a, Message> {
    let base = button(text(label).size(TEXT_MD).font(UI_MEDIUM))
        .padding([4, 8])
        .style(style::sidebar_item(current));
    if current {
        base.into()
    } else {
        base.on_press(Message::Crumb(depth)).into()
    }
}

fn prompt_view<'a>(prompt: &'a Prompt, busy: bool) -> Element<'a, Message> {
    let title = match &prompt.kind {
        PromptKind::NewFile => "Name the new file".to_string(),
        PromptKind::NewFolder => "Name the new folder".to_string(),
        PromptKind::Rename(name) => format!("Rename {name}"),
        PromptKind::Duplicate(name) => format!("Duplicate {name} as"),
    };
    let action = match &prompt.kind {
        PromptKind::NewFile | PromptKind::NewFolder => "Create",
        PromptKind::Rename(_) => "Rename",
        PromptKind::Duplicate(_) => "Duplicate",
    };
    let mut field = text_input("Name", &prompt.input)
        .id(PROMPT_ID)
        .size(TEXT_MD)
        .padding([8, 12])
        .style(if prompt.error.is_some() {
            style::input_error
        } else {
            style::input
        });
    if !busy {
        field = field
            .on_input(Message::PromptInput)
            .on_submit(Message::PromptSubmit);
    }
    let mut body = column![
        text(title).size(TEXT_MD).font(UI_MEDIUM),
        row![
            field.width(Length::Fill),
            crate::ui::busy_button(
                action,
                "Working",
                busy,
                Some(Message::PromptSubmit),
                style::primary
            ),
            button(text("Cancel").size(TEXT_MD).font(UI_MEDIUM))
                .padding([8, 16])
                .style(style::ghost)
                .on_press_maybe((!busy).then_some(Message::PromptCancel)),
        ]
        .spacing(SPACE_2)
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_2);
    if let Some(error) = &prompt.error {
        body = body.push(text(error).size(TEXT_SM).color(CORAL));
    }
    container(body)
        .padding(SPACE_4)
        .width(Length::Fill)
        .style(style::panel)
        .into()
}

fn list_view<'a>(state: &'a State) -> Element<'a, Message> {
    if !state.loaded {
        return container(text("Loading files").size(TEXT_MD).style(style::muted))
            .padding(SPACE_5)
            .into();
    }
    if let Some(error) = &state.failure {
        return container(
            column![
                text("Couldn't read this folder").size(TEXT_LG).font(UI_MEDIUM),
                text(error).size(TEXT_MD).color(CORAL),
                labeled(Icon::Restart, "Try again", Some(Message::Retry), style::primary),
            ]
            .spacing(SPACE_3)
            .align_x(Alignment::Center),
        )
        .center_x(Length::Fill)
        .padding(SPACE_6)
        .into();
    }
    if state.entries.is_empty() {
        return container(
            column![
                text("This folder is empty").size(TEXT_LG).font(UI_MEDIUM),
                text("Create a file here, or add files from your PC.")
                    .size(TEXT_MD)
                    .style(style::muted),
                labeled(
                    Icon::Upload,
                    "Add files",
                    (!state.busy).then_some(Message::Upload),
                    style::primary
                ),
            ]
            .spacing(SPACE_3)
            .align_x(Alignment::Center),
        )
        .center_x(Length::Fill)
        .padding(SPACE_6)
        .into();
    }
    let rows = state
        .entries
        .iter()
        .fold(column![].spacing(2), |list, entry| list.push(entry_row(entry)));
    scrollable(rows)
        .style(style::scroll)
        .height(Length::Fill)
        .into()
}

fn entry_row<'a>(entry: &'a FileEntry) -> Element<'a, Message> {
    let name = entry.name.as_str();
    let (glyph, opens, size) = if entry.is_directory {
        (Icon::Folder, Message::Enter(entry.name.clone()), String::new())
    } else {
        (
            Icon::File,
            Message::Open(entry.name.clone()),
            format_size(entry.size),
        )
    };
    let main = button(
        row![
            icon_colored(
                glyph,
                16.0,
                if entry.is_directory { MOSS } else { INK_SECONDARY }
            ),
            text(name).size(TEXT_MD).color(INK).width(Length::Fill),
            text(size)
                .size(TEXT_SM)
                .style(style::muted)
                .width(Length::Fixed(88.0))
                .align_x(iced::alignment::Horizontal::Right),
            text(format_modified(&entry.modified_at))
                .size(TEXT_SM)
                .style(style::muted)
                .width(Length::Fixed(132.0))
                .align_x(iced::alignment::Horizontal::Right),
        ]
        .spacing(SPACE_3)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([8, 12])
    .style(style::ghost)
    .on_press(opens);

    let mut actions = row![icon_action(
        Icon::Edit,
        "Rename",
        Message::Ask(PromptKind::Rename(entry.name.clone())),
        style::icon_button
    )]
    .spacing(SPACE_1);
    actions = actions.push(icon_action(
        Icon::Copy,
        "Duplicate",
        Message::Ask(PromptKind::Duplicate(entry.name.clone())),
        style::icon_button,
    ));
    actions = actions.push(icon_action(
        Icon::Trash,
        "Delete",
        Message::Delete(entry.name.clone()),
        style::danger,
    ));

    row![main, actions]
        .spacing(SPACE_2)
        .align_y(Alignment::Center)
        .into()
}

fn editor_view<'a>(editor: &'a Editor) -> Element<'a, Message> {
    let status: Element<'a, Message> = if editor.saving {
        text("Saving").size(TEXT_SM).style(style::muted).into()
    } else if editor.dirty {
        text("Unsaved changes").size(TEXT_SM).color(AMBER).into()
    } else {
        text("Saved").size(TEXT_SM).style(style::faint).into()
    };

    let mut header = row![
        icon_action(Icon::ChevronLeft, "Back to files", Message::Close, style::icon_button),
        text(editor.name.as_str()).size(TEXT_LG).font(UI_MEDIUM),
        status,
        Space::new().width(Length::Fill),
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center);

    if editor.leaving {
        header = header
            .push(text("Save before leaving?").size(TEXT_MD).style(style::muted))
            .push(
                button(text("Discard").size(TEXT_MD).font(UI_MEDIUM))
                    .padding([6, 12])
                    .style(style::danger)
                    .on_press(Message::Discard),
            )
            .push(
                button(text("Keep editing").size(TEXT_MD).font(UI_MEDIUM))
                    .padding([6, 12])
                    .style(style::secondary)
                    .on_press(Message::KeepEditing),
            )
            .push(crate::ui::busy_button(
                "Save and close",
                "Saving",
                editor.saving,
                Some(Message::Save),
                style::primary,
            ));
    } else {
        header = header
            .push(labeled(
                Icon::Restart,
                "Revert",
                editor.dirty.then_some(Message::Revert),
                style::ghost,
            ))
            .push(crate::ui::busy_button(
                "Save",
                "Saving",
                editor.saving,
                editor.dirty.then_some(Message::Save),
                style::primary,
            ));
    }

    let area = text_editor(&editor.content)
        .on_action(Message::Edit)
        .font(MONO)
        .size(TEXT_SM)
        .padding(SPACE_4)
        .height(Length::Fill)
        .style(editor_style);

    column![header, area].spacing(SPACE_3).into()
}

fn editor_style(_: &iced::Theme, status: text_editor::Status) -> text_editor::Style {
    let focused = matches!(status, text_editor::Status::Focused { .. });
    text_editor::Style {
        background: Background::Color(GROUND),
        border: Border {
            color: if focused {
                MOSS.scale_alpha(0.55)
            } else {
                iced::Color::TRANSPARENT
            },
            width: if focused { FOCUS_WIDTH } else { 0.0 },
            radius: iced::border::radius(RADIUS_PANEL),
        },
        placeholder: INK_TERTIARY,
        value: INK,
        selection: MOSS.scale_alpha(0.28),
    }
}

pub fn on_event(state: &mut State, event: &Event, ctx: &Ctx) -> Task<Message> {
    match event {
        Event::ServerStatus(_) if state.loaded => reload(state, ctx),
        _ => Task::none(),
    }
}

pub fn on_show(state: &mut State, ctx: &Ctx) -> Task<Message> {
    if state.server_id != ctx.selected {
        *state = State::default();
        state.server_id = ctx.selected.clone();
    }
    reload(state, ctx)
}

pub fn subscription(state: &State) -> Subscription<Message> {
    if state.watch > 0 {
        iced::time::every(Duration::from_millis(600)).map(|_| Message::Tick)
    } else {
        Subscription::none()
    }
}
