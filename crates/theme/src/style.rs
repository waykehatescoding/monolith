use iced::widget::overlay::menu;
use iced::widget::slider::{Handle, HandleShape, Rail};
use iced::widget::{
    button, checkbox, container, pick_list, progress_bar, rule, scrollable, slider, text,
    text_input, toggler,
};
use iced::{border, Background, Border, Color, Shadow, Theme, Vector};

use crate::token::*;

fn edge(color: Color, width: f32) -> Border {
    Border {
        color,
        width,
        radius: border::radius(RADIUS),
    }
}

fn fill(color: Color) -> Option<Background> {
    Some(Background::Color(color))
}

fn dim(color: Color) -> Color {
    color.scale_alpha(0.45)
}

fn button_style(
    background: Option<Background>,
    text_color: Color,
    line: Color,
) -> button::Style {
    button::Style {
        background,
        text_color,
        border: edge(line, BORDER),
        shadow: Shadow::default(),
        snap: true,
    }
}

fn solid(
    status: button::Status,
    rest: Color,
    hover: Color,
    press: Color,
    ink: Color,
) -> button::Style {
    match status {
        button::Status::Active => button_style(fill(rest), ink, Color::TRANSPARENT),
        button::Status::Hovered => button_style(fill(hover), ink, Color::TRANSPARENT),
        button::Status::Pressed => button_style(fill(press), ink, Color::TRANSPARENT),
        button::Status::Disabled => button_style(fill(STRATUM), dim(INK_MUTED), LINE_SOFT),
    }
}

pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    solid(status, GRASS, GRASS_HOVER, GRASS_PRESSED, GROUND)
}

pub fn danger(_: &Theme, status: button::Status) -> button::Style {
    solid(status, REDSTONE, REDSTONE_HOVER, REDSTONE_PRESSED, GROUND)
}

pub fn secondary(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active => button_style(fill(STRATUM_HIGH), INK, LINE),
        button::Status::Hovered => {
            button_style(fill(Color::from_rgb8(0x36, 0x3e, 0x47)), INK, STONE)
        }
        button::Status::Pressed => button_style(fill(STRATUM), INK, STONE),
        button::Status::Disabled => button_style(fill(STRATUM), dim(INK_MUTED), LINE_SOFT),
    }
}

pub fn ghost(_: &Theme, status: button::Status) -> button::Style {
    let none = Color::TRANSPARENT;
    match status {
        button::Status::Active => button_style(None, INK_MUTED, none),
        button::Status::Hovered => button_style(fill(STRATUM_HIGH), INK, none),
        button::Status::Pressed => button_style(fill(STRATUM), INK, none),
        button::Status::Disabled => button_style(None, dim(INK_MUTED), none),
    }
}

pub fn icon_button(theme: &Theme, status: button::Status) -> button::Style {
    ghost(theme, status)
}

pub fn danger_ghost(_: &Theme, status: button::Status) -> button::Style {
    let none = Color::TRANSPARENT;
    match status {
        button::Status::Active => button_style(None, REDSTONE, none),
        button::Status::Hovered => {
            button_style(fill(REDSTONE.scale_alpha(0.16)), REDSTONE_HOVER, none)
        }
        button::Status::Pressed => {
            button_style(fill(REDSTONE.scale_alpha(0.28)), REDSTONE_HOVER, none)
        }
        button::Status::Disabled => button_style(None, dim(REDSTONE), none),
    }
}

pub fn nav(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let none = Color::TRANSPARENT;
        match (selected, status) {
            (_, button::Status::Disabled) => button_style(None, dim(INK_MUTED), none),
            (true, button::Status::Pressed) => button_style(fill(STRATUM), INK, none),
            (true, _) => button_style(fill(STRATUM_HIGH), INK, none),
            (false, button::Status::Active) => button_style(None, INK_MUTED, none),
            (false, button::Status::Hovered) => button_style(fill(STRATUM), INK, none),
            (false, button::Status::Pressed) => button_style(fill(STRATUM_HIGH), INK, none),
        }
    }
}

pub fn tab(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let none = Color::TRANSPARENT;
        match (selected, status) {
            (_, button::Status::Disabled) => button_style(None, dim(INK_MUTED), LINE_SOFT),
            (true, _) => button_style(fill(STRATUM_HIGH), INK, LINE),
            (false, button::Status::Active) => button_style(None, INK_MUTED, none),
            (false, button::Status::Hovered) => button_style(fill(STRATUM), INK, LINE_SOFT),
            (false, button::Status::Pressed) => button_style(fill(STRATUM_HIGH), INK, LINE_SOFT),
        }
    }
}

pub fn tile(focused: bool, dimmed: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let (bg, line) = match status {
            button::Status::Hovered => (STRATUM_HIGH, STONE),
            button::Status::Pressed => (STRATUM, STONE),
            _ if focused => (STRATUM_HIGH, INK),
            _ => (STRATUM, LINE),
        };
        let fade = if dimmed && status == button::Status::Active { 0.45 } else { 1.0 };
        button_style(fill(bg.scale_alpha(fade)), INK.scale_alpha(fade), line.scale_alpha(fade))
    }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    input_with(status, LINE, STONE)
}

pub fn input_error(_: &Theme, status: text_input::Status) -> text_input::Style {
    input_with(status, REDSTONE, REDSTONE_HOVER)
}

fn input_with(status: text_input::Status, rest: Color, hover: Color) -> text_input::Style {
    let (bg, border) = match status {
        text_input::Status::Active => (WELL, edge(rest, BORDER)),
        text_input::Status::Hovered => (WELL, edge(hover, BORDER)),
        text_input::Status::Focused { .. } => (WELL, edge(WATER_LIGHT, FOCUS_WIDTH)),
        text_input::Status::Disabled => (STRATUM, edge(LINE_SOFT, BORDER)),
    };
    let disabled = status == text_input::Status::Disabled;
    text_input::Style {
        background: Background::Color(bg),
        border,
        icon: INK_MUTED,
        placeholder: if disabled { dim(INK_MUTED) } else { PLACEHOLDER },
        value: if disabled { dim(INK) } else { INK },
        selection: WATER.scale_alpha(0.5),
    }
}

pub fn pick(_: &Theme, status: pick_list::Status) -> pick_list::Style {
    let border = match status {
        pick_list::Status::Active => edge(LINE, BORDER),
        pick_list::Status::Hovered => edge(STONE, BORDER),
        pick_list::Status::Opened { .. } => edge(WATER_LIGHT, FOCUS_WIDTH),
    };
    pick_list::Style {
        text_color: INK,
        placeholder_color: PLACEHOLDER,
        handle_color: INK_MUTED,
        background: Background::Color(WELL),
        border,
    }
}

pub fn pick_menu(_: &Theme) -> menu::Style {
    menu::Style {
        background: Background::Color(STRATUM_HIGH),
        border: edge(LINE, BORDER),
        text_color: INK,
        selected_text_color: GROUND,
        selected_background: Background::Color(GRASS),
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
        (true, _, true) => (STRATUM_HIGH, LINE),
        (true, true, _) => (GRASS_HOVER, GRASS_HOVER),
        (true, false, _) => (GRASS, GRASS),
        (false, _, true) => (STRATUM, LINE_SOFT),
        (false, true, _) => (WELL, STONE),
        (false, false, _) => (WELL, LINE),
    };
    checkbox::Style {
        background: Background::Color(bg),
        icon_color: if disabled { dim(INK_MUTED) } else { GROUND },
        border: edge(line, BORDER),
        text_color: Some(if disabled { dim(INK_MUTED) } else { INK }),
    }
}

pub fn switch(_: &Theme, status: toggler::Status) -> toggler::Style {
    let (on, hovered, disabled) = match status {
        toggler::Status::Active { is_toggled } => (is_toggled, false, false),
        toggler::Status::Hovered { is_toggled } => (is_toggled, true, false),
        toggler::Status::Disabled { is_toggled } => (is_toggled, false, true),
    };
    let (track, knob) = match (on, hovered, disabled) {
        (_, _, true) => (STRATUM_HIGH, dim(INK_MUTED)),
        (true, true, _) => (GRASS_HOVER, GROUND),
        (true, false, _) => (GRASS, GROUND),
        (false, true, _) => (Color::from_rgb8(0x36, 0x3e, 0x47), INK),
        (false, false, _) => (STRATUM_HIGH, INK_MUTED),
    };
    toggler::Style {
        background: Background::Color(track),
        background_border_width: BORDER,
        background_border_color: if on && !disabled { track } else { LINE },
        foreground: Background::Color(knob),
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
        text_color: Some(if disabled { dim(INK_MUTED) } else { INK }),
        border_radius: Some(border::radius(RADIUS)),
        padding_ratio: 0.2,
    }
}

pub fn range(_: &Theme, status: slider::Status) -> slider::Style {
    let handle = match status {
        slider::Status::Active => INK,
        slider::Status::Hovered => WATER_LIGHT,
        slider::Status::Dragged => GRASS,
    };
    slider::Style {
        rail: Rail {
            backgrounds: (Background::Color(GRASS), Background::Color(STRATUM_HIGH)),
            width: 4.0,
            border: edge(Color::TRANSPARENT, 0.0),
        },
        handle: Handle {
            shape: HandleShape::Rectangle {
                width: 8,
                border_radius: border::radius(RADIUS),
            },
            background: Background::Color(handle),
            border_width: BORDER,
            border_color: GROUND,
        },
    }
}

pub fn scroll(_: &Theme, status: scrollable::Status) -> scrollable::Style {
    let scroller = match status {
        scrollable::Status::Active { .. } => LINE,
        scrollable::Status::Hovered {
            is_vertical_scrollbar_hovered,
            is_horizontal_scrollbar_hovered,
            ..
        } => {
            if is_vertical_scrollbar_hovered || is_horizontal_scrollbar_hovered {
                INK_MUTED
            } else {
                STONE
            }
        }
        scrollable::Status::Dragged { .. } => INK,
    };
    let rail = scrollable::Rail {
        background: None,
        border: edge(Color::TRANSPARENT, 0.0),
        scroller: scrollable::Scroller {
            background: Background::Color(scroller),
            border: edge(Color::TRANSPARENT, 0.0),
        },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: Background::Color(STRATUM_HIGH),
            border: edge(LINE, BORDER),
            shadow: Shadow::default(),
            icon: INK,
        },
    }
}

fn bar(color: Color) -> progress_bar::Style {
    progress_bar::Style {
        background: Background::Color(STRATUM_HIGH),
        bar: Background::Color(color),
        border: edge(Color::TRANSPARENT, 0.0),
    }
}

pub fn progress(_: &Theme) -> progress_bar::Style {
    bar(GRASS)
}

pub fn progress_warn(_: &Theme) -> progress_bar::Style {
    bar(SAND)
}

pub fn progress_danger(_: &Theme) -> progress_bar::Style {
    bar(REDSTONE)
}

pub fn hairline(_: &Theme) -> rule::Style {
    rule::Style {
        color: LINE_SOFT,
        radius: border::radius(0.0),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

fn panel(background: Color, line: Color) -> container::Style {
    container::Style {
        background: fill(background),
        text_color: Some(INK),
        border: edge(line, BORDER),
        ..container::Style::default()
    }
}

pub fn ground(_: &Theme) -> container::Style {
    container::Style {
        background: fill(GROUND),
        text_color: Some(INK),
        ..container::Style::default()
    }
}

pub fn stratum(_: &Theme) -> container::Style {
    panel(STRATUM, LINE_SOFT)
}

pub fn rail(_: &Theme) -> container::Style {
    panel(STRATUM, LINE_SOFT)
}

pub fn module(_: &Theme) -> container::Style {
    panel(STRATUM, LINE)
}

pub fn module_tab(_: &Theme) -> container::Style {
    panel(STRATUM_HIGH, LINE)
}

pub fn well(_: &Theme) -> container::Style {
    panel(WELL, LINE_SOFT)
}

pub fn console(_: &Theme) -> container::Style {
    panel(WELL, LINE_SOFT)
}

pub fn tile_card(focused: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        if focused {
            panel(STRATUM_HIGH, INK)
        } else {
            panel(STRATUM, LINE)
        }
    }
}

pub fn state_band(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: fill(color),
        ..container::Style::default()
    }
}

pub fn overlay(_: &Theme) -> container::Style {
    container::Style {
        shadow: overlay_shadow(),
        ..panel(STRATUM_HIGH, LINE)
    }
}

pub fn tooltip(theme: &Theme) -> container::Style {
    overlay(theme)
}

fn overlay_shadow() -> Shadow {
    Shadow {
        color: Color::from_rgba8(0, 0, 0, 0.55),
        offset: Vector::new(0.0, 6.0),
        blur_radius: 16.0,
    }
}

pub fn muted(_: &Theme) -> text::Style {
    text::Style {
        color: Some(INK_MUTED),
    }
}

pub fn colored(color: Color) -> impl Fn(&Theme) -> text::Style {
    move |_| text::Style { color: Some(color) }
}

pub fn link(_: &Theme) -> text::Style {
    text::Style {
        color: Some(WATER_LIGHT),
    }
}
