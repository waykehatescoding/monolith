use iced::widget::{column, container, row, text, Container};
use iced::{Color, Element, Length, Padding};

use crate::{style, token::*, UI_SEMIBOLD};

pub fn module<'a, M: 'a>(title: &'a str, body: impl Into<Element<'a, M>>) -> Element<'a, M> {
    let tab = container(text(title).size(TEXT_SM).font(UI_SEMIBOLD))
        .padding(Padding::from([SPACE_1, SPACE_3]))
        .style(style::module_tab);
    column![
        tab,
        container(body)
            .padding(CHUNK)
            .width(Length::Fill)
            .style(style::module)
    ]
    .into()
}

pub fn chunk_tile<'a, M: 'a>(
    state: Color,
    focused: bool,
    body: impl Into<Element<'a, M>>,
) -> Container<'a, M> {
    container(column![
        container(row![])
            .width(Length::Fill)
            .height(4)
            .style(style::state_band(state)),
        container(body).padding(CHUNK).width(Length::Fill)
    ])
    .style(style::tile_card(focused))
}
