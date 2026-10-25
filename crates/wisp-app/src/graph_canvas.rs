use super::Message;
use super::graph_ui::{GraphEditor, GraphMessage};
use crate::graph::{label, ports};
use iced::{
    Color, Point, Rectangle, Size, mouse,
    widget::{
        Action,
        canvas::{self, Frame, Path, Stroke, Text},
    },
};

pub(super) struct GraphCanvas<'a>(pub &'a GraphEditor, pub bool);
#[derive(Default)]
pub(super) struct State {
    drag: Option<(String, Point)>,
    pan: Option<Point>,
}
const WIDTH: f32 = 206.;
fn height(inputs: usize) -> f32 {
    60. + inputs as f32 * 24.
}
fn input(p: Point, i: usize) -> Point {
    Point::new(p.x, p.y + 60. + i as f32 * 24.)
}
fn output(p: Point) -> Point {
    Point::new(p.x + WIDTH, p.y + 28.)
}
impl GraphCanvas<'_> {
    fn screen(&self, p: [f32; 2]) -> Point {
        Point::new(
            p[0] * self.0.zoom + self.0.pan.x,
            p[1] * self.0.zoom + self.0.pan.y,
        )
    }
    fn world(&self, p: Point) -> Point {
        Point::new(
            (p.x - self.0.pan.x) / self.0.zoom,
            (p.y - self.0.pan.y) / self.0.zoom,
        )
    }
}
impl canvas::Program<Message> for GraphCanvas<'_> {
    type State = State;
    fn update(
        &self,
        state: &mut State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        let emit = |m| Some(Action::publish(Message::Graph(m)).and_capture());
        if matches!(event, canvas::Event::Mouse(mouse::Event::ButtonReleased(_))) {
            state.drag = None;
            state.pan = None;
            return None;
        }
        if matches!(
            event,
            canvas::Event::Window(iced::window::Event::RedrawRequested(_))
        ) && self.0.canvas_size != bounds.size()
        {
            return emit(GraphMessage::CanvasSize(bounds.size()));
        }
        let p = cursor.position_in(bounds)?;
        match event {
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 30.,
                };
                emit(GraphMessage::Zoom(y, p))
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some((id, offset)) = &state.drag {
                    let w = self.world(p);
                    return emit(GraphMessage::Move(
                        id.clone(),
                        [w.x - offset.x, w.y - offset.y],
                    ));
                }
                if let Some(previous) = state.pan.replace(p) {
                    return emit(GraphMessage::Pan(p - previous));
                }
                state.pan = None;
                self.0.wire.as_ref().map(|_| Action::request_redraw())
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle)) => {
                state.pan = Some(p);
                Some(Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                emit(GraphMessage::CancelWire)
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if !self.1 => {
                let doc = self.0.document.as_ref()?;
                let w = self.world(p);
                for n in doc.graph.nodes.iter().rev() {
                    let pos = doc.layout.positions.get(&n.id).copied().unwrap_or_default();
                    let pos = Point::new(pos[0], pos[1]);
                    if !matches!(n.op, wisp_ir::Op::Output { .. }) && w.distance(output(pos)) < 12.
                    {
                        return emit(GraphMessage::Wire(n.id.clone()));
                    }
                    for (i, port) in ports(&n.op).iter().enumerate() {
                        if w.distance(input(pos, i)) < 12. {
                            return emit(GraphMessage::Socket(n.id.clone(), (*port).into()));
                        }
                    }
                    if Rectangle::new(pos, Size::new(WIDTH, height(ports(&n.op).len()))).contains(w)
                    {
                        state.drag = Some((n.id.clone(), Point::new(w.x - pos.x, w.y - pos.y)));
                        return emit(GraphMessage::Select(n.id.clone()));
                    }
                }
                state.pan = Some(p);
                emit(GraphMessage::CancelWire)
            }
            _ => None,
        }
    }
    fn draw(
        &self,
        _: &State,
        renderer: &iced::Renderer,
        theme: &iced::Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        f.fill_rectangle(Point::ORIGIN, bounds.size(), super::style::surface(theme));
        let Some(doc) = &self.0.document else {
            return vec![f.into_geometry()];
        };
        let zoom = self.0.zoom;
        for x in (0..bounds.width as usize).step_by(24) {
            for y in (0..bounds.height as usize).step_by(24) {
                f.fill(
                    &Path::circle(Point::new(x as f32, y as f32), 0.7),
                    super::style::line(theme),
                );
            }
        }
        let color = |ty: Option<wisp_ir::ValueType>| match ty {
            Some(wisp_ir::ValueType::Color) => Color::from_rgb8(181, 154, 116),
            Some(
                wisp_ir::ValueType::Vector2
                | wisp_ir::ValueType::Vector3
                | wisp_ir::ValueType::Vector4,
            ) => Color::from_rgb8(131, 157, 178),
            _ => Color::from_rgb8(139, 164, 145),
        };
        let curve = |a: Point, b: Point| {
            Path::new(|p| {
                p.move_to(a);
                let d = ((b.x - a.x).abs() * 0.5).max(40.);
                p.bezier_curve_to(Point::new(a.x + d, a.y), Point::new(b.x - d, b.y), b);
            })
        };
        for n in &doc.graph.nodes {
            let p = self.screen(doc.layout.positions.get(&n.id).copied().unwrap_or_default());
            for (i, source) in n.op.inputs().iter().enumerate() {
                if let Some(pos) = doc.layout.positions.get(*source) {
                    let a = self.screen([pos[0] + WIDTH, pos[1] + 28.]);
                    let b = Point::new(p.x, p.y + (60. + i as f32 * 24.) * zoom);
                    f.stroke(
                        &curve(a, b),
                        Stroke::default()
                            .with_width(2.)
                            .with_color(color(doc.value_type(source))),
                    );
                }
            }
        }
        if let Some(source) = &self.0.wire
            && let Some(pos) = doc.layout.positions.get(source)
            && let Some(b) = cursor.position_in(bounds)
        {
            f.stroke(
                &curve(self.screen([pos[0] + WIDTH, pos[1] + 28.]), b),
                Stroke::default()
                    .with_width(2.)
                    .with_color(theme.palette().text),
            );
        }
        for n in &doc.graph.nodes {
            let p = self.screen(doc.layout.positions.get(&n.id).copied().unwrap_or_default());
            let size = Size::new(WIDTH * zoom, height(ports(&n.op).len()) * zoom);
            let rect =
                Path::rounded_rectangle(p, size, (super::style::CORNER_RADIUS * zoom).into());
            f.fill(&rect, theme.palette().background);
            f.stroke(
                &rect,
                Stroke::default()
                    .with_width(if self.0.selected.as_ref() == Some(&n.id) {
                        2.
                    } else {
                        1.
                    })
                    .with_color(if self.0.selected.as_ref() == Some(&n.id) {
                        theme.palette().primary
                    } else {
                        super::style::line(theme)
                    }),
            );
            let title = if doc.graph.output == n.id {
                format!("{} · active", label(&n.op))
            } else {
                label(&n.op)
            };
            f.fill_text(Text {
                content: title.to_lowercase(),
                position: Point::new(p.x + 10. * zoom, p.y + 5. * zoom),
                font: super::style::FONT,
                size: (14. * zoom).into(),
                color: theme.palette().text,
                ..Default::default()
            });
            f.fill_text(Text {
                content: format!(
                    "{} · {}",
                    n.id.to_lowercase(),
                    doc.value_type(&n.id)
                        .map(|t| format!("{t:?}").to_lowercase())
                        .unwrap_or("?".into())
                ),
                position: Point::new(p.x + 10. * zoom, p.y + 28. * zoom),
                font: super::style::FONT,
                size: (11. * zoom).into(),
                color: super::style::muted(theme),
                ..Default::default()
            });
            if !matches!(n.op, wisp_ir::Op::Output { .. }) {
                f.fill(
                    &Path::circle(Point::new(p.x + WIDTH * zoom, p.y + 28. * zoom), 5. * zoom),
                    color(doc.value_type(&n.id)),
                );
            }
            for (i, port) in ports(&n.op).iter().enumerate() {
                let pos = Point::new(p.x, p.y + (60. + i as f32 * 24.) * zoom);
                f.fill(
                    &Path::circle(pos, 5. * zoom),
                    color(doc.input_type(&n.id, port)),
                );
                f.fill_text(Text {
                    content: format!(
                        "{port} · {}",
                        doc.input_type(&n.id, port)
                            .map(|t| format!("{t:?}").to_lowercase())
                            .unwrap_or("same type".into())
                    ),
                    position: Point::new(pos.x + 10. * zoom, pos.y - 8. * zoom),
                    font: super::style::FONT,
                    size: (12. * zoom).into(),
                    color: theme.palette().text,
                    ..Default::default()
                });
            }
        }
        vec![f.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::Program;
    #[test]
    fn canvas_emits_selection_move_wiring_pan_and_zoom() {
        let editor = GraphEditor {
            document: Some(std::sync::Arc::new(crate::graph::Document::default())),
            zoom: 1.,
            ..Default::default()
        };
        let canvas = GraphCanvas(&editor, false);
        let mut state = State::default();
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(700., 400.));
        let send = |state: &mut State, event: canvas::Event, p: Point| {
            canvas
                .update(state, &event, bounds, mouse::Cursor::Available(p))
                .unwrap()
                .into_inner()
                .0
                .unwrap()
        };
        assert!(
            matches!(send(&mut state,canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),Point::new(50.,40.)),Message::Graph(GraphMessage::Select(id)) if id=="color")
        );
        assert!(
            matches!(send(&mut state,canvas::Event::Mouse(mouse::Event::CursorMoved{position:Point::new(80.,70.)}),Point::new(80.,70.)),Message::Graph(GraphMessage::Move(id,p)) if id=="color" && p==[60.,60.])
        );
        canvas.update(
            &mut state,
            &canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            bounds,
            mouse::Cursor::Unavailable,
        );
        assert!(state.drag.is_none());
        assert!(
            matches!(send(&mut state,canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),Point::new(236.,58.)),Message::Graph(GraphMessage::Wire(id)) if id=="color")
        );
        assert!(
            matches!(send(&mut state,canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),Point::new(280.,90.)),Message::Graph(GraphMessage::Socket(id,port)) if id=="output" && port=="color")
        );
        let _ = send(
            &mut state,
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            Point::new(650., 350.),
        );
        assert!(
            matches!(send(&mut state,canvas::Event::Mouse(mouse::Event::CursorMoved{position:Point::new(640.,340.)}),Point::new(640.,340.)),Message::Graph(GraphMessage::Pan(v)) if v==iced::Vector::new(-10.,-10.))
        );
        assert!(matches!(
            send(
                &mut state,
                canvas::Event::Mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Lines { x: 0., y: 1. }
                }),
                Point::new(400., 200.)
            ),
            Message::Graph(GraphMessage::Zoom(1., _))
        ));
    }
}
