use iced::widget::{
    button, column, container, pick_list, row, scrollable, text, text_input, tooltip, Space,
};
use monolith_theme::token::{BUTTON_MD, BUTTON_SM};
use monolith_theme::widget::{sized, sized_icon};
use iced::{Alignment, Element, Length, Subscription, Task};
use monolith_core::default_plugins;
use monolith_core::events::Event;
use monolith_core::modrinth;
use monolith_core::types::{DefaultPlugin, DefaultPluginSource, ModrinthSearchHit};
use monolith_theme::style;
use monolith_theme::token::{
    CORAL, INK, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, TEXT_LG, TEXT_MD, TEXT_SM, TEXT_XL,
    TEXT_XS,
};
use monolith_theme::{icon, Icon, UI_MEDIUM, UI_SEMIBOLD};
use serde_json::json;
use std::collections::BTreeSet;

use crate::ctx::Ctx;

const GENERAL: &str = "General";

#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    New,
    Rename(String),
}

#[derive(Debug, Default)]
pub struct State {
    plugins: Option<Vec<DefaultPlugin>>,
    extra_groups: Vec<String>,
    collapsed: BTreeSet<String>,
    adding: bool,
    target: Option<String>,
    query: String,
    searching: bool,
    searched: bool,
    hits: Vec<ModrinthSearchHit>,
    search_error: Option<String>,
    edit: Option<Edit>,
    edit_input: String,
    edit_error: Option<String>,
    deleting: Option<String>,
    busy: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(Vec<DefaultPlugin>),
    ToggleAdd,
    Target(String),
    Query(String),
    Search,
    Searched(Result<Vec<ModrinthSearchHit>, String>),
    AddHit(usize),
    PickFiles,
    Toggle(String),
    Move(String, String),
    Remove(String),
    StartEdit(Edit),
    EditInput(String),
    SubmitEdit,
    CancelEdit,
    AskDelete(String),
    CancelDelete,
    DeleteGroup(String, bool),
    Done(Result<String, String>),
}

fn key(plugin: &DefaultPlugin) -> &str {
    match &plugin.source {
        DefaultPluginSource::Modrinth { project_id, .. } => project_id,
        DefaultPluginSource::Manual { file_name, .. } => file_name,
    }
}

fn name(plugin: &DefaultPlugin) -> &str {
    match &plugin.source {
        DefaultPluginSource::Modrinth { title, .. } => title,
        DefaultPluginSource::Manual { file_name, .. } => file_name,
    }
}

fn group_of(plugin: &DefaultPlugin) -> &str {
    plugin.group.as_deref().unwrap_or(GENERAL)
}

fn group_arg(group: &str) -> Option<String> {
    (group != GENERAL).then(|| group.to_string())
}

impl State {
    fn groups(&self) -> Vec<String> {
        let mut names: BTreeSet<String> = self
            .plugins
            .iter()
            .flatten()
            .map(|p| group_of(p).to_string())
            .chain(self.extra_groups.iter().cloned())
            .collect();
        names.remove(GENERAL);
        let mut all = vec![GENERAL.to_string()];
        all.extend(names);
        all
    }

    fn target_group(&self) -> String {
        self.target.clone().unwrap_or_else(|| GENERAL.to_string())
    }

    fn keys_in(&self, group: &str) -> Vec<String> {
        self.plugins
            .iter()
            .flatten()
            .filter(|p| group_of(p) == group)
            .map(|p| key(p).to_string())
            .collect()
    }

    fn is_added(&self, project_id: &str) -> bool {
        self.plugins.iter().flatten().any(|p| {
            matches!(&p.source, DefaultPluginSource::Modrinth { project_id: id, .. } if id == project_id)
        })
    }
}

fn reload() -> Task<Message> {
    Task::perform(default_plugins::list_default_plugins(), Message::Loaded)
}

fn mutate<F>(state: &mut State, work: F) -> Task<Message>
where
    F: std::future::Future<Output = Result<String, String>> + Send + 'static,
{
    state.busy = true;
    Task::perform(work, Message::Done)
}

async fn move_plugin(key: String, group: Option<String>) -> Result<String, String> {
    let result = default_plugins::set_default_plugin_group(key, group).await?;
    if result.success {
        Ok(String::new())
    } else {
        Err(result
            .error
            .unwrap_or_else(|| "The plugin could not be moved.".into()))
    }
}

async fn remove_plugin(key: String) -> Result<String, String> {
    default_plugins::remove_default_plugin(key).await?;
    Ok("Removed from default plugins".into())
}

async fn add_hit(hit: ModrinthSearchHit, group: Option<String>) -> Result<String, String> {
    let result = default_plugins::add_default_plugin(json!({
        "projectId": hit.project_id,
        "title": hit.title,
        "slug": hit.slug,
        "iconUrl": hit.icon_url,
        "group": group,
    }))
    .await?;
    if result.success {
        Ok(format!("{} added to default plugins", hit.title))
    } else {
        Err(result
            .error
            .unwrap_or_else(|| "The plugin could not be added.".into()))
    }
}

async fn add_files(group: Option<String>) -> Result<String, String> {
    let Some(files) = rfd::AsyncFileDialog::new()
        .set_title("Add plugin files")
        .add_filter("Plugin files", &["jar"])
        .pick_files()
        .await
    else {
        return Ok(String::new());
    };
    let paths = files.iter().map(|f| f.path().to_path_buf()).collect();
    let result = default_plugins::add_manual_default_plugins(paths, group).await?;
    if result.success {
        Ok(match result.added.len() {
            1 => "1 plugin file added".into(),
            n => format!("{n} plugin files added"),
        })
    } else {
        Err(result.error.unwrap_or_else(|| "No files were added.".into()))
    }
}

async fn delete_group(keys: Vec<String>, remove: bool) -> Result<String, String> {
    for key in keys {
        if remove {
            default_plugins::remove_default_plugin(key).await?;
        } else {
            move_plugin(key, None).await?;
        }
    }
    Ok("Group deleted".into())
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Loaded(plugins) => {
            state.plugins = Some(plugins);
            Task::none()
        }
        Message::ToggleAdd => {
            state.adding = !state.adding;
            if state.adding && !state.searched && state.hits.is_empty() {
                return update(state, Message::Search, ctx);
            }
            Task::none()
        }
        Message::Target(group) => {
            state.target = Some(group);
            Task::none()
        }
        Message::Query(query) => {
            state.query = query;
            Task::none()
        }
        Message::Search => {
            let query = state.query.trim().to_string();
            if state.searching {
                return Task::none();
            }
            let sort = if query.is_empty() { "downloads" } else { "relevance" };
            state.searching = true;
            state.search_error = None;
            state.hits.clear();
            Task::perform(
                async move {
                    modrinth::search_modrinth(
                        json!({ "query": query, "projectType": "plugin", "sort": sort, "limit": 20 }),
                    )
                    .await
                    .map(|result| result.hits)
                },
                Message::Searched,
            )
        }
        Message::Searched(result) => {
            state.searching = false;
            state.searched = true;
            match result {
                Ok(hits) => state.hits = hits,
                Err(error) => {
                    state.hits.clear();
                    state.search_error = Some(error);
                }
            }
            Task::none()
        }
        Message::AddHit(index) => match state.hits.get(index).cloned() {
            Some(hit) if !state.busy => {
                let group = group_arg(&state.target_group());
                mutate(state, add_hit(hit, group))
            }
            _ => Task::none(),
        },
        Message::PickFiles => {
            if state.busy {
                return Task::none();
            }
            let group = group_arg(&state.target_group());
            mutate(state, add_files(group))
        }
        Message::Toggle(group) => {
            if !state.collapsed.remove(&group) {
                state.collapsed.insert(group);
            }
            Task::none()
        }
        Message::Move(plugin, group) => {
            if state.busy {
                return Task::none();
            }
            mutate(state, move_plugin(plugin, group_arg(&group)))
        }
        Message::Remove(plugin) => {
            if state.busy {
                return Task::none();
            }
            mutate(state, remove_plugin(plugin))
        }
        Message::StartEdit(edit) => {
            state.edit_error = None;
            state.edit_input = match &edit {
                Edit::New => String::new(),
                Edit::Rename(group) => group.clone(),
            };
            state.edit = Some(edit);
            Task::none()
        }
        Message::EditInput(value) => {
            state.edit_input = value;
            state.edit_error = None;
            Task::none()
        }
        Message::CancelEdit => {
            state.edit = None;
            state.edit_error = None;
            Task::none()
        }
        Message::SubmitEdit => {
            let new_name = state.edit_input.trim().to_string();
            let Some(edit) = state.edit.clone() else {
                return Task::none();
            };
            if new_name.is_empty() || new_name.eq_ignore_ascii_case(GENERAL) {
                state.edit_error = Some("Pick a group name that is not empty or General.".into());
                return Task::none();
            }
            let renaming = match &edit {
                Edit::Rename(old) => Some(old.as_str()),
                Edit::New => None,
            };
            if state
                .groups()
                .iter()
                .any(|g| g.eq_ignore_ascii_case(&new_name) && Some(g.as_str()) != renaming)
            {
                state.edit_error = Some("A group with that name already exists.".into());
                return Task::none();
            }
            if renaming.is_some() && state.busy {
                return Task::none();
            }
            state.edit = None;
            state.edit_error = None;
            match edit {
                Edit::New => {
                    state.extra_groups.push(new_name);
                    Task::none()
                }
                Edit::Rename(old) => {
                    let keys = state.keys_in(&old);
                    state.extra_groups.retain(|g| *g != old);
                    if state.target.as_deref() == Some(old.as_str()) {
                        state.target = Some(new_name.clone());
                    }
                    if state.collapsed.remove(&old) {
                        state.collapsed.insert(new_name.clone());
                    }
                    if keys.is_empty() {
                        state.extra_groups.push(new_name);
                        return Task::none();
                    }
                    mutate(state, async move {
                        for key in keys {
                            move_plugin(key, Some(new_name.clone())).await?;
                        }
                        Ok("Group renamed".into())
                    })
                }
            }
        }
        Message::AskDelete(group) => {
            if state.keys_in(&group).is_empty() {
                state.extra_groups.retain(|g| *g != group);
            } else {
                state.deleting = Some(group);
            }
            Task::none()
        }
        Message::CancelDelete => {
            state.deleting = None;
            Task::none()
        }
        Message::DeleteGroup(group, remove) => {
            if state.busy {
                return Task::none();
            }
            state.deleting = None;
            state.extra_groups.retain(|g| *g != group);
            state.collapsed.remove(&group);
            if state.target.as_deref() == Some(group.as_str()) {
                state.target = None;
            }
            let keys = state.keys_in(&group);
            mutate(state, delete_group(keys, remove))
        }
        Message::Done(result) => {
            state.busy = false;
            match result {
                Ok(text) if !text.is_empty() => ctx.success(text),
                Ok(_) => {}
                Err(error) => ctx.error(format!("{error} Try again.")),
            }
            reload()
        }
    }
}

fn action<'a>(
    label: &'a str,
    glyph: Icon,
    message: Option<Message>,
    primary: bool,
) -> Element<'a, Message> {
    sized(
        row![icon(glyph, 16.0), text(label).size(TEXT_MD).font(UI_MEDIUM)]
            .spacing(SPACE_2)
            .align_y(Alignment::Center), BUTTON_MD)
    .style(if primary { style::primary } else { style::secondary })
    .on_press_maybe(message)
    .into()
}

fn small<'a>(label: &'a str, message: Option<Message>, danger: bool) -> Element<'a, Message> {
    sized(text(label).size(TEXT_SM).font(UI_MEDIUM), BUTTON_SM)
        .style(if danger { style::danger } else { style::ghost })
        .on_press_maybe(message)
        .into()
}

fn icon_action<'a>(glyph: Icon, label: &'a str, message: Option<Message>) -> Element<'a, Message> {
    tooltip(
        sized_icon(icon(glyph, 14.0), BUTTON_SM)
            .style(style::icon_button)
            .on_press_maybe(message),
        container(text(label).size(TEXT_XS).color(INK))
            .padding([4, 8])
            .style(style::tooltip),
        tooltip::Position::Left,
    )
    .into()
}

fn plugin_row<'a>(state: &'a State, plugin: &'a DefaultPlugin) -> Element<'a, Message> {
    let source = match &plugin.source {
        DefaultPluginSource::Modrinth { .. } => "Modrinth",
        DefaultPluginSource::Manual { .. } => "Added from a file",
    };
    let plugin_key = key(plugin).to_string();
    let current = group_of(plugin);
    let others: Vec<String> = state.groups().into_iter().filter(|g| g != current).collect();
    let move_key = plugin_key.clone();
    let mut controls = row![].spacing(SPACE_2).align_y(Alignment::Center);
    if !others.is_empty() {
        controls = controls.push(
            pick_list(others, None::<String>, move |group| {
                Message::Move(move_key.clone(), group)
            })
            .placeholder("Move to")
            .text_size(TEXT_SM)
            .style(style::pick)
            .menu_style(style::pick_menu),
        );
    }
    controls = controls.push(icon_action(
        Icon::Trash,
        "Remove from default plugins",
        (!state.busy).then_some(Message::Remove(plugin_key)),
    ));

    row![
        icon(Icon::Package, 20.0),
        column![
            text(name(plugin)).size(TEXT_MD).font(UI_MEDIUM),
            text(source).size(TEXT_SM).style(style::muted),
        ]
        .spacing(2)
        .width(Length::Fill),
        controls,
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center)
    .padding([SPACE_2, SPACE_3])
    .into()
}

fn edit_row<'a>(state: &'a State) -> Element<'a, Message> {
    let label = match &state.edit {
        Some(Edit::Rename(_)) => "Rename",
        _ => "Create group",
    };
    let field = text_input("Group name", &state.edit_input)
        .on_input(Message::EditInput)
        .on_submit(Message::SubmitEdit)
        .size(TEXT_MD)
        .padding(SPACE_2)
        .width(280);
    let field = if state.edit_error.is_some() {
        field.style(style::input_error)
    } else {
        field.style(style::input)
    };
    let mut block = column![row![
        field,
        small(label, Some(Message::SubmitEdit), false),
        small("Cancel", Some(Message::CancelEdit), false),
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center)]
    .spacing(SPACE_1);
    if let Some(error) = &state.edit_error {
        block = block.push(text(error).size(TEXT_SM).style(style::colored(CORAL)));
    }
    block.into()
}

fn group_section<'a>(state: &'a State, group: &str) -> Element<'a, Message> {
    let members: Vec<&DefaultPlugin> = state
        .plugins
        .iter()
        .flatten()
        .filter(|p| group_of(p) == group)
        .collect();
    let open = !state.collapsed.contains(group);
    let count = match members.len() {
        1 => "1 plugin".to_string(),
        n => format!("{n} plugins"),
    };

    let mut header = row![
        button(
            row![
                icon(
                    if open { Icon::ChevronDown } else { Icon::ChevronRight },
                    16.0
                ),
                text(group.to_string()).size(TEXT_LG).font(UI_SEMIBOLD),
                text(count).size(TEXT_SM).style(style::muted),
            ]
            .spacing(SPACE_2)
            .align_y(Alignment::Center)
        )
        .padding([SPACE_1, SPACE_2])
        .style(style::ghost)
        .on_press(Message::Toggle(group.to_string())),
        Space::new().width(Length::Fill),
    ]
    .align_y(Alignment::Center);
    if group != GENERAL {
        header = header
            .push(icon_action(
                Icon::Edit,
                "Rename group",
                Some(Message::StartEdit(Edit::Rename(group.to_string()))),
            ))
            .push(icon_action(
                Icon::Trash,
                "Delete group",
                Some(Message::AskDelete(group.to_string())),
            ));
    }

    let mut section = column![header].spacing(SPACE_1);

    if state.deleting.as_deref() == Some(group) {
        section = section.push(
            container(
                column![
                    text(format!(
                        "Delete {group}? Its {} can move to General or be removed from default plugins.",
                        if members.len() == 1 {
                            "plugin".to_string()
                        } else {
                            format!("{} plugins", members.len())
                        }
                    ))
                    .size(TEXT_MD),
                    row![
                        small(
                            "Move to General",
                            Some(Message::DeleteGroup(group.to_string(), false)),
                            false
                        ),
                        small(
                            "Remove plugins",
                            Some(Message::DeleteGroup(group.to_string(), true)),
                            true
                        ),
                        small("Cancel", Some(Message::CancelDelete), false),
                    ]
                    .spacing(SPACE_2),
                ]
                .spacing(SPACE_2),
            )
            .padding(SPACE_3)
            .width(Length::Fill)
            .style(style::well),
        );
    }

    if open {
        if members.is_empty() {
            section = section.push(
                text("No plugins here yet. Use Add plugins to put some in this group, or move one over from another group. An empty group is forgotten when you leave this page.")
                    .size(TEXT_SM)
                    .style(style::muted),
            );
        } else {
            for plugin in members {
                section = section.push(plugin_row(state, plugin));
            }
        }
    }
    section.into()
}

fn hit_row<'a>(state: &'a State, index: usize, hit: &'a ModrinthSearchHit) -> Element<'a, Message> {
    let added = state.is_added(&hit.project_id);
    let press = (!added && !state.busy).then_some(Message::AddHit(index));
    row![
        icon(Icon::Package, 20.0),
        column![
            text(&hit.title).size(TEXT_MD).font(UI_MEDIUM),
            text(&hit.description)
                .size(TEXT_SM)
                .style(style::muted),
        ]
        .spacing(2)
        .width(Length::Fill),
        small(if added { "Added" } else { "Add" }, press, false),
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center)
    .padding([SPACE_2, SPACE_3])
    .into()
}

fn add_panel<'a>(state: &'a State) -> Element<'a, Message> {
    let search = row![
        text_input("Search Modrinth, or browse the most popular plugins below", &state.query)
            .on_input(Message::Query)
            .on_submit(Message::Search)
            .size(TEXT_MD)
            .padding(SPACE_2)
            .style(style::input)
            .width(Length::Fill),
        action(
            if state.searching { "Searching" } else { "Search" },
            Icon::Search,
            (!state.searching).then_some(Message::Search),
            true,
        ),
    ]
    .spacing(SPACE_2)
    .align_y(Alignment::Center);

    let target = row![
        text("Add to").size(TEXT_MD).style(style::muted),
        pick_list(state.groups(), Some(state.target_group()), Message::Target)
            .text_size(TEXT_MD)
            .style(style::pick)
            .menu_style(style::pick_menu),
        Space::new().width(Length::Fill),
        action(
            "Add .jar files",
            Icon::Upload,
            (!state.busy).then_some(Message::PickFiles),
            false,
        ),
    ]
    .spacing(SPACE_3)
    .align_y(Alignment::Center);

    let mut results = column![].spacing(SPACE_1);
    if let Some(error) = &state.search_error {
        results = results.push(
            text(format!(
                "{error} Check your internet connection and search again."
            ))
            .size(TEXT_SM)
            .style(style::colored(CORAL)),
        );
    } else if state.searched && state.hits.is_empty() {
        results = results.push(
            text("Nothing matched that search. Try a shorter name or a different spelling.")
                .size(TEXT_SM)
                .style(style::muted),
        );
    }
    for (index, hit) in state.hits.iter().enumerate() {
        results = results.push(hit_row(state, index, hit));
    }

    container(column![target, search, results].spacing(SPACE_3))
        .padding(SPACE_4)
        .width(Length::Fill)
        .style(style::panel)
        .into()
}

fn empty<'a>() -> Element<'a, Message> {
    column![
        text("No default plugins yet").size(TEXT_LG).font(UI_SEMIBOLD),
        text("Add the plugins you install on every server, such as permissions or world protection. New servers can start with them already in place.")
            .size(TEXT_MD)
            .style(style::muted),
        Space::new().height(SPACE_1),
        action("Add plugins", Icon::Plus, Some(Message::ToggleAdd), true),
    ]
    .spacing(SPACE_2)
    .max_width(520)
    .into()
}

pub fn view<'a>(state: &'a State, _ctx: &'a Ctx) -> Element<'a, Message> {
    let Some(plugins) = &state.plugins else {
        return container(text("Loading default plugins").size(TEXT_MD).style(style::muted))
            .padding([0.0, SPACE_4])
            .into();
    };

    let is_empty = plugins.is_empty() && state.extra_groups.is_empty() && !state.adding;
    let mut header = row![column![
        text("Default plugins").size(TEXT_XL).font(UI_SEMIBOLD),
        text("Installed automatically into new servers. Group them to keep setups tidy.")
            .size(TEXT_MD)
            .style(style::muted),
    ]
    .spacing(SPACE_1)
    .width(Length::Fill)]
    .spacing(SPACE_2)
    .align_y(Alignment::Center);
    if !is_empty {
        header = header.push(action(
            if state.adding { "Done adding" } else { "Add plugins" },
            if state.adding { Icon::Check } else { Icon::Plus },
            Some(Message::ToggleAdd),
            !state.adding,
        ));
    }
    header = header.push(action(
        "New group",
        Icon::Folder,
        Some(Message::StartEdit(Edit::New)),
        false,
    ));
    let mut page = column![header].spacing(SPACE_4);

    if state.edit.is_some() {
        page = page.push(edit_row(state));
    }
    if state.adding {
        page = page.push(add_panel(state));
    }

    if is_empty {
        page = page.push(empty());
    } else {
        for group in state.groups() {
            page = page.push(group_section(state, &group));
        }
    }

    scrollable(container(page.spacing(SPACE_5)).padding([0.0, SPACE_4]))
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
