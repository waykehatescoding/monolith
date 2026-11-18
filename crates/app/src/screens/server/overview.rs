use iced::widget::{container, text};
use iced::{Element, Length, Subscription, Task};
use monolith_core::events::Event;
use monolith_theme::style;
use monolith_theme::token::TEXT_LG;

use crate::ctx::Ctx;

#[derive(Debug, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Message {}

pub fn update(_state: &mut State, message: Message, _ctx: &Ctx) -> Task<Message> {
    match message {}
}

pub fn view<'a>(_state: &'a State, _ctx: &'a Ctx) -> Element<'a, Message> {
    container(text("Overview").size(TEXT_LG).style(style::muted))
        .center(Length::Fill)
        .into()
}

pub fn header<'a>(_state: &'a State, ctx: &'a Ctx) -> Element<'a, Message> {
    let name = ctx.selected_server().map(|server| server.name.as_str()).unwrap_or("");
    text(name).size(monolith_theme::token::TEXT_2XL).font(monolith_theme::UI_SEMIBOLD).into()
}

pub fn on_event(_state: &mut State, _event: &Event, _ctx: &Ctx) -> Task<Message> {
    Task::none()
}

pub fn on_show(_state: &mut State, _ctx: &Ctx) -> Task<Message> {
    Task::none()
}

pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::none()
}
