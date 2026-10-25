use iced::{
    Border, Color, Font, Theme,
    widget::{
        button, checkbox, container, overlay::menu, pick_list, rule, scrollable, slider, text,
        text_input,
    },
};

pub(super) const FONT: Font = Font::with_name("Source Serif 4");
pub(super) const FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/SourceSerif4-Regular.otf");
pub(super) const CORNER_RADIUS: f32 = 6.;

pub(super) fn theme(dark: bool) -> Theme {
    Theme::custom(
        "wisp",
        iced::theme::Palette {
            background: if dark {
                Color::from_rgb8(22, 22, 21)
            } else {
                Color::from_rgb8(250, 249, 246)
            },
            text: if dark {
                Color::from_rgb8(224, 222, 216)
            } else {
                Color::from_rgb8(41, 40, 37)
            },
            primary: if dark {
                Color::from_rgb8(224, 222, 216)
            } else {
                Color::from_rgb8(41, 40, 37)
            },
            success: Color::from_rgb8(115, 150, 125),
            warning: Color::from_rgb8(180, 149, 100),
            danger: if dark {
                Color::from_rgb8(208, 143, 134)
            } else {
                Color::from_rgb8(155, 65, 61)
            },
        },
    )
}

fn blend(theme: &Theme, amount: f32) -> Color {
    let p = theme.palette();
    Color::from_rgb(
        p.background.r + (p.text.r - p.background.r) * amount,
        p.background.g + (p.text.g - p.background.g) * amount,
        p.background.b + (p.text.b - p.background.b) * amount,
    )
}

pub(super) fn muted(theme: &Theme) -> Color {
    blend(theme, 0.65)
}
pub(super) fn line(theme: &Theme) -> Color {
    blend(theme, 0.16)
}
pub(super) fn surface(theme: &Theme) -> Color {
    blend(theme, 0.035)
}

pub(super) fn error(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.palette().danger),
    }
}

fn border(theme: &Theme) -> Border {
    Border {
        radius: CORNER_RADIUS.into(),
        width: 1.,
        color: line(theme),
    }
}

pub(super) fn quiet(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::Style {
        text_color: theme.palette().text,
        border: Border {
            width: 0.,
            ..border(theme)
        },
        ..Default::default()
    };
    match status {
        button::Status::Hovered => style.background = Some(blend(theme, 0.06).into()),
        button::Status::Pressed => style.background = Some(blend(theme, 0.1).into()),
        button::Status::Disabled => style.text_color = blend(theme, 0.3),
        button::Status::Active => {}
    }
    style
}

pub(super) fn secondary(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = quiet(theme, status);
    style.border = border(theme);
    if status == button::Status::Disabled {
        style.border.color = blend(theme, 0.08);
    }
    style
}

pub(super) fn primary(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = secondary(theme, status);
    style.background = Some(
        blend(
            theme,
            match status {
                button::Status::Disabled => 0.09,
                button::Status::Hovered => 0.85,
                button::Status::Pressed => 0.75,
                button::Status::Active => 1.,
            },
        )
        .into(),
    );
    if status != button::Status::Disabled {
        style.text_color = theme.palette().background;
    }
    style.border.width = 0.;
    style
}

pub(super) fn selected(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = quiet(theme, status);
    style.background = Some(
        blend(
            theme,
            match status {
                button::Status::Active => 0.08,
                button::Status::Hovered => 0.12,
                button::Status::Pressed => 0.16,
                button::Status::Disabled => 0.04,
            },
        )
        .into(),
    );
    style.border = border(theme);
    style
}

pub(super) fn panel(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.into()),
        border: border(theme),
        ..Default::default()
    }
}

pub(super) fn stage(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(surface(theme).into()),
        border: border(theme),
        ..Default::default()
    }
}

pub(super) fn divider(theme: &Theme) -> rule::Style {
    rule::Style {
        color: line(theme),
        snap: true,
        radius: 0.into(),
        fill_mode: rule::FillMode::Full,
    }
}

pub(super) fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let disabled = status == text_input::Status::Disabled;
    let mut border = border(theme);
    border.color = match status {
        text_input::Status::Focused { .. } => muted(theme),
        text_input::Status::Hovered => blend(theme, 0.35),
        _ => line(theme),
    };
    text_input::Style {
        background: if disabled {
            surface(theme)
        } else {
            theme.palette().background
        }
        .into(),
        border,
        icon: muted(theme),
        placeholder: muted(theme),
        value: if disabled {
            muted(theme)
        } else {
            theme.palette().text
        },
        selection: blend(theme, 0.2),
    }
}

pub(super) fn choice(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let mut border = border(theme);
    if status != pick_list::Status::Active {
        border.color = muted(theme);
    }
    pick_list::Style {
        text_color: theme.palette().text,
        placeholder_color: muted(theme),
        handle_color: muted(theme),
        background: theme.palette().background.into(),
        border,
    }
}

pub(super) fn menu(theme: &Theme) -> menu::Style {
    menu::Style {
        background: theme.palette().background.into(),
        border: border(theme),
        text_color: theme.palette().text,
        selected_text_color: theme.palette().text,
        selected_background: blend(theme, 0.08).into(),
        shadow: Default::default(),
    }
}

pub(super) fn check(theme: &Theme, status: checkbox::Status) -> checkbox::Style {
    let (checked, disabled, hovered) = match status {
        checkbox::Status::Active { is_checked } => (is_checked, false, false),
        checkbox::Status::Hovered { is_checked } => (is_checked, false, true),
        checkbox::Status::Disabled { is_checked } => (is_checked, true, false),
    };
    let ink = if disabled {
        blend(theme, 0.3)
    } else {
        theme.palette().text
    };
    checkbox::Style {
        background: if checked {
            ink
        } else if hovered {
            surface(theme)
        } else {
            theme.palette().background
        }
        .into(),
        icon_color: theme.palette().background,
        border: Border {
            radius: 3.into(),
            width: 1.,
            color: if checked || hovered {
                ink
            } else {
                blend(theme, 0.35)
            },
        },
        text_color: Some(if disabled {
            muted(theme)
        } else {
            theme.palette().text
        }),
    }
}

pub(super) fn scroll(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let mut style = scrollable::default(theme, status);
    let (horizontal_active, vertical_active) = match status {
        scrollable::Status::Active { .. } => (false, false),
        scrollable::Status::Hovered {
            is_horizontal_scrollbar_hovered,
            is_vertical_scrollbar_hovered,
            ..
        } => (
            is_horizontal_scrollbar_hovered,
            is_vertical_scrollbar_hovered,
        ),
        scrollable::Status::Dragged {
            is_horizontal_scrollbar_dragged,
            is_vertical_scrollbar_dragged,
            ..
        } => (
            is_horizontal_scrollbar_dragged,
            is_vertical_scrollbar_dragged,
        ),
    };
    for (rail, active) in [
        (&mut style.horizontal_rail, horizontal_active),
        (&mut style.vertical_rail, vertical_active),
    ] {
        rail.background = None;
        rail.scroller.background = blend(theme, if active { 0.5 } else { 0.2 }).into();
    }
    style
}

pub(super) fn range(theme: &Theme, status: slider::Status) -> slider::Style {
    let ink = if status == slider::Status::Active {
        blend(theme, 0.8)
    } else {
        theme.palette().text
    };
    slider::Style {
        rail: slider::Rail {
            backgrounds: (ink.into(), line(theme).into()),
            width: 2.,
            border: Border {
                radius: 1.into(),
                ..Default::default()
            },
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 5. },
            background: theme.palette().background.into(),
            border_width: 1.5,
            border_color: ink,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_disabled_buttons_keep_disabled_feedback() {
        for dark in [false, true] {
            let theme = theme(dark);
            let disabled = selected(&theme, button::Status::Disabled);
            assert_eq!(
                disabled.text_color,
                quiet(&theme, button::Status::Disabled).text_color
            );
            assert_ne!(
                disabled.text_color,
                selected(&theme, button::Status::Active).text_color
            );
            assert!(disabled.background.is_some());
        }
    }

    #[test]
    fn controls_share_borders_and_keep_focus_and_disabled_feedback() {
        for dark in [false, true] {
            let theme = theme(dark);
            let input = input(&theme, text_input::Status::Active);
            assert_eq!(
                input.border,
                choice(&theme, pick_list::Status::Active).border
            );
            assert_eq!(
                input.border,
                secondary(&theme, button::Status::Active).border
            );
            assert_ne!(
                input.border.color,
                super::input(&theme, text_input::Status::Focused { is_hovered: false })
                    .border
                    .color
            );
            let active = primary(&theme, button::Status::Active);
            let disabled = primary(&theme, button::Status::Disabled);
            assert_ne!(active.background, disabled.background);
            assert_ne!(active.text_color, disabled.text_color);
            assert_eq!(active.text_color, theme.palette().background);
        }
    }
}
