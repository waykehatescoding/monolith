use std::collections::HashMap;

use iced::widget::{
    button, column, container, row, scrollable, text, text_input, toggler, tooltip,
};
use iced::{Alignment, Element, Length, Padding, Subscription, Task};
use monolith_core::ai_assistant::{self, AiChatRequest};
use monolith_core::ai_settings;
use monolith_core::events::Event;
use monolith_core::server_files;
use monolith_core::types::{
    AiAssistantAction, AiAssistantActionKind, AiAssistantActionResult, AiAssistantChatResult,
    AiAssistantProvider, PublicAiAssistantSettings,
};
use monolith_theme::token::{
    CORAL, MOSS, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, TEXT_LG, TEXT_MD, TEXT_SM, TEXT_XL,
    TEXT_XS,
};
use monolith_theme::{icon, icon_colored, style, Icon, MONO, UI_MEDIUM, UI_SEMIBOLD};
use serde_json::json;

use crate::ctx::{Ctx, Route, SettingsPage};
use crate::ui::busy_button;

const PROVIDERS: [AiAssistantProvider; 4] = [
    AiAssistantProvider::Openai,
    AiAssistantProvider::Openrouter,
    AiAssistantProvider::Groq,
    AiAssistantProvider::Custom,
];

fn provider_label(provider: AiAssistantProvider) -> &'static str {
    match provider {
        AiAssistantProvider::Openai => "OpenAI",
        AiAssistantProvider::Openrouter => "OpenRouter",
        AiAssistantProvider::Groq => "Groq",
        AiAssistantProvider::Custom => "Custom",
    }
}

fn preset(provider: AiAssistantProvider) -> (&'static str, &'static str) {
    match provider {
        AiAssistantProvider::Openai => ("gpt-4o-mini", "https://api.openai.com/v1"),
        AiAssistantProvider::Openrouter => ("openai/gpt-4o-mini", "https://openrouter.ai/api/v1"),
        AiAssistantProvider::Groq => ("llama-3.3-70b-versatile", "https://api.groq.com/openai/v1"),
        AiAssistantProvider::Custom => ("gpt-4o-mini", "http://localhost:1234/v1"),
    }
}

fn valid_base_url(url: &str) -> Result<(), &'static str> {
    let url = url.trim();
    if let Some(rest) = url.strip_prefix("https://") {
        return if rest.split(['/', ':']).next().is_some_and(|h| !h.is_empty()) {
            Ok(())
        } else {
            Err("The base URL needs a host name, for example https://api.openai.com/v1.")
        };
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return Err("The base URL must start with https://.");
    };
    match rest.split(['/', ':']).next().unwrap_or_default() {
        "localhost" | "127.0.0.1" | "[::1]" => Ok(()),
        _ => Err("Plain http:// is only allowed for localhost. Use https:// so your key is encrypted."),
    }
}

fn needs_key(provider: AiAssistantProvider) -> bool {
    provider != AiAssistantProvider::Custom
}

#[derive(Debug, Clone, PartialEq)]
enum Progress {
    Pending,
    Running,
    Done(String),
    Failed(String),
    Dismissed,
}

#[derive(Debug, Clone)]
pub enum FileState {
    Loading,
    Missing,
    Unreadable(String),
    Text(String),
}

#[derive(Debug, Clone)]
struct Card {
    action: AiAssistantAction,
    current: FileState,
    progress: Progress,
    undo: Option<AiAssistantAction>,
}

#[derive(Debug)]
enum Entry {
    User(String),
    Assistant { text: String, cards: Vec<Card> },
    Failure(String),
}

#[derive(Debug, Default)]
pub struct State {
    settings: Option<PublicAiAssistantSettings>,
    provider: AiAssistantProvider,
    model: String,
    base_url: String,
    api_key: String,
    saving: bool,
    enabling: bool,
    generation: u32,
    save_error: Option<String>,
    entries: Vec<Entry>,
    input: String,
    sending: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(PublicAiAssistantSettings),
    Enabled(bool),
    Provider(AiAssistantProvider),
    Model(String),
    BaseUrl(String),
    ApiKey(String),
    Save,
    Saved(Result<PublicAiAssistantSettings, String>),
    OpenSettings,
    Close,
    Input(String),
    Send,
    Replied(u32, AiAssistantChatResult),
    FileLoaded(String, FileState),
    Apply(String),
    Applied(String, Result<AiAssistantActionResult, String>),
    Dismiss(String),
    Undo(String),
    Undone(String, Result<AiAssistantActionResult, String>),
    Clear,
}

fn adopt(state: &mut State, settings: PublicAiAssistantSettings) {
    state.provider = settings.provider;
    state.model = settings.model.clone();
    state.base_url = settings.base_url.clone();
    state.api_key.clear();
    state.settings = Some(settings);
}

fn configured(state: &State) -> bool {
    state
        .settings
        .as_ref()
        .is_some_and(|s| s.enabled && (s.has_api_key || !needs_key(s.provider)))
}

fn find_card<'a>(state: &'a mut State, id: &str) -> Option<&'a mut Card> {
    state
        .entries
        .iter_mut()
        .filter_map(|entry| match entry {
            Entry::Assistant { cards, .. } => Some(cards),
            _ => None,
        })
        .flatten()
        .find(|card| card.action.id == id)
}

fn run_action(action: &AiAssistantAction) -> Task<Result<AiAssistantActionResult, String>> {
    match serde_json::to_value(action) {
        Ok(value) => Task::perform(ai_assistant::apply_ai_assistant_action(value), |r| r),
        Err(e) => Task::done(Err(e.to_string())),
    }
}

fn outcome(
    result: Result<AiAssistantActionResult, String>,
) -> Result<AiAssistantActionResult, String> {
    match result {
        Ok(r) if r.success => Ok(r),
        Ok(r) => Err(r.error.unwrap_or_else(|| "The action did not run.".into())),
        Err(e) => Err(e),
    }
}

fn chat_request(state: &State, ctx: &Ctx) -> serde_json::Result<AiChatRequest> {
    let messages: Vec<_> = state
        .entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::User(text) => Some(json!({ "role": "user", "content": text })),
            Entry::Assistant { text, .. } if !text.is_empty() => {
                Some(json!({ "role": "assistant", "content": text }))
            }
            _ => None,
        })
        .collect();
    let server = ctx.selected_server();
    serde_json::from_value(json!({
        "messages": messages,
        "context": {
            "locale": "en",
            "route": server.map_or("/".to_string(), |s| format!("/servers/{}", s.id)),
            "pageTitle": server.map_or("Monolith".to_string(), |s| format!("{} server", s.name)),
            "serverId": server.map(|s| s.id.clone()),
        },
    }))
}

pub fn update(state: &mut State, message: Message, ctx: &Ctx) -> Task<Message> {
    match message {
        Message::Loaded(settings) => {
            if !state.saving {
                adopt(state, settings);
            }
            Task::none()
        }
        Message::Enabled(enabled) => {
            if state.enabling {
                return Task::none();
            }
            state.enabling = true;
            state.save_error = None;
            Task::perform(
                ai_settings::update_ai_assistant_settings(
                    json!({ "enabled": enabled, "onboardingCompleted": true }),
                ),
                Message::Saved,
            )
        }
        Message::Provider(provider) => {
            let (old_model, old_url) = preset(state.provider);
            let (model, base_url) = preset(provider);
            if state.model.trim().is_empty() || state.model == old_model {
                state.model = model.into();
            }
            if state.base_url.trim().is_empty() || state.base_url == old_url {
                state.base_url = base_url.into();
            }
            state.provider = provider;
            state.save_error = None;
            Task::none()
        }
        Message::Model(value) => {
            state.model = value;
            Task::none()
        }
        Message::BaseUrl(value) => {
            state.base_url = value;
            Task::none()
        }
        Message::ApiKey(value) => {
            state.api_key = value;
            Task::none()
        }
        Message::Save => {
            if state.model.trim().is_empty() || state.base_url.trim().is_empty() {
                state.save_error = Some("Fill in the model and the base URL before saving.".into());
                return Task::none();
            }
            if let Err(reason) = valid_base_url(&state.base_url) {
                state.save_error = Some(reason.into());
                return Task::none();
            }
            state.saving = true;
            state.save_error = None;
            let mut update = json!({
                "provider": state.provider,
                "model": state.model.trim(),
                "baseUrl": state.base_url.trim(),
                "onboardingCompleted": true,
            });
            if !state.api_key.trim().is_empty() {
                update["apiKey"] = json!(state.api_key.trim());
            }
            Task::perform(
                ai_settings::update_ai_assistant_settings(update),
                Message::Saved,
            )
        }
        Message::Saved(result) => {
            state.saving = false;
            state.enabling = false;
            match result {
                Ok(settings) => {
                    adopt(state, settings);
                    ctx.success("Assistant settings saved");
                }
                Err(e) => {
                    state.save_error = Some(format!("Could not save the assistant settings: {e}"));
                }
            }
            Task::none()
        }
        Message::OpenSettings => {
            ctx.navigate(Route::Settings(SettingsPage::Assistant));
            ctx.toggle_assistant();
            Task::none()
        }
        Message::Close => {
            ctx.toggle_assistant();
            Task::none()
        }
        Message::Input(value) => {
            state.input = value;
            Task::none()
        }
        Message::Send => {
            let prompt = state.input.trim().to_string();
            if prompt.is_empty() || state.sending {
                return Task::none();
            }
            state.entries.push(Entry::User(prompt));
            state.input.clear();
            match chat_request(state, ctx) {
                Ok(request) => {
                    state.sending = true;
                    let generation = state.generation;
                    Task::perform(
                        ai_assistant::send_ai_assistant_message(request),
                        move |result| {
                            Message::Replied(
                                generation,
                                result.unwrap_or_else(|e| AiAssistantChatResult {
                                    error: Some(e),
                                    ..Default::default()
                                }),
                            )
                        },
                    )
                }
                Err(e) => {
                    state.entries.push(Entry::Failure(e.to_string()));
                    Task::none()
                }
            }
        }
        Message::Replied(generation, result) => {
            if generation != state.generation {
                return Task::none();
            }
            state.sending = false;
            if !result.success {
                state.entries.push(Entry::Failure(
                    result
                        .error
                        .unwrap_or_else(|| "The assistant did not answer. Try again.".into()),
                ));
                return Task::none();
            }
            let cards: Vec<Card> = result
                .actions
                .unwrap_or_default()
                .into_iter()
                .map(|action| Card {
                    action,
                    current: FileState::Loading,
                    progress: Progress::Pending,
                    undo: None,
                })
                .collect();
            let loads: Vec<_> = cards
                .iter()
                .filter_map(|card| match &card.action.kind {
                    AiAssistantActionKind::WriteServerFile { relative_path, .. } => Some((
                        card.action.id.clone(),
                        card.action.server_id.clone(),
                        relative_path.clone(),
                    )),
                    _ => None,
                })
                .collect();
            state.entries.push(Entry::Assistant {
                text: result.reply.unwrap_or_default(),
                cards,
            });
            Task::batch(loads.into_iter().map(|(id, server_id, path)| {
                Task::perform(server_files::read_server_file(server_id, path), move |r| {
                    let current = match (r.content, r.error) {
                        (Some(text), _) => FileState::Text(text),
                        (None, Some(e))
                            if e.contains("cannot find")
                                || e.contains("No such file")
                                || e.contains("os error 2") =>
                        {
                            FileState::Missing
                        }
                        (None, e) => {
                            FileState::Unreadable(e.unwrap_or_else(|| "unknown error".into()))
                        }
                    };
                    Message::FileLoaded(id.clone(), current)
                })
            }))
        }
        Message::FileLoaded(id, current) => {
            if let Some(card) = find_card(state, &id) {
                card.current = current;
            }
            Task::none()
        }
        Message::Apply(id) => {
            let Some(card) = find_card(state, &id) else {
                return Task::none();
            };
            if !matches!(card.progress, Progress::Pending | Progress::Failed(_)) {
                return Task::none();
            }
            card.progress = Progress::Running;
            run_action(&card.action).map(move |r| Message::Applied(id.clone(), r))
        }
        Message::Applied(id, result) => {
            if let Some(card) = find_card(state, &id) {
                match outcome(result) {
                    Ok(r) => {
                        card.progress = Progress::Done(r.message.unwrap_or_else(|| "Done".into()));
                        card.undo = r.undo_action;
                        ctx.refresh_servers();
                    }
                    Err(e) => card.progress = Progress::Failed(e),
                }
            }
            Task::none()
        }
        Message::Dismiss(id) => {
            if let Some(card) = find_card(state, &id) {
                card.progress = Progress::Dismissed;
            }
            Task::none()
        }
        Message::Undo(id) => {
            let Some(card) = find_card(state, &id) else {
                return Task::none();
            };
            let Some(undo) = card.undo.clone() else {
                return Task::none();
            };
            if !matches!(card.progress, Progress::Done(_) | Progress::Failed(_)) {
                return Task::none();
            }
            card.progress = Progress::Running;
            run_action(&undo).map(move |r| Message::Undone(id.clone(), r))
        }
        Message::Undone(id, result) => {
            if let Some(card) = find_card(state, &id) {
                card.progress = match outcome(result) {
                    Ok(_) => {
                        card.undo = None;
                        ctx.refresh_servers();
                        Progress::Done("Change undone".into())
                    }
                    Err(e) => Progress::Failed(format!("Could not undo: {e}")),
                };
            }
            Task::none()
        }
        Message::Clear => {
            state.entries.clear();
            state.sending = false;
            state.generation += 1;
            Task::none()
        }
    }
}

fn field<'a>(label: &'a str, input: Element<'a, Message>) -> Element<'a, Message> {
    column![text(label).size(TEXT_SM).style(style::muted), input]
        .spacing(SPACE_1)
        .into()
}

pub fn view<'a>(state: &'a State, _ctx: &'a Ctx) -> Element<'a, Message> {
    let title = text("Assistant").size(TEXT_XL).font(UI_SEMIBOLD);
    let Some(settings) = &state.settings else {
        return column![
            title,
            text("Loading assistant settings").size(TEXT_MD).style(style::muted),
        ]
        .spacing(SPACE_3)
        .into();
    };

    let providers = row(PROVIDERS.iter().map(|&provider| {
        button(text(provider_label(provider)).size(TEXT_MD).font(UI_MEDIUM))
            .padding([6, 12])
            .style(style::tab(state.provider == provider))
            .on_press(Message::Provider(provider))
            .into()
    }))
    .spacing(SPACE_1);

    let saved_key = settings.has_api_key && state.provider == settings.provider;
    let key_hint = match (&settings.censored_api_key, saved_key) {
        (Some(key), true) => format!("Saved key {key}. Type a new one to replace it."),
        _ if needs_key(state.provider) && settings.has_api_key => format!(
            "Paste your {} API key. The saved key is for {}.",
            provider_label(state.provider),
            provider_label(settings.provider)
        ),
        _ if needs_key(state.provider) => "Paste your API key".to_string(),
        _ => "Optional for local servers".to_string(),
    };

    let mut form = column![
        field("Provider", providers.into()),
        field(
            "Model",
            text_input("gpt-4o-mini", &state.model)
                .on_input(Message::Model)
                .padding([8, 12])
                .style(style::input)
                .into()
        ),
        field(
            "Base URL",
            text_input("https://api.openai.com/v1", &state.base_url)
                .on_input(Message::BaseUrl)
                .padding([8, 12])
                .font(MONO)
                .style(style::input)
                .into()
        ),
        field(
            "API key",
            text_input(&key_hint, &state.api_key)
                .on_input(Message::ApiKey)
                .secure(true)
                .padding([8, 12])
                .style(style::input)
                .into()
        ),
    ]
    .spacing(SPACE_4);

    form = form.push(busy_button(
        "Save",
        "Saving",
        state.saving,
        Some(Message::Save),
        style::primary,
    ));

    let usable = settings.has_api_key || !needs_key(settings.provider);
    let mut switch = toggler(settings.enabled).size(20).style(style::switch);
    if usable && !state.enabling {
        switch = switch.on_toggle(Message::Enabled);
    }
    let toggle = row![
        column![
            text("Enable the assistant").size(TEXT_MD).font(UI_MEDIUM),
            text(if state.enabling {
                "Saving"
            } else if usable {
                "Adds a chat panel next to your servers. Nothing changes until you confirm it."
            } else {
                "Save an API key first, then switch the assistant on."
            })
            .size(TEXT_SM)
            .style(style::muted),
        ]
        .spacing(SPACE_1)
        .width(Length::Fill),
        switch
    ]
    .spacing(SPACE_5)
    .align_y(Alignment::Center);

    let error: Element<Message> = match &state.save_error {
        Some(e) => text(e.as_str()).size(TEXT_SM).color(CORAL).into(),
        None => column![].into(),
    };

    scrollable(
        column![
            title,
            text("Connect any OpenAI-compatible service. Your key stays on this PC and is only sent to the base URL below.")
                .size(TEXT_MD)
                .style(style::muted),
            toggle,
            error,
            form,
        ]
        .spacing(SPACE_5)
        .max_width(520),
    )
    .style(style::scroll)
    .into()
}

fn diff_summary(current: &FileState, new: &str) -> String {
    let old = match current {
        FileState::Loading => return "Checking the current file".into(),
        FileState::Missing => return format!("New file, {} lines", new.lines().count()),
        FileState::Unreadable(e) => {
            return format!("Could not read the current file ({e}). Confirming overwrites it.")
        }
        FileState::Text(old) => old,
    };
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for line in old.lines() {
        *counts.entry(line).or_default() -= 1;
    }
    for line in new.lines() {
        *counts.entry(line).or_default() += 1;
    }
    let added: i64 = counts.values().filter(|n| **n > 0).sum();
    let removed: i64 = -counts.values().filter(|n| **n < 0).sum::<i64>();
    if added == 0 && removed == 0 {
        return if old == new {
            "No changes".into()
        } else {
            "Same lines in a different order".into()
        };
    }
    format!(
        "{added} lines added, {removed} removed, {} lines in total",
        new.lines().count()
    )
}

fn describe(card: &Card) -> (String, Element<'_, Message>) {
    match &card.action.kind {
        AiAssistantActionKind::SetServerProperty { key, value } => (
            "Change a server property".into(),
            text(format!("{key}={value}"))
                .size(TEXT_SM)
                .font(MONO)
                .into(),
        ),
        AiAssistantActionKind::SendServerCommand { command } => (
            "Run a console command".into(),
            text(command.as_str()).size(TEXT_SM).font(MONO).into(),
        ),
        AiAssistantActionKind::WriteServerFile {
            relative_path,
            content,
        } => (
            format!("Overwrite {relative_path} in the server folder"),
            container(column![
                text(diff_summary(&card.current, content))
                    .size(TEXT_SM)
                    .style(style::muted),
                scrollable(text(content.as_str()).size(TEXT_SM).font(MONO))
                    .style(style::scroll)
                    .height(Length::Shrink),
            ]
            .spacing(SPACE_2))
            .max_height(240)
            .into(),
        ),
    }
}

fn card_view<'a>(card: &'a Card, ctx: &'a Ctx) -> Element<'a, Message> {
    let id = card.action.id.clone();
    let (title, detail) = describe(card);
    let target = ctx.server(&card.action.server_id).map_or_else(
        || "Unknown server".to_string(),
        |s| format!("On {}", s.name),
    );

    let footer: Element<Message> = match &card.progress {
        Progress::Pending => row![
            button(text("Confirm").size(TEXT_MD).font(UI_MEDIUM))
                .padding([6, 14])
                .style(style::primary)
                .on_press(Message::Apply(id.clone())),
            button(text("Dismiss").size(TEXT_MD).font(UI_MEDIUM))
                .padding([6, 14])
                .style(style::ghost)
                .on_press(Message::Dismiss(id)),
        ]
        .spacing(SPACE_2)
        .into(),
        Progress::Running if card.undo.is_some() => {
            busy_button::<Message>("Undo", "Undoing", true, None, style::secondary)
        }
        Progress::Running => {
            busy_button::<Message>("Confirm", "Applying", true, None, style::primary)
        }
        Progress::Done(message) => {
            let mut line = row![
                icon_colored(Icon::Check, 16.0, MOSS),
                text(message.as_str()).size(TEXT_SM).width(Length::Fill),
            ]
            .spacing(SPACE_2)
            .align_y(Alignment::Center);
            if card.undo.is_some() {
                line = line.push(
                    button(text("Undo").size(TEXT_SM).font(UI_MEDIUM))
                        .padding([4, 10])
                        .style(style::secondary)
                        .on_press(Message::Undo(id)),
                );
            }
            line.into()
        }
        Progress::Failed(error) => {
            let retry = if card.undo.is_some() {
                Message::Undo(id.clone())
            } else {
                Message::Apply(id.clone())
            };
            row![
                icon_colored(Icon::Error, 16.0, CORAL),
                text(error.as_str())
                    .size(TEXT_SM)
                    .color(CORAL)
                    .width(Length::Fill),
                button(text("Retry").size(TEXT_SM).font(UI_MEDIUM))
                    .padding([4, 10])
                    .style(style::secondary)
                    .on_press(retry),
                button(text("Dismiss").size(TEXT_SM).font(UI_MEDIUM))
                    .padding([4, 10])
                    .style(style::ghost)
                    .on_press(Message::Dismiss(id)),
            ]
            .spacing(SPACE_2)
            .align_y(Alignment::Center)
            .into()
        }
        Progress::Dismissed => text("Dismissed").size(TEXT_SM).style(style::faint).into(),
    };

    container(
        column![
            text(card.action.label.as_str())
                .size(TEXT_MD)
                .font(UI_MEDIUM),
            text(format!("{target}. {title}"))
                .size(TEXT_XS)
                .style(style::muted),
            detail,
            footer,
        ]
        .spacing(SPACE_2),
    )
    .padding(SPACE_3)
    .width(Length::Fill)
    .style(style::panel)
    .into()
}

fn bubble<'a>(entry: &'a Entry, ctx: &'a Ctx) -> Element<'a, Message> {
    match entry {
        Entry::User(content) => container(
            container(text(content.as_str()).size(TEXT_MD))
                .padding([SPACE_2, SPACE_3])
                .style(style::panel),
        )
        .width(Length::Fill)
        .align_right(Length::Fill)
        .into(),
        Entry::Assistant { text: reply, cards } => {
            let mut body = column![].spacing(SPACE_3);
            if !reply.is_empty() {
                body = body.push(text(reply.as_str()).size(TEXT_MD));
            }
            for card in cards {
                body = body.push(card_view(card, ctx));
            }
            body.into()
        }
        Entry::Failure(error) => row![
            icon_colored(Icon::Error, 16.0, CORAL),
            text(error.as_str())
                .size(TEXT_SM)
                .color(CORAL)
                .width(Length::Fill),
        ]
        .spacing(SPACE_2)
        .into(),
    }
}

fn hint<'a>(content: Element<'a, Message>, label: &'a str) -> Element<'a, Message> {
    tooltip(
        content,
        container(text(label).size(TEXT_XS))
            .padding([4, 8])
            .style(style::tooltip),
        tooltip::Position::Bottom,
    )
    .into()
}

pub fn panel<'a>(state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let mut head = row![text("Assistant")
        .size(TEXT_LG)
        .font(UI_SEMIBOLD)
        .width(Length::Fill)]
    .spacing(SPACE_2)
    .align_y(Alignment::Center);
    if !state.entries.is_empty() {
        head = head.push(
            button(text("Clear").size(TEXT_SM).font(UI_MEDIUM))
                .padding([4, 10])
                .style(style::ghost)
                .on_press(Message::Clear),
        );
    }
    head = head.push(hint(
        button(icon(Icon::Close, 16.0))
            .padding(SPACE_2)
            .style(style::icon_button)
            .on_press(Message::Close)
            .into(),
        "Close",
    ));

    let body: Element<Message> = if state.settings.is_none() {
        text("Loading").size(TEXT_MD).style(style::muted).into()
    } else if !configured(state) {
        column![
            text("Set up the assistant").size(TEXT_LG).font(UI_SEMIBOLD),
            text("Add an API key from any OpenAI-compatible service. The assistant can then suggest fixes for your servers, and you approve them one by one.")
                .size(TEXT_MD)
                .style(style::muted),
            button(text("Open assistant settings").size(TEXT_MD).font(UI_MEDIUM))
                .padding([8, 16])
                .style(style::primary)
                .on_press(Message::OpenSettings),
        ]
        .spacing(SPACE_3)
        .into()
    } else {
        let list: Element<Message> = if state.entries.is_empty() {
            let name = ctx.selected_server().map(|s| s.name.as_str());
            column![
                text(match name {
                    Some(name) => format!("Ask about {name}"),
                    None => "Select a server to ask about it".to_string(),
                })
                .size(TEXT_LG)
                .font(UI_SEMIBOLD),
                text("Try \"why did my server crash?\" or \"set the difficulty to hard\". Nothing changes until you confirm it.")
                    .size(TEXT_MD)
                    .style(style::muted),
            ]
            .spacing(SPACE_2)
            .into()
        } else {
            let mut entries = column(state.entries.iter().map(|entry| bubble(entry, ctx))).spacing(SPACE_4);
            if state.sending {
                entries = entries.push(text("Thinking").size(TEXT_MD).style(style::muted));
            }
            scrollable(entries.padding(Padding::ZERO.right(SPACE_3)))
                .anchor_bottom()
                .style(style::scroll)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

        let mut input = text_input("Ask the assistant", &state.input)
            .padding([8, 12])
            .style(style::input);
        if !state.sending {
            input = input.on_input(Message::Input).on_submit(Message::Send);
        }
        let can_send = !state.sending && !state.input.trim().is_empty();
        column![
            container(list).height(Length::Fill),
            row![
                input,
                button(text("Send").size(TEXT_MD).font(UI_MEDIUM))
                    .padding([8, 14])
                    .style(style::primary)
                    .on_press_maybe(can_send.then_some(Message::Send)),
            ]
            .spacing(SPACE_2)
            .align_y(Alignment::Center),
        ]
        .spacing(SPACE_3)
        .into()
    };

    column![head, container(body).height(Length::Fill)]
        .spacing(SPACE_4)
        .padding(SPACE_4)
        .into()
}

pub fn on_event(_state: &mut State, _event: &Event, _ctx: &Ctx) -> Task<Message> {
    Task::none()
}

pub fn on_show(_state: &mut State, _ctx: &Ctx) -> Task<Message> {
    Task::perform(ai_settings::get_ai_assistant_settings(), Message::Loaded)
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::none()
}
