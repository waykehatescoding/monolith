use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Length};
use monolith_theme::token::{CORAL, INK_SECONDARY, MOSS, SPACE_2, SPACE_3, SPACE_5, TEXT_MD};
use monolith_theme::{icon, icon_colored, style, Icon};

use crate::ctx::ToastKind;

#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub text: String,
}

pub fn view<'a, M: Clone + 'a>(toasts: &'a [Toast], dismiss: impl Fn(u64) -> M) -> Element<'a, M> {
    let list = toasts
        .iter()
        .fold(column![].spacing(SPACE_2), |list, toast| {
            let glyph = match toast.kind {
                ToastKind::Success => icon_colored(Icon::Check, 16.0, MOSS),
                ToastKind::Error => icon_colored(Icon::Error, 16.0, CORAL),
                ToastKind::Info => icon_colored(Icon::Info, 16.0, INK_SECONDARY),
            };
            list.push(
                container(
                    row![
                        glyph,
                        text(&toast.text).size(TEXT_MD).width(Length::Fill),
                        button(icon(Icon::Close, 16.0))
                            .width(28)
                            .height(28)
                            .padding(6)
                            .style(style::icon_button)
                            .on_press(dismiss(toast.id)),
                    ]
                    .spacing(SPACE_3)
                    .align_y(Alignment::Center),
                )
                .padding([SPACE_2, SPACE_3])
                .width(360)
                .style(style::overlay),
            )
        });
    container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(SPACE_5)
        .align_x(Alignment::End)
        .align_y(Alignment::End)
        .into()
}
