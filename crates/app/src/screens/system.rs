use iced::{Subscription, Task};
use monolith_core::events::Event;

use crate::ctx::Ctx;

#[derive(Debug, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Message {}

pub fn update(_state: &mut State, message: Message, _ctx: &Ctx) -> Task<Message> {
    match message {}
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
