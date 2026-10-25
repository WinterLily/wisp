use iced::{
    Border, Color, Theme,
    widget::{button, container},
};

pub(super) fn theme(dark: bool) -> Theme {
    Theme::custom(
        "Wisp",
        iced::theme::Palette {
            background: if dark {
                Color::from_rgb8(26, 27, 29)
            } else {
                Color::from_rgb8(245, 244, 242)
            },
            text: if dark {
                Color::from_rgb8(232, 232, 234)
            } else {
                Color::from_rgb8(42, 43, 46)
            },
            primary: if dark {
                Color::from_rgb8(141, 191, 229)
            } else {
                Color::from_rgb8(43, 104, 153)
            },
            success: Color::from_rgb8(91, 170, 139),
            warning: Color::from_rgb8(210, 163, 91),
            danger: Color::from_rgb8(205, 102, 102),
        },
    )
}

pub(super) fn muted(theme: &Theme) -> Color {
    let p = theme.palette();
    Color::from_rgb(
        p.text.r * 0.6 + p.background.r * 0.4,
        p.text.g * 0.6 + p.background.g * 0.4,
        p.text.b * 0.6 + p.background.b * 0.4,
    )
}

pub(super) fn quiet(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.extended_palette();
    let mut style = button::Style {
        text_color: p.background.base.text,
        border: Border {
            radius: 7.into(),
            ..Default::default()
        },
        ..Default::default()
    };
    if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        style.background = Some(p.background.weak.color.into());
    }
    if status == button::Status::Disabled {
        style.text_color = muted(theme).scale_alpha(0.5);
    }
    style
}

pub(super) fn selected(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = quiet(theme, status);
    style.background = Some(theme.extended_palette().background.weak.color.into());
    if status != button::Status::Disabled {
        style.text_color = theme.palette().primary;
    }
    style
}

pub(super) fn panel(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.into()),
        border: Border {
            radius: 12.into(),
            width: 1.,
            color: theme.extended_palette().background.weak.color,
        },
        ..Default::default()
    }
}

pub(super) fn stage(theme: &Theme) -> container::Style {
    let color = if theme.extended_palette().is_dark {
        Color::from_rgb8(18, 19, 21)
    } else {
        Color::from_rgb8(224, 223, 220)
    };
    container::Style {
        background: Some(color.into()),
        border: Border {
            radius: 10.into(),
            ..Default::default()
        },
        ..Default::default()
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
}
