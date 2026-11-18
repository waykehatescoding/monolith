use crate::icon::{icon_colored, Icon};
use crate::style;
use crate::token::{INK, SPACE_2, TEXT_XS, TITLE_BAR_HEIGHT};
use iced::widget::{button, column, container, mouse_area, row, svg, text, tooltip, Button, MouseArea, Row, Space, Tooltip};
use iced::window::Direction;
use iced::{mouse, Color, Element, Length, Theme};

pub fn server_icon<'a, M: 'a>(color: Color, state: impl Into<String>) -> Tooltip<'a, M, Theme> {
    tooltip(
        icon_colored(Icon::Server, 16.0, color),
        container(text(state.into()).size(TEXT_XS).color(INK))
            .padding([4, 8])
            .style(style::tooltip),
        tooltip::Position::Right,
    )
    .gap(SPACE_2 + 4.0)
}

const MINIMIZE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M0 5.5H10" stroke="#fff"/></svg>"##;
const MAXIMIZE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="#fff"/></svg>"##;
const RESTORE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M2.5 2.5V0.5H9.5V7.5H7.5M0.5 2.5H7.5V9.5H0.5Z" fill="none" stroke="#fff"/></svg>"##;
const CLOSE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M0.5 0.5L9.5 9.5M9.5 0.5L0.5 9.5" stroke="#fff"/></svg>"##;

fn window_button<'a, M: Clone + 'a>(
    glyph: &'static [u8],
    message: M,
    style: fn(&Theme, button::Status) -> button::Style,
) -> Button<'a, M, Theme> {
    let glyph = svg(svg::Handle::from_memory(glyph))
        .width(Length::Fixed(10.0))
        .height(Length::Fixed(10.0))
        .style(|_: &Theme, _| svg::Style { color: Some(INK) });
    button(container(glyph).center(Length::Fill))
        .width(Length::Fixed(46.0))
        .height(Length::Fixed(TITLE_BAR_HEIGHT))
        .padding(0)
        .style(style)
        .on_press(message)
}

pub fn window_controls<'a, M: Clone + 'a>(maximized: bool, minimize: M, toggle: M, close: M) -> Row<'a, M, Theme> {
    row![
        window_button(MINIMIZE, minimize, style::window_control),
        window_button(if maximized { RESTORE } else { MAXIMIZE }, toggle, style::window_control),
        window_button(CLOSE, close, style::window_close),
    ]
}

pub fn drag_region<'a, M: Clone + 'a>(
    content: impl Into<Element<'a, M, Theme>>,
    drag: M,
    toggle: M,
) -> MouseArea<'a, M, Theme> {
    mouse_area(content).on_press(drag).on_double_click(toggle)
}

fn grip<'a, M: Clone + 'a>(
    width: Length,
    height: Length,
    direction: Direction,
    on_resize: &impl Fn(Direction) -> M,
) -> Element<'a, M, Theme> {
    let cursor = match direction {
        Direction::North | Direction::South => mouse::Interaction::ResizingVertically,
        Direction::East | Direction::West => mouse::Interaction::ResizingHorizontally,
        Direction::NorthWest | Direction::SouthEast => mouse::Interaction::ResizingDiagonallyDown,
        Direction::NorthEast | Direction::SouthWest => mouse::Interaction::ResizingDiagonallyUp,
    };
    mouse_area(Space::new().width(width).height(height))
        .on_press(on_resize(direction))
        .interaction(cursor)
        .into()
}

pub fn resize_grips<'a, M: Clone + 'a>(on_resize: impl Fn(Direction) -> M) -> Element<'a, M, Theme> {
    let edge = Length::Fixed(5.0);
    let corner = Length::Fixed(10.0);
    column![
        row![
            grip(corner, edge, Direction::NorthWest, &on_resize),
            grip(Length::Fill, edge, Direction::North, &on_resize),
            grip(corner, edge, Direction::NorthEast, &on_resize),
        ],
        row![
            grip(edge, Length::Fill, Direction::West, &on_resize),
            Space::new().width(Length::Fill).height(Length::Fill),
            grip(edge, Length::Fill, Direction::East, &on_resize),
        ]
        .height(Length::Fill),
        row![
            grip(corner, edge, Direction::SouthWest, &on_resize),
            grip(Length::Fill, edge, Direction::South, &on_resize),
            grip(corner, edge, Direction::SouthEast, &on_resize),
        ],
    ]
    .into()
}
