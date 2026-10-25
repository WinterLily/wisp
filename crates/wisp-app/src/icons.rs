use super::{Message, style};
use iced::{
    Element,
    widget::{button, container, svg, text, tooltip},
};
use std::time::Duration;

macro_rules! icons {
    ($($name:ident => $file:literal),+ $(,)?) => {
        #[derive(Clone, Copy)]
        pub(super) enum Icon { $($name),+ }

        impl Icon {
            #[cfg(test)]
            const ALL: &'static [Self] = &[$(Self::$name),+];

            fn handle(self) -> svg::Handle {
                let bytes: &'static [u8] = match self {
                    $(Self::$name => include_bytes!(concat!("../assets/icons/", $file, ".svg"))),+
                };
                svg::Handle::from_memory(bytes)
            }
        }
    };
}

icons! {
    Library => "grid-2x2",
    Create => "image-plus",
    Graph => "workflow",
    Settings => "settings",
    Import => "import",
    Adjust => "sliders-horizontal",
    Display => "monitor",
    Info => "info",
    Layers => "layers",
    Effects => "sparkles",
    Project => "folder",
    Play => "play",
    Pause => "pause",
    Focus => "maximize",
    Unfocus => "minimize",
    Close => "x",
    Forward => "arrow-up",
    Backward => "arrow-down",
    Remove => "trash-2",
    Motion => "move",
    Mask => "scan",
    HardRound => "brush",
    Airbrush => "spray-can",
    Lasso => "lasso",
    MagicWand => "wand-sparkles",
    Paint => "paintbrush",
    Erase => "eraser",
    Done => "check",
    Add => "plus",
    Arrange => "network",
    Reset => "rotate-ccw",
    Disconnect => "unlink",
}

pub(super) fn glyph(icon: Icon, size: u32) -> svg::Svg<'static> {
    svg(icon.handle())
        .width(size)
        .height(size)
        .style(|theme: &iced::Theme, _| svg::Style {
            color: Some(theme.palette().text),
        })
}

pub(super) fn icon_button<'a>(
    icon: Icon,
    label: impl Into<std::borrow::Cow<'a, str>>,
    message: Option<Message>,
    selected: bool,
) -> Element<'a, Message> {
    let enabled = message.is_some();
    let glyph = svg(icon.handle())
        .width(20)
        .height(20)
        .style(move |theme: &iced::Theme, _| svg::Style {
            color: Some(if !enabled {
                style::muted(theme).scale_alpha(0.5)
            } else if selected {
                theme.palette().primary
            } else {
                theme.palette().text
            }),
        });
    let control = button(container(glyph).center(36))
        .padding(0)
        .style(if selected {
            style::selected
        } else {
            style::quiet
        })
        .on_press_maybe(message);
    tooltip(
        control,
        container(text(label.into()).size(12)).padding([6, 9]),
        tooltip::Position::Bottom,
    )
    .gap(6)
    .delay(Duration::from_millis(300))
    .snap_within_viewport(true)
    .style(container::rounded_box)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::App;
    use crate::{library::Library, painting::PaintTool};
    use iced::{Event, Point, Rectangle, Size, mouse};
    use iced_wgpu::core::{
        Layout, Renderer as _, Shell, clipboard, layout, renderer, widget::Tree,
    };

    fn renderer() -> iced_wgpu::Renderer {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let gpu = pollster::block_on(wisp_runtime::Gpu::new(&instance, None)).unwrap();
        let engine = iced_wgpu::Engine::new(
            &gpu.adapter,
            gpu.device,
            gpu.queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            None,
            iced_wgpu::graphics::Shell::headless(),
        );
        iced_wgpu::Renderer::new(engine, iced::Font::DEFAULT, iced::Pixels(14.))
    }

    fn click(
        mut element: Element<'_, Message>,
        renderer: &iced_wgpu::Renderer,
        point: Point,
    ) -> Vec<Message> {
        let size = Size::new(900., 620.);
        let mut tree = Tree::new(element.as_widget());
        let node = element.as_widget_mut().layout(
            &mut tree,
            renderer,
            &layout::Limits::new(Size::ZERO, size),
        );
        let mut messages = Vec::new();
        for event in [
            mouse::Event::ButtonPressed(mouse::Button::Left),
            mouse::Event::ButtonReleased(mouse::Button::Left),
        ] {
            element.as_widget_mut().update(
                &mut tree,
                &Event::Mouse(event),
                Layout::new(&node),
                mouse::Cursor::Available(point),
                renderer,
                &mut clipboard::Null,
                &mut Shell::new(&mut messages),
                &Rectangle::new(Point::ORIGIN, size),
            );
        }
        messages
    }

    #[test]
    #[ignore = "requires a working graphics adapter"]
    fn icons_render_in_both_themes_and_disabled_controls_ignore_clicks() {
        let mut renderer = renderer();
        let size = Size::new(36., 36.);
        let bounds = Rectangle::new(Point::ORIGIN, size);
        for dark in [false, true] {
            let theme = style::theme(dark);
            for &icon in Icon::ALL {
                let mut frames = Vec::new();
                for (enabled, selected) in [(true, false), (true, true), (false, false)] {
                    let mut element = icon_button(
                        icon,
                        "Test control",
                        enabled.then_some(Message::Focus),
                        selected,
                    );
                    let mut tree = Tree::new(element.as_widget());
                    let node = element.as_widget_mut().layout(
                        &mut tree,
                        &renderer,
                        &layout::Limits::new(size, size),
                    );
                    element.as_widget_mut().update(
                        &mut tree,
                        &Event::Window(iced::window::Event::RedrawRequested(
                            iced::time::Instant::now(),
                        )),
                        Layout::new(&node),
                        mouse::Cursor::Unavailable,
                        &renderer,
                        &mut clipboard::Null,
                        &mut Shell::new(&mut Vec::new()),
                        &bounds,
                    );
                    renderer.reset(bounds);
                    element.as_widget().draw(
                        &tree,
                        &mut renderer,
                        &theme,
                        &renderer::Style {
                            text_color: theme.palette().text,
                        },
                        Layout::new(&node),
                        mouse::Cursor::Unavailable,
                        &bounds,
                    );
                    let viewport =
                        iced_wgpu::graphics::Viewport::with_physical_size(Size::new(36, 36), 1.);
                    let pixels = renderer.screenshot(&viewport, theme.palette().background);
                    let background = &pixels[..4];
                    assert!(
                        pixels
                            .chunks_exact(4)
                            .filter(|pixel| *pixel != background)
                            .count()
                            > 10,
                        "an icon rendered empty"
                    );
                    frames.push(pixels);
                    let messages = click(
                        icon_button(
                            icon,
                            "Test control",
                            enabled.then_some(Message::Focus),
                            selected,
                        ),
                        &renderer,
                        Point::new(18., 18.),
                    );
                    assert_eq!(messages.len(), usize::from(enabled));
                    assert!(
                        messages
                            .iter()
                            .all(|message| matches!(message, Message::Focus))
                    );
                }
                assert_ne!(frames[0], frames[1]);
                assert_ne!(frames[0], frames[2]);
            }
        }
    }

    #[test]
    #[ignore = "requires a working graphics adapter"]
    fn unified_layer_rows_select_images_and_effects_and_toggle_effect_visibility() {
        use crate::designer::Draft;
        use crate::ui::designer_ui::{DesignEffect, DesignerMessage, DesignerSelection};

        let renderer = renderer();
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("image.png");
        std::fs::write(&source, wisp_runtime::EXAMPLE_IMAGE).unwrap();
        let (mut app, _) = App::new(
            Library::open(Some(temporary.path().join("library"))).unwrap(),
            Default::default(),
            None,
            None,
        );
        app.busy = false;
        app.designer.draft = Some(std::sync::Arc::new(Draft::from_image(&source).unwrap()));

        for (point, selection) in [
            (
                Point::new(120., 60.),
                DesignerSelection::Effect(DesignEffect::Vignette),
            ),
            (Point::new(120., 225.), DesignerSelection::Color),
            (Point::new(120., 270.), DesignerSelection::Image),
        ] {
            let messages = click(
                app.designer_layers_view(app.designer.draft.as_ref().unwrap()),
                &renderer,
                point,
            );
            assert_eq!(messages.len(), 1);
            for message in messages {
                let _ = app.update(message);
            }
            assert_eq!(app.designer.selection, selection);
            assert!(app.designer.draft.as_ref().unwrap().layers.is_none());
        }
        let messages = click(
            app.designer_layers_view(app.designer.draft.as_ref().unwrap()),
            &renderer,
            Point::new(10., 60.),
        );
        assert!(matches!(
            messages.as_slice(),
            [Message::Designer(DesignerMessage::Toggle(
                DesignEffect::Vignette,
                false
            ))]
        ));
        app.busy = true;
        for point in [
            Point::new(120., 60.),
            Point::new(10., 60.),
            Point::new(120., 270.),
        ] {
            assert!(
                click(
                    app.designer_layers_view(app.designer.draft.as_ref().unwrap()),
                    &renderer,
                    point
                )
                .is_empty()
            );
        }
    }

    #[test]
    #[ignore = "requires a working graphics adapter"]
    fn add_image_icon_keeps_the_layer_import_target_and_limits() {
        use crate::designer::Draft;
        use crate::layers::ImportTarget;
        use crate::ui::designer_ui::{DesignerMessage, DesignerPanel, layer_ui::LayerMessage};

        let renderer = renderer();
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("image.png");
        std::fs::write(&source, wisp_runtime::EXAMPLE_IMAGE).unwrap();
        let mut draft = Draft::from_image(&source).unwrap();
        draft.enable_layers();
        let library = Library::open(Some(temporary.path().join("library"))).unwrap();
        let (mut app, _) = App::new(library, Default::default(), None, None);
        app.designer.draft = Some(std::sync::Arc::new(draft));
        app.designer.panel = Some(DesignerPanel::Layers);

        for (busy, layer_count) in [(false, 1), (true, 1), (false, 5), (false, 6)] {
            app.busy = busy;
            let stack = std::sync::Arc::make_mut(app.designer.draft.as_mut().unwrap())
                .layers
                .as_mut()
                .unwrap();
            stack.layers.resize(layer_count, stack.layers[0].clone());
            let draft = app.designer.draft.as_ref().unwrap();
            let messages = click(
                app.designer_layers_view(draft),
                &renderer,
                Point::new(882., 18.),
            );
            assert_eq!(messages.len(), usize::from(!busy && layer_count < 6));
            assert!(messages.iter().all(|message| matches!(
                message,
                Message::Designer(DesignerMessage::Layer(LayerMessage::Import(
                    ImportTarget::Layer
                )))
            )));
        }
    }

    #[test]
    #[ignore = "requires a working graphics adapter"]
    fn paint_tool_icons_select_each_tool_and_paint_mode() {
        let renderer = renderer();
        let temporary = tempfile::tempdir().unwrap();
        let library = Library::open(Some(temporary.path().to_owned())).unwrap();
        let (mut app, _) = App::new(library, Default::default(), None, None);
        app.busy = false;
        app.designer.paint_expanded = true;
        for (index, tool) in PaintTool::ALL.into_iter().enumerate() {
            let messages = click(
                app.designer_view(),
                &renderer,
                Point::new(26. + index as f32 * 40., 70.),
            );
            assert_eq!(messages.len(), 1);
            for message in messages {
                let _ = app.update(message);
            }
            assert_eq!(app.designer.paint_tool, tool);
        }
        for (x, erase) in [(814., true), (768., false)] {
            let messages = click(app.designer_view(), &renderer, Point::new(x, 22.));
            assert_eq!(messages.len(), 1);
            for message in messages {
                let _ = app.update(message);
            }
            assert_eq!(app.designer.erase, erase);
        }
    }
}
