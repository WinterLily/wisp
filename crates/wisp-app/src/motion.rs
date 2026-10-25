use iced::{
    Element, Event, Length, Padding, Rectangle, Size, Theme, Transformation, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, mouse, overlay,
        renderer::{self},
        widget::{Operation, Tree, tree},
    },
    time::Instant,
    widget::button,
};

// An analytic spring preserves momentum when interrupted and stays stable after long frames.
#[derive(Debug)]
struct Spring {
    value: f32,
    velocity: f32,
    target: f32,
}

impl Spring {
    fn new(value: f32) -> Self {
        Self {
            value,
            velocity: 0.,
            target: 0.,
        }
    }

    fn is_moving(&self) -> bool {
        (self.value - self.target).abs() > 0.001 || self.velocity.abs() > 0.01
    }

    fn advance(&mut self, seconds: f32) {
        let damping = 13.;
        let frequency = 17.;
        let displacement = self.value - self.target;
        let momentum = (self.velocity + damping * displacement) / frequency;
        let (sin, cos) = (frequency * seconds).sin_cos();
        let decay = (-damping * seconds).exp();
        self.value = self.target + decay * (displacement * cos + momentum * sin);
        self.velocity = decay
            * ((momentum * frequency - damping * displacement) * cos
                - (displacement * frequency + damping * momentum) * sin);
        if !self.is_moving() {
            self.value = self.target;
            self.velocity = 0.;
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Control { enabled: bool, selected: bool },
    Entrance { key: u64, offset: Vector },
}

struct State {
    kind: Kind,
    spring: Spring,
    last_frame: Option<Instant>,
    pressed: bool,
}

impl State {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            spring: Spring::new(if matches!(kind, Kind::Entrance { .. }) {
                1.
            } else {
                0.
            }),
            last_frame: None,
            pressed: false,
        }
    }
}

struct Motion<'a, Message, Render = iced::Renderer> {
    content: Element<'a, Message, Theme, Render>,
    kind: Kind,
}

pub(super) fn entrance<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    key: u64,
    offset: Vector,
) -> Element<'a, Message> {
    Element::new(Motion {
        content: content.into(),
        kind: Kind::Entrance { key, offset },
    })
}

impl<Message, Render: renderer::Renderer> Widget<Message, Theme, Render>
    for Motion<'_, Message, Render>
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new(self.kind))
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        if state.kind != self.kind {
            match (state.kind, self.kind) {
                (_, Kind::Entrance { .. }) => *state = State::new(self.kind),
                (_, Kind::Control { enabled: false, .. }) => *state = State::new(self.kind),
                (
                    Kind::Control {
                        selected: false, ..
                    },
                    Kind::Control { selected: true, .. },
                ) => {
                    state.spring.value = -0.8;
                    state.spring.velocity = 0.;
                }
                _ => {}
            }
            state.kind = self.kind;
        }
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Render,
        limits: &layout::Limits,
    ) -> layout::Node {
        let mut child =
            self.content
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, limits);
        let size = child.size();
        if let Kind::Entrance { offset, .. } = self.kind {
            let progress = tree.state.downcast_ref::<State>().spring.value;
            child = child.translate(offset * progress);
        }
        // Moving the actual layout keeps GPU previews, input and popup anchors together.
        layout::Node::with_children(size, vec![child])
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Render,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let available = !shell.is_event_captured();
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        let state = tree.state.downcast_mut::<State>();
        if let Kind::Control { enabled, .. } = self.kind {
            let hovered = enabled && cursor.is_over(layout.bounds()) && cursor.is_over(*viewport);
            match event {
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                | Event::Touch(iced::touch::Event::FingerPressed { .. })
                    if available && hovered =>
                {
                    state.pressed = true;
                }
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                | Event::Touch(iced::touch::Event::FingerLifted { .. })
                | Event::Touch(iced::touch::Event::FingerLost { .. })
                | Event::Window(iced::window::Event::Unfocused) => state.pressed = false,
                _ => {}
            }
            state.spring.target = if !hovered {
                0.
            } else if state.pressed {
                -1.4
            } else {
                1.
            };
        }
        if let Event::Window(iced::window::Event::RedrawRequested(now)) = event {
            let delta = state
                .last_frame
                .map_or(0., |last| now.saturating_duration_since(last).as_secs_f32());
            state.last_frame = Some(*now);
            let moving = state.spring.is_moving();
            state.spring.advance(delta);
            if moving && matches!(self.kind, Kind::Entrance { .. }) {
                shell.invalidate_layout();
            }
        }
        if state.spring.is_moving() && layout.bounds().intersects(viewport) {
            shell.request_redraw();
        } else {
            // Idle time must not consume a new hover or selection animation's first frame.
            state.last_frame = None;
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Render,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let value = tree.state.downcast_ref::<State>().spring.value;
        let transformation = if matches!(self.kind, Kind::Control { .. }) {
            // Cap growth on wide cards so neighboring controls never get crowded out.
            let scale = 1. + value * (3. / bounds.width.max(bounds.height).max(1.)).min(0.035);
            Transformation::translate(bounds.center_x(), bounds.center_y() - value * 1.5)
                * Transformation::scale(scale)
                * Transformation::translate(-bounds.center_x(), -bounds.center_y())
        } else {
            Transformation::IDENTITY
        };
        // Control hit areas stay fixed while their visual surface bounces.
        renderer.with_transformation(transformation, |renderer| {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout.children().next().unwrap(),
                cursor,
                viewport,
            );
        });
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Render,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        )
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Render,
        operation: &mut dyn Operation,
    ) {
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Render,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Render>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

pub(super) struct Button<'a, Message> {
    content: button::Button<'a, Message>,
    enabled: bool,
    selected: bool,
}

impl<'a, Message: Clone + 'a> Button<'a, Message> {
    pub(super) fn new(content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            content: button(content),
            enabled: false,
            selected: false,
        }
    }

    pub(super) fn width(mut self, width: impl Into<Length>) -> Self {
        self.content = self.content.width(width);
        self
    }

    pub(super) fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.content = self.content.padding(padding);
        self
    }

    pub(super) fn style(
        mut self,
        style: impl Fn(&Theme, button::Status) -> button::Style + 'a,
    ) -> Self {
        self.content = self.content.style(style);
        self
    }

    pub(super) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub(super) fn on_press(self, message: Message) -> Self {
        self.on_press_maybe(Some(message))
    }

    pub(super) fn on_press_maybe(mut self, message: Option<Message>) -> Self {
        self.enabled = message.is_some();
        self.content = self.content.on_press_maybe(message);
        self
    }
}

impl<'a, Message: Clone + 'a> From<Button<'a, Message>> for Element<'a, Message> {
    fn from(button: Button<'a, Message>) -> Self {
        Element::new(Motion {
            content: button.content.into(),
            kind: Kind::Control {
                enabled: button.enabled,
                selected: button.selected,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::{Point, advanced::clipboard, widget::space, window};
    use std::time::Duration;

    #[test]
    fn spring_bounces_settles_and_is_independent_of_frame_rate() {
        for fps in [15, 30, 60, 144] {
            let mut spring = Spring::new(1.);
            let mut overshot = false;
            for _ in 0..fps {
                spring.advance(1. / fps as f32);
                overshot |= spring.value < 0.;
            }
            assert!(overshot);
            assert!(!spring.is_moving());
            assert_eq!(spring.value, 0.);
        }
        let mut single_frame = Spring::new(1.);
        single_frame.advance(0.1);
        let mut many_frames = Spring::new(1.);
        for _ in 0..10 {
            many_frames.advance(0.01);
        }
        assert!((single_frame.value - many_frames.value).abs() < 0.00001);
        assert!((single_frame.velocity - many_frames.velocity).abs() < 0.0001);
    }

    #[test]
    fn spring_retargets_without_jumps_and_survives_a_suspended_window() {
        let mut spring = Spring::new(0.);
        for target in [1., -1.4, 1., 0.] {
            let previous = (spring.value, spring.velocity);
            spring.target = target;
            spring.advance(0.);
            assert!((spring.value - previous.0).abs() < 0.00001);
            assert!((spring.velocity - previous.1).abs() < 0.00001);
            spring.advance(0.05);
        }
        spring.advance(3600.);
        assert_eq!(spring.value, 0.);
        assert_eq!(spring.velocity, 0.);
        assert!(!spring.is_moving());
    }

    struct Harness {
        element: Element<'static, u8, Theme, ()>,
        tree: Tree,
        node: layout::Node,
        messages: Vec<u8>,
    }

    impl Harness {
        fn new(kind: Kind) -> Self {
            let content = button(space().width(100).height(36))
                .padding(0)
                .on_press_maybe(
                    (!matches!(kind, Kind::Control { enabled: false, .. })).then_some(7),
                );
            let element = Element::new(Motion {
                content: content.into(),
                kind,
            });
            let tree = Tree::new(&element);
            let mut harness = Self {
                element,
                tree,
                node: layout::Node::new(Size::ZERO),
                messages: vec![],
            };
            harness.layout();
            harness
        }

        fn layout(&mut self) {
            self.node = self.element.as_widget_mut().layout(
                &mut self.tree,
                &(),
                &layout::Limits::new(Size::ZERO, Size::new(400., 400.)),
            );
        }

        fn event(&mut self, event: Event, cursor: mouse::Cursor) -> window::RedrawRequest {
            let mut shell = Shell::new(&mut self.messages);
            self.element.as_widget_mut().update(
                &mut self.tree,
                &event,
                Layout::new(&self.node),
                cursor,
                &(),
                &mut clipboard::Null,
                &mut shell,
                &Rectangle::with_size(Size::new(400., 400.)),
            );
            let redraw = shell.redraw_request();
            if shell.is_layout_invalid() {
                self.layout();
            }
            redraw
        }
    }

    #[test]
    fn controls_animate_on_demand_with_fixed_hit_areas_and_preserve_clicks() {
        let mut harness = Harness::new(Kind::Control {
            enabled: true,
            selected: false,
        });
        let cursor = mouse::Cursor::Available(Point::new(1., 1.));
        let now = Instant::now();
        for frame in 0..=60 {
            harness.event(
                Event::Window(window::Event::RedrawRequested(
                    now + Duration::from_millis(frame * 16),
                )),
                cursor,
            );
        }
        assert_eq!(harness.tree.state.downcast_ref::<State>().spring.value, 1.);
        assert_eq!(harness.node.bounds().size(), Size::new(100., 36.));
        harness.event(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            cursor,
        );
        assert_eq!(
            harness.tree.state.downcast_ref::<State>().spring.target,
            -1.4
        );
        harness.event(
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            cursor,
        );
        assert_eq!(harness.messages, vec![7]);
        for frame in 61..=121 {
            let redraw = harness.event(
                Event::Window(window::Event::RedrawRequested(
                    now + Duration::from_millis(frame * 16),
                )),
                mouse::Cursor::Unavailable,
            );
            if frame == 121 {
                assert_eq!(redraw, window::RedrawRequest::Wait);
            }
        }
        assert_eq!(harness.tree.state.downcast_ref::<State>().spring.value, 0.);
    }

    #[test]
    fn entrance_moves_layout_and_clicks_together_then_stops_redrawing() {
        let mut harness = Harness::new(Kind::Entrance {
            key: 1,
            offset: Vector::new(24., 0.),
        });
        assert_eq!(harness.node.children()[0].bounds().x, 24.);
        let outside = mouse::Cursor::Available(Point::new(2., 10.));
        harness.event(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            outside,
        );
        harness.event(
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            outside,
        );
        assert!(harness.messages.is_empty());
        let inside = mouse::Cursor::Available(Point::new(120., 10.));
        harness.event(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            inside,
        );
        harness.event(
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            inside,
        );
        assert_eq!(harness.messages, vec![7]);
        let now = Instant::now();
        for frame in 0..=60 {
            let redraw = harness.event(
                Event::Window(window::Event::RedrawRequested(
                    now + Duration::from_millis(frame * 16),
                )),
                mouse::Cursor::Unavailable,
            );
            if frame == 60 {
                assert_eq!(redraw, window::RedrawRequest::Wait);
            }
        }
        assert_eq!(harness.node.children()[0].bounds().x, 0.);
    }

    #[test]
    fn rebuilds_preserve_motion_selection_pops_and_disabled_controls_stay_still() {
        let mut harness = Harness::new(Kind::Control {
            enabled: true,
            selected: false,
        });
        let cursor = mouse::Cursor::Available(Point::new(10., 10.));
        harness.event(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            cursor,
        );
        harness.tree.state.downcast_mut::<State>().spring.value = -0.5;
        let unchanged = Harness::new(Kind::Control {
            enabled: true,
            selected: false,
        });
        unchanged.element.as_widget().diff(&mut harness.tree);
        assert_eq!(
            harness.tree.state.downcast_ref::<State>().spring.value,
            -0.5
        );

        let selected = Harness::new(Kind::Control {
            enabled: true,
            selected: true,
        });
        selected.element.as_widget().diff(&mut harness.tree);
        assert_eq!(
            harness.tree.state.downcast_ref::<State>().spring.value,
            -0.8
        );

        let disabled = Harness::new(Kind::Control {
            enabled: false,
            selected: true,
        });
        disabled.element.as_widget().diff(&mut harness.tree);
        harness.element = disabled.element;
        assert!(!harness.tree.state.downcast_ref::<State>().pressed);
        for event in [
            mouse::Event::ButtonPressed(mouse::Button::Left),
            mouse::Event::ButtonReleased(mouse::Button::Left),
        ] {
            harness.event(Event::Mouse(event), cursor);
        }
        assert!(harness.messages.is_empty());
        assert!(
            !harness
                .tree
                .state
                .downcast_ref::<State>()
                .spring
                .is_moving()
        );
    }

    #[test]
    fn entrance_only_restarts_when_its_identity_changes() {
        let mut harness = Harness::new(Kind::Entrance {
            key: 1,
            offset: Vector::new(24., 0.),
        });
        harness
            .tree
            .state
            .downcast_mut::<State>()
            .spring
            .advance(1.);
        let same_panel = Harness::new(Kind::Entrance {
            key: 1,
            offset: Vector::new(24., 0.),
        });
        same_panel.element.as_widget().diff(&mut harness.tree);
        assert_eq!(harness.tree.state.downcast_ref::<State>().spring.value, 0.);
        let next_panel = Harness::new(Kind::Entrance {
            key: 2,
            offset: Vector::new(24., 0.),
        });
        next_panel.element.as_widget().diff(&mut harness.tree);
        assert_eq!(harness.tree.state.downcast_ref::<State>().spring.value, 1.);
    }
}
