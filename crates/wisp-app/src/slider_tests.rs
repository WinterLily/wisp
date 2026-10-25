use super::*;
use crate::{designer::Draft, layers::ControlId};
use designer_ui::{
    DesignEffect, DesignerMessage, DesignerPanel, DesignerSelection, layer_ui::LayerMessage,
};
use iced::{Event, Point, Rectangle, Size, mouse};
use iced_wgpu::core::{Layout, Shell, clipboard, layout, widget::Tree};

fn designer() -> (tempfile::TempDir, App) {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source.png");
    std::fs::write(&source, wisp_runtime::EXAMPLE_IMAGE).unwrap();
    let mut draft = Draft::from_image(&source).unwrap();
    draft.design.blur = true;
    let wallpaper = Arc::new(Wallpaper::from_package(&draft.compile().unwrap().package).unwrap());
    let library = Library::open(Some(temporary.path().join("library"))).unwrap();
    let (mut app, _) = App::new(library, Settings::default(), None, None);
    app.page = Page::Designer;
    app.designer.draft = Some(Arc::new(draft));
    app.designer.panel = Some(DesignerPanel::Layers);
    app.designer.selection = DesignerSelection::Effect(DesignEffect::Pulse);
    let _ = app.designer_update(DesignerMessage::Compiled(Ok((wallpaper, String::new()))));
    (temporary, app)
}

#[test]
fn scene_slider_edits_during_compilation_survive_completion() {
    let (_temporary, mut app) = designer();
    let pending_wallpaper = app.designer.preview.as_ref().unwrap().wallpaper.clone();
    app.busy = true;
    app.designer.compiling = true;
    for control in crate::designer::CONTROLS {
        for fraction in [0.25, 0.75] {
            let value = control.min + (control.max - control.min) * fraction;
            let _ = app.designer_update(DesignerMessage::Value(control.id.into(), value));
            assert_eq!(
                app.designer.draft.as_ref().unwrap().design.values[control.id],
                value
            );
        }
    }
    let _ = app.designer_update(DesignerMessage::Compiled(Ok((
        pending_wallpaper,
        String::new(),
    ))));
    for control in crate::designer::CONTROLS {
        assert_eq!(
            app.designer.preview.as_ref().unwrap().parameters[control.id],
            ParameterValue::Float(app.designer.draft.as_ref().unwrap().design.values[control.id]),
        );
    }
    assert!(!app.busy);
}

#[test]
fn layer_slider_release_during_compilation_rebuilds_latest_draft() {
    for failed_compile in [false, true] {
        let (_temporary, mut app) = designer();
        Arc::make_mut(app.designer.draft.as_mut().unwrap()).enable_layers();
        let pending_wallpaper = app.designer.preview.as_ref().unwrap().wallpaper.clone();
        app.busy = true;
        app.designer.compiling = true;
        for value in [0.25, 0.75] {
            let _ = app.designer_update(DesignerMessage::Layer(LayerMessage::Value(
                ControlId::Opacity,
                value,
            )));
            assert_eq!(
                app.designer
                    .draft
                    .as_ref()
                    .unwrap()
                    .layers
                    .as_ref()
                    .unwrap()
                    .layers[0]
                    .opacity,
                value
            );
            let _ = app.designer_update(DesignerMessage::Layer(LayerMessage::Commit));
        }
        let result = if failed_compile {
            Err("previous preview failed".into())
        } else {
            Ok((pending_wallpaper, String::new()))
        };
        let _ = app.designer_update(DesignerMessage::Compiled(result));
        assert!(
            app.busy && app.designer.compiling,
            "the released edit must get its own preview compile"
        );
        let latest = app.designer.draft.as_ref().unwrap().compile().unwrap();
        let latest = Arc::new(Wallpaper::from_package(&latest.package).unwrap());
        let _ = app.designer_update(DesignerMessage::Compiled(Ok((latest, String::new()))));
        assert!(!app.busy && !app.designer.compiling);
    }
}

#[test]
fn slider_edits_still_respect_other_busy_operations() {
    let (_temporary, mut app) = designer();
    Arc::make_mut(app.designer.draft.as_mut().unwrap()).enable_layers();
    app.busy = true;
    let _ = app.designer_update(DesignerMessage::Value("brightness".into(), 0.5));
    let _ = app.designer_update(DesignerMessage::Layer(LayerMessage::Value(
        ControlId::Opacity,
        0.5,
    )));
    let draft = app.designer.draft.as_ref().unwrap();
    assert_eq!(draft.design.values["brightness"], 1.);
    assert_eq!(draft.layers.as_ref().unwrap().layers[0].opacity, 1.);
}

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

const WINDOW_SIZE: Size = Size::new(1180., 1400.);

fn slider_bounds(layout: Layout<'_>, bounds: &mut Vec<Rectangle>) {
    if layout.children().count() == 0
        && layout.bounds().height == iced::widget::Slider::<f32, Message>::DEFAULT_HEIGHT
        && layout.bounds().width > 30.
    {
        bounds.push(layout.bounds());
    }
    for layout in layout.children() {
        slider_bounds(layout, bounds);
    }
}

fn dispatch(
    app: &mut App,
    tree: &mut Tree,
    renderer: &iced_wgpu::Renderer,
    event: mouse::Event,
    point: Point,
) -> Vec<Message> {
    let mut element = app.view();
    tree.diff(element.as_widget());
    let node = element.as_widget_mut().layout(
        tree,
        renderer,
        &layout::Limits::new(Size::ZERO, WINDOW_SIZE),
    );
    let mut messages = Vec::new();
    let event = match event {
        mouse::Event::CursorMoved { .. } => mouse::Event::CursorMoved { position: point },
        event => event,
    };
    element.as_widget_mut().update(
        tree,
        &Event::Mouse(event),
        Layout::new(&node),
        mouse::Cursor::Available(point),
        renderer,
        &mut clipboard::Null,
        &mut Shell::new(&mut messages),
        &Rectangle::new(Point::ORIGIN, WINDOW_SIZE),
    );
    drop(element);
    for message in messages.iter().cloned() {
        let _ = app.update(message);
    }
    messages
}

#[test]
#[ignore = "requires a working graphics adapter"]
fn effects_sliders_follow_pointer_across_view_rebuilds_while_compiling() {
    let renderer = renderer();
    for layers in [false, true] {
        let (_temporary, mut app) = designer();
        if layers {
            let stack = Arc::make_mut(app.designer.draft.as_mut().unwrap()).enable_layers();
            stack.add_effect(0, crate::layers::Effect::Hair).unwrap();
            app.designer.panel = Some(DesignerPanel::Layers);
            app.designer.selection = DesignerSelection::LayerEffect(0);
        }
        app.busy = true;
        app.designer.compiling = true;
        let mut tree = Tree::new(app.view().as_widget());
        let mut element = app.view();
        let node = element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, WINDOW_SIZE),
        );
        let mut bounds = Vec::new();
        slider_bounds(Layout::new(&node), &mut bounds);
        assert_eq!(bounds.len(), if layers { 5 } else { 2 });
        drop(element);
        for bounds in bounds {
            for (event, fraction) in [
                (mouse::Event::ButtonPressed(mouse::Button::Left), 0.37),
                (
                    mouse::Event::CursorMoved {
                        position: Point::ORIGIN,
                    },
                    0.73,
                ),
            ] {
                let point = Point::new(bounds.x + bounds.width * fraction, bounds.center_y());
                let messages = dispatch(&mut app, &mut tree, &renderer, event, point);
                assert_eq!(messages.len(), 1, "slider did not respond at {point:?}");
                let draft = app.designer.draft.as_ref().unwrap();
                match &messages[0] {
                    Message::Designer(DesignerMessage::Value(id, value)) => {
                        assert_eq!(draft.design.values[id], *value);
                    }
                    Message::Designer(DesignerMessage::Layer(LayerMessage::Value(id, value))) => {
                        assert_eq!(
                            draft.layers.as_ref().unwrap().layers[0].effects[0].value(*id),
                            *value
                        );
                    }
                    _ => panic!("slider emitted an unexpected message"),
                }
            }
            let messages = dispatch(
                &mut app,
                &mut tree,
                &renderer,
                mouse::Event::ButtonReleased(mouse::Button::Left),
                bounds.center(),
            );
            assert_eq!(messages.len(), usize::from(layers));
        }
    }
}
