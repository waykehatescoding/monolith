use iced::widget::{container, mouse_area, opaque, stack, Space};
use iced::{Element, Length, Padding};
use monolith_theme::style;

pub fn modal<'a, M: Clone + 'a>(
    content: impl Into<Element<'a, M>>,
    on_dismiss: M,
    top_offset: Option<f32>,
) -> Element<'a, M> {
    let scrim = mouse_area(
        container(Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(style::scrim),
    )
    .on_press(on_dismiss);
    let placed = match top_offset {
        Some(top) => container(opaque(content))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .padding(Padding {
                top,
                ..Padding::ZERO
            }),
        None => container(opaque(content)).center(Length::Fill),
    };
    stack![scrim, placed].into()
}
