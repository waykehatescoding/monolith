use iced::widget::{container, Container, Space};
use iced::{border, Background, Color, Length};

pub fn status_dot<'a, M: 'a>(color: Color) -> Container<'a, M> {
    container(Space::new())
        .width(Length::Fixed(6.0))
        .height(Length::Fixed(6.0))
        .style(move |_| container::Style {
            background: Some(Background::Color(color)),
            border: border::rounded(3.0),
            ..container::Style::default()
        })
}
