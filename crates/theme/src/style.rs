use iced::widget::overlay::menu;
use iced::widget::slider::{Handle, HandleShape, Rail};
use iced::widget::{
    button, checkbox, container, pick_list, progress_bar, rule, scrollable, slider, text,
    text_input, toggler,
};
use iced::{border, Background, Border, Color, Shadow, Theme, Vector};

use crate::token::*;

fn edge(color: Color, width: f32, radius: f32) -> Border {
    Border {
        color,
        width,
        radius: border::radius(radius),
    }
}

fn fill(color: Color) -> Option<Background> {
    Some(Background::Color(color))
}

fn dim(color: Color) -> Color {
    color.scale_alpha(0.5)
}

fn button_style(background: Option<Background>, text_color: Color) -> button::Style {
    button::Style {
        background,
        text_color,
        border: edge(Color::TRANSPARENT, 0.0, RADIUS_CONTROL),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active => button_style(fill(MOSS), GROUND),
        button::Status::Hovered => button_style(fill(MOSS_HOVER), GROUND),
        button::Status::Pressed => button_style(fill(MOSS_PRESSED), GROUND),
        button::Status::Disabled => button_style(fill(MOSS.scale_alpha(0.22)), dim(MOSS)),
    }
}

pub fn secondary(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active => button_style(fill(LAYER_3), INK),
        button::Status::Hovered => button_style(fill(LAYER_3_HOVER), INK),
        button::Status::Pressed => button_style(fill(LAYER_2), INK),
        button::Status::Disabled => button_style(fill(LAYER_2), INK_TERTIARY),
    }
}

pub fn ghost(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active => button_style(None, INK_SECONDARY),
        button::Status::Hovered => button_style(fill(TONE_HOVER), INK),
        button::Status::Pressed => button_style(fill(TONE_PRESSED), INK),
        button::Status::Disabled => button_style(None, INK_TERTIARY),
    }
}

pub fn icon_button(theme: &Theme, status: button::Status) -> button::Style {
    ghost(theme, status)
}

pub fn danger(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active => button_style(None, CORAL),
        button::Status::Hovered => button_style(fill(CORAL.scale_alpha(0.14)), CORAL_HOVER),
        button::Status::Pressed => button_style(fill(CORAL.scale_alpha(0.24)), CORAL_HOVER),
        button::Status::Disabled => button_style(None, dim(CORAL)),
    }
}

pub fn sidebar_item(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| match (selected, status) {
        (_, button::Status::Disabled) => button_style(None, INK_TERTIARY),
        (true, button::Status::Pressed) => button_style(fill(LAYER_3), INK),
        (true, _) => button_style(fill(LAYER_2), INK),
        (false, button::Status::Active) => button_style(None, INK_SECONDARY),
        (false, button::Status::Hovered) => button_style(fill(TONE_HOVER), INK),
        (false, button::Status::Pressed) => button_style(fill(TONE_PRESSED), INK),
    }
}

pub fn tab(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| match (selected, status) {
        (_, button::Status::Disabled) => button_style(None, INK_TERTIARY),
        (true, button::Status::Pressed) => button_style(fill(LAYER_3), INK),
        (true, _) => button_style(fill(LAYER_2), INK),
        (false, button::Status::Active) => button_style(None, INK_SECONDARY),
        (false, button::Status::Hovered) => button_style(fill(TONE_HOVER), INK),
        (false, button::Status::Pressed) => button_style(fill(TONE_PRESSED), INK),
    }
}

pub fn focus_ring(inner: button::Style) -> button::Style {
    button::Style {
        border: edge(MOSS.scale_alpha(0.55), FOCUS_WIDTH, RADIUS_CONTROL),
        ..inner
    }
}

fn field(status: text_input::Status, rest: Color, hover: Color, focus: Color) -> text_input::Style {
    let border = match status {
        text_input::Status::Active => edge(rest, BORDER, RADIUS_CONTROL),
        text_input::Status::Hovered => edge(hover, BORDER, RADIUS_CONTROL),
        text_input::Status::Focused { .. } => edge(focus, FOCUS_WIDTH, RADIUS_CONTROL),
        text_input::Status::Disabled => edge(Color::TRANSPARENT, 0.0, RADIUS_CONTROL),
    };
    let disabled = status == text_input::Status::Disabled;
    text_input::Style {
        background: Background::Color(if disabled { LAYER_1 } else { LAYER_2 }),
        border,
        icon: INK_SECONDARY,
        placeholder: INK_TERTIARY,
        value: if disabled { INK_TERTIARY } else { INK },
        selection: MOSS.scale_alpha(0.28),
    }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    field(status, HAIRLINE, HAIRLINE_STRONG, MOSS.scale_alpha(0.7))
}

pub fn input_error(_: &Theme, status: text_input::Status) -> text_input::Style {
    field(status, CORAL.scale_alpha(0.7), CORAL, CORAL)
}

pub fn pick(_: &Theme, status: pick_list::Status) -> pick_list::Style {
    let border = match status {
        pick_list::Status::Active => edge(HAIRLINE, BORDER, RADIUS_CONTROL),
        pick_list::Status::Hovered => edge(HAIRLINE_STRONG, BORDER, RADIUS_CONTROL),
        pick_list::Status::Opened { .. } => {
            edge(MOSS.scale_alpha(0.7), FOCUS_WIDTH, RADIUS_CONTROL)
        }
    };
    pick_list::Style {
        text_color: INK,
        placeholder_color: INK_TERTIARY,
        handle_color: INK_SECONDARY,
        background: Background::Color(LAYER_2),
        border,
    }
}

pub fn pick_menu(_: &Theme) -> menu::Style {
    menu::Style {
        background: Background::Color(LAYER_3),
        border: edge(HAIRLINE, BORDER, RADIUS_PANEL),
        text_color: INK,
        selected_text_color: INK,
        selected_background: Background::Color(TONE_PRESSED),
        shadow: overlay_shadow(),
    }
}

pub fn check(_: &Theme, status: checkbox::Status) -> checkbox::Style {
    let (checked, hovered, disabled) = match status {
        checkbox::Status::Active { is_checked } => (is_checked, false, false),
        checkbox::Status::Hovered { is_checked } => (is_checked, true, false),
        checkbox::Status::Disabled { is_checked } => (is_checked, false, true),
    };
    let (bg, line) = match (checked, hovered, disabled) {
        (true, _, true) => (LAYER_3, Color::TRANSPARENT),
        (true, true, _) => (MOSS_HOVER, Color::TRANSPARENT),
        (true, false, _) => (MOSS, Color::TRANSPARENT),
        (false, _, true) => (LAYER_1, HAIRLINE),
        (false, true, _) => (LAYER_2, HAIRLINE_STRONG),
        (false, false, _) => (LAYER_2, HAIRLINE_STRONG.scale_alpha(0.7)),
    };
    checkbox::Style {
        background: Background::Color(bg),
        icon_color: if disabled { INK_TERTIARY } else { GROUND },
        border: edge(line, BORDER, 4.0),
        text_color: Some(if disabled { INK_TERTIARY } else { INK }),
    }
}

pub fn switch(_: &Theme, status: toggler::Status) -> toggler::Style {
    let (on, hovered, disabled) = match status {
        toggler::Status::Active { is_toggled } => (is_toggled, false, false),
        toggler::Status::Hovered { is_toggled } => (is_toggled, true, false),
        toggler::Status::Disabled { is_toggled } => (is_toggled, false, true),
    };
    let (track, knob) = match (on, hovered, disabled) {
        (true, _, true) => (MOSS.scale_alpha(0.25), dim(INK)),
        (false, _, true) => (LAYER_2, INK_TERTIARY),
        (true, true, _) => (MOSS_HOVER, GROUND),
        (true, false, _) => (MOSS, GROUND),
        (false, true, _) => (LAYER_3_HOVER, INK),
        (false, false, _) => (LAYER_3, INK_SECONDARY),
    };
    toggler::Style {
        background: Background::Color(track),
        background_border_width: 0.0,
        background_border_color: Color::TRANSPARENT,
        foreground: Background::Color(knob),
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
        text_color: Some(if disabled { INK_TERTIARY } else { INK }),
        border_radius: Some(border::radius(20.0)),
        padding_ratio: 0.18,
    }
}

pub fn range(_: &Theme, status: slider::Status) -> slider::Style {
    let handle = match status {
        slider::Status::Active => INK,
        slider::Status::Hovered => MOSS_HOVER,
        slider::Status::Dragged => MOSS,
    };
    slider::Style {
        rail: Rail {
            backgrounds: (Background::Color(MOSS), Background::Color(LAYER_3)),
            width: 4.0,
            border: edge(Color::TRANSPARENT, 0.0, 2.0),
        },
        handle: Handle {
            shape: HandleShape::Circle { radius: 7.0 },
            background: Background::Color(handle),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

pub fn scroll(_: &Theme, status: scrollable::Status) -> scrollable::Style {
    let scroller = match status {
        scrollable::Status::Active { .. } => Color::from_rgba(1.0, 1.0, 1.0, 0.10),
        scrollable::Status::Hovered { .. } => Color::from_rgba(1.0, 1.0, 1.0, 0.20),
        scrollable::Status::Dragged { .. } => Color::from_rgba(1.0, 1.0, 1.0, 0.32),
    };
    let rail = scrollable::Rail {
        background: None,
        border: edge(Color::TRANSPARENT, 0.0, 3.0),
        scroller: scrollable::Scroller {
            background: Background::Color(scroller),
            border: edge(Color::TRANSPARENT, 0.0, 3.0),
        },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: Background::Color(LAYER_3),
            border: edge(HAIRLINE, BORDER, RADIUS_PANEL),
            shadow: Shadow::default(),
            icon: INK,
        },
    }
}

fn bar(color: Color) -> progress_bar::Style {
    progress_bar::Style {
        background: Background::Color(LAYER_3),
        bar: Background::Color(color),
        border: edge(Color::TRANSPARENT, 0.0, 3.0),
    }
}

pub fn progress(_: &Theme) -> progress_bar::Style {
    bar(MOSS)
}

pub fn progress_warn(_: &Theme) -> progress_bar::Style {
    bar(AMBER)
}

pub fn progress_danger(_: &Theme) -> progress_bar::Style {
    bar(CORAL)
}

pub fn hairline(_: &Theme) -> rule::Style {
    rule::Style {
        color: HAIRLINE,
        radius: border::radius(0.0),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

fn layer(background: Color, radius: f32) -> container::Style {
    container::Style {
        background: fill(background),
        text_color: Some(INK),
        border: edge(Color::TRANSPARENT, 0.0, radius),
        ..container::Style::default()
    }
}

pub fn ground(_: &Theme) -> container::Style {
    layer(GROUND, 0.0)
}

pub fn surface(_: &Theme) -> container::Style {
    layer(LAYER_1, 0.0)
}

pub fn panel(_: &Theme) -> container::Style {
    layer(LAYER_2, RADIUS_PANEL)
}

pub fn well(_: &Theme) -> container::Style {
    layer(GROUND, RADIUS_PANEL)
}

pub fn console(theme: &Theme) -> container::Style {
    well(theme)
}

pub fn overlay(_: &Theme) -> container::Style {
    container::Style {
        shadow: overlay_shadow(),
        border: edge(HAIRLINE, BORDER, RADIUS_OVERLAY),
        ..layer(LAYER_3, RADIUS_OVERLAY)
    }
}

pub fn tooltip(_: &Theme) -> container::Style {
    container::Style {
        shadow: overlay_shadow(),
        ..layer(LAYER_3, RADIUS_CONTROL)
    }
}

pub fn scrim(_: &Theme) -> container::Style {
    container::Style {
        background: fill(Color::from_rgba(0.0, 0.0, 0.0, 0.5)),
        ..container::Style::default()
    }
}

fn overlay_shadow() -> Shadow {
    Shadow {
        color: Color::from_rgba8(0, 0, 0, 0.5),
        offset: Vector::new(0.0, 12.0),
        blur_radius: 32.0,
    }
}

pub fn muted(_: &Theme) -> text::Style {
    text::Style {
        color: Some(INK_SECONDARY),
    }
}

pub fn faint(_: &Theme) -> text::Style {
    text::Style {
        color: Some(INK_TERTIARY),
    }
}

pub fn colored(color: Color) -> impl Fn(&Theme) -> text::Style {
    move |_| text::Style { color: Some(color) }
}

pub fn link(_: &Theme) -> text::Style {
    text::Style { color: Some(MOSS) }
}

fn square_button(background: Option<Background>, text_color: Color) -> button::Style {
    button::Style {
        background,
        text_color,
        border: Border::default(),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn window_control(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active | button::Status::Disabled => square_button(None, INK_SECONDARY),
        button::Status::Hovered => square_button(fill(TONE_HOVER), INK),
        button::Status::Pressed => square_button(fill(TONE_PRESSED), INK),
    }
}

pub fn window_close(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active | button::Status::Disabled => square_button(None, INK_SECONDARY),
        button::Status::Hovered => square_button(fill(CORAL), INK),
        button::Status::Pressed => square_button(fill(CORAL_HOVER), INK),
    }
}
