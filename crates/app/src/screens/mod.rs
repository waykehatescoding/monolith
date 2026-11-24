pub mod assistant;
pub mod create;
pub mod onboarding;
pub mod palette;
pub mod remote;
pub mod server;
pub mod settings;
pub mod system;

use iced::{Subscription, Task};
use monolith_core::events::Event;

use crate::ctx::Ctx;
use server::settings as server_settings;
use server::{analytics, backups, console, content, files, overview, players};
use settings::{defaults, general, sharing, templates};

macro_rules! registry {
    ($($field:ident => $variant:ident),* $(,)?) => {
        #[derive(Default)]
        pub struct Screens {
            $(pub $field: $field::State,)*
        }

        #[derive(Clone)]
        pub enum Message {
            $($variant($field::Message),)*
        }

        impl Screens {
            pub fn update(&mut self, message: Message, ctx: &Ctx) -> Task<Message> {
                match message {
                    $(Message::$variant(message) => {
                        $field::update(&mut self.$field, message, ctx).map(Message::$variant)
                    })*
                }
            }

            pub fn on_event(&mut self, event: &Event, ctx: &Ctx) -> Task<Message> {
                Task::batch([
                    $($field::on_event(&mut self.$field, event, ctx).map(Message::$variant),)*
                ])
            }

            pub fn subscription(&self) -> Subscription<Message> {
                Subscription::batch([
                    $($field::subscription(&self.$field).map(Message::$variant),)*
                ])
            }
        }
    };
}

registry! {
    overview => Overview,
    console => Console,
    files => Files,
    players => Players,
    content => Content,
    backups => Backups,
    analytics => Analytics,
    server_settings => ServerSettings,
    create => Create,
    general => General,
    defaults => Defaults,
    sharing => Sharing,
    assistant => Assistant,
    palette => Palette,
    onboarding => Onboarding,
    system => System,
    remote => Remote,
    templates => Templates,
}
