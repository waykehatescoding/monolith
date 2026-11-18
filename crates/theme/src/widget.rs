use crate::font::UI_SEMIBOLD;
use crate::token::{INK_SECONDARY, LAYER_3, TEXT_XS};
use iced::widget::{container, text, Container};
use iced::{alignment, Background, Border, Color, Length};

pub fn server_avatar<'a, M: 'a>(name: &str, ring: Option<Color>) -> Container<'a, M> {
    let initial = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    container(text(initial).size(TEXT_XS).font(UI_SEMIBOLD).color(INK_SECONDARY))
        .width(Length::Fixed(24.0))
        .height(Length::Fixed(24.0))
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .style(move |_| container::Style {
            background: Some(Background::Color(LAYER_3)),
            border: Border {
                color: ring.unwrap_or(Color::TRANSPARENT),
                width: if ring.is_some() { 1.5 } else { 0.0 },
                radius: 7.0.into(),
            },
            ..container::Style::default()
        })
}
