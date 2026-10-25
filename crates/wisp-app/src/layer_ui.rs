use super::*;
use crate::layers::Stack;
use iced::widget::column;
use iced::{
    Color, Point, Rectangle, mouse,
    widget::{
        Action,
        canvas::{self, Frame, Path, Stroke},
    },
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::ui) enum LayerTab {
    Adjust,
    #[default]
    Motion,
    Mask,
}

#[derive(Clone)]
pub(in crate::ui) enum LayerMessage {
    Tab(LayerTab),
    Enable,
    Select(usize),
    Import(Option<(usize, bool)>),
    Imported(Outcome<Option<Arc<Draft>>>),
    Name(String),
    Visible(usize, bool),
    Move(bool),
    Remove,
    Blend(wisp_ir::BlendMode),
    Effect(crate::layers::Effect),
    Value(String, f32),
    Commit,
    Mask(bool),
    MaskFlag(usize, bool),
    ClearMask,
    Brush(f32),
    Erase(bool),
    Paint([f32; 2], [f32; 2]),
    StrokeEnd,
    Expand(bool),
}
impl App {
    pub(super) fn refresh_painter(&mut self) {
        self.designer.paint_source = None;
        self.designer.paint_mask = None;
        self.designer.paint_pixels = None;
        self.designer.paint_asset = None;
        self.designer.paint_dirty = false;
        if let Some(s) = self.designer.draft.as_ref().and_then(|d| d.layers.as_ref())
            && let Some(l) = s.layers.get(self.designer.layer)
        {
            if let Some(a) = s.assets.get(&l.asset)
                && let Ok(source) = wisp_compiler::decode_source_image(&a.bytes)
            {
                self.designer.paint_dimensions = [source.width(), source.height()];
                let source = source.thumbnail(2048, 2048).into_rgba8();
                self.designer.paint_source = Some(image::Handle::from_rgba(
                    source.width(),
                    source.height(),
                    source.into_raw(),
                ));
            }
            if let Some(m) = &l.mask
                && let Some(a) = s.assets.get(&m.asset)
                && let Ok(mask) = wisp_compiler::decode_source_image(&a.bytes)
            {
                self.designer.paint_pixels = Some(
                    mask.resize_exact(1024, 1024, ::image::imageops::FilterType::Triangle)
                        .into_luma8(),
                );
                self.designer.paint_asset = Some(m.asset.clone());
            }
        }
        self.refresh_mask_overlay(None);
    }
    fn refresh_mask_overlay(&mut self, region: Option<[u32; 4]>) {
        if let Some(mask) = &self.designer.paint_pixels {
            let inverted = self
                .designer
                .draft
                .as_ref()
                .and_then(|d| d.layers.as_ref())
                .and_then(|s| s.layers.get(self.designer.layer))
                .and_then(|l| l.mask.as_ref())
                .is_some_and(|m| m.invert);
            self.designer
                .paint_overlay
                .resize((mask.width() * mask.height() * 4) as usize, 0);
            let [x0, y0, x1, y1] = region.unwrap_or([0, 0, mask.width(), mask.height()]);
            for y in y0..y1 {
                for x in x0..x1 {
                    let gray = mask.get_pixel(x, y).0[0];
                    let coverage = if inverted { 255 - gray } else { gray };
                    let offset = ((y * mask.width() + x) * 4) as usize;
                    self.designer.paint_overlay[offset..offset + 4].copy_from_slice(&[
                        30,
                        220,
                        210,
                        (u16::from(coverage) * 115 / 255) as u8,
                    ]);
                }
            }
            self.designer.paint_mask = Some(image::Handle::from_rgba(
                mask.width(),
                mask.height(),
                self.designer.paint_overlay.clone(),
            ));
        }
    }
    /// Preserve the raster in the editable draft when switching layers or saving.
    pub(in crate::ui) fn flush_paint(&mut self) -> Result<()> {
        if !self.designer.paint_dirty {
            return Ok(());
        }
        let pixels = self
            .designer
            .paint_pixels
            .as_ref()
            .context("missing paint raster")?;
        let id = self
            .designer
            .paint_asset
            .as_ref()
            .context("missing mask asset")?;
        let asset = crate::layers::encode(::image::DynamicImage::ImageLuma8(pixels.clone()))?;
        let stack = Arc::make_mut(self.designer.draft.as_mut().context("missing draft")?)
            .layers
            .as_mut()
            .context("missing layer stack")?;
        Arc::make_mut(&mut stack.assets).insert(id.clone(), asset);
        self.designer.paint_dirty = false;
        Ok(())
    }
    pub(super) fn layer_update(&mut self, message: LayerMessage) -> Task<Message> {
        use LayerMessage::*;
        if let Tab(tab) = &message {
            self.designer.layer_tab = *tab;
            return Task::none();
        }
        let painting = matches!(
            message,
            Paint(..) | StrokeEnd | Brush(_) | Erase(_) | Expand(_)
        );
        if self.busy && !matches!(message, Imported(_)) && !(painting && self.designer.compiling) {
            return Task::none();
        }
        if !painting
            && !matches!(message, Imported(_))
            && let Err(e) = self.flush_paint()
        {
            self.status = format!("Could not preserve mask: {e:#}");
            return Task::none();
        }
        match message {
            Import(target) => {
                if let Some(draft) = self.designer.draft.clone() {
                    self.busy = true;
                    return Task::perform(
                        async move {
                            let Some(file) = rfd::AsyncFileDialog::new()
                                .set_title("Choose layer, mask or displacement image")
                                .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                                .pick_file()
                                .await
                            else {
                                return Ok(None);
                            };
                            let path = file.path().to_owned();
                            tokio::task::spawn_blocking(move || -> Result<_> {
                                let mut draft = (*draft).clone();
                                draft.enable_layers().import(&path, target)?;
                                Ok(Some(Arc::new(draft)))
                            })
                            .await
                            .map_err(|e| e.to_string())?
                            .map_err(|e| format!("{e:#}"))
                        },
                        |r| Message::Designer(DesignerMessage::Layer(Imported(r))),
                    );
                }
            }
            Imported(result) => {
                self.busy = false;
                match result {
                    Ok(Some(draft)) => {
                        self.designer.draft = Some(draft);
                        self.designer.dirty = true;
                        self.refresh_painter();
                        return self.compile_design();
                    }
                    Ok(None) => {}
                    Err(e) => self.status = format!("Could not import image: {e}"),
                }
            }
            Select(index) => {
                self.designer.layer = index;
                self.refresh_painter();
            }
            Expand(value) => self.designer.paint_expanded = value,
            StrokeEnd => {}
            Paint(from, to) => {
                if let Some(pixels) = &mut self.designer.paint_pixels {
                    let [w, h] = self.designer.paint_dimensions;
                    let region = match crate::layers::paint_raster(
                        pixels,
                        from,
                        to,
                        self.designer.brush.max(0.005),
                        if self.designer.erase { 0 } else { 255 },
                        w.max(1) as f32 / h.max(1) as f32,
                    ) {
                        Ok(region) => region,
                        Err(e) => {
                            self.status = format!("Could not paint mask: {e:#}");
                            return Task::none();
                        }
                    };
                    self.designer.paint_dirty = true;
                    self.designer.paint_pending = true;
                    self.designer.dirty = true;
                    self.refresh_mask_overlay(Some(region));
                }
            }
            Brush(v) => self.designer.brush = v.clamp(0.005, 0.25),
            Erase(v) => self.designer.erase = v,
            Commit => return self.compile_design(),
            _ => {
                let Some(draft) = &mut self.designer.draft else {
                    return Task::none();
                };
                let draft = Arc::make_mut(draft);
                let stack = draft.enable_layers();
                let i = self.designer.layer.min(stack.layers.len() - 1);
                self.designer.layer = i;
                let refresh = matches!(
                    message,
                    Enable | Mask(_) | ClearMask | MaskFlag(..) | Move(_) | Remove | Effect(_)
                );
                let mut compile = true;
                let result = (|| -> Result<()> {
                    match message {
                        Enable => {}
                        Name(v) => {
                            stack.layers[i].name = v;
                            compile = false;
                        }
                        Visible(index, v) => {
                            if let Some(l) = stack.layers.get_mut(index) {
                                l.visible = v;
                            }
                        }
                        Move(up) => {
                            let target = if up {
                                (i + 1).min(stack.layers.len() - 1)
                            } else {
                                i.saturating_sub(1)
                            };
                            stack.layers.swap(i, target);
                            self.designer.layer = target;
                        }
                        Remove if stack.layers.len() > 1 => {
                            stack.layers.remove(i);
                            self.designer.layer = i.min(stack.layers.len() - 1);
                            stack.prune();
                        }
                        Blend(v) => stack.layers[i].blend = v,
                        Effect(v) => {
                            stack.layers[i].preset(v);
                            if matches!(
                                v,
                                crate::layers::Effect::Hair | crate::layers::Effect::Clothing
                            ) && stack.layers[i].mask.is_none()
                            {
                                stack.blank_mask(i, false)?;
                            }
                        }
                        Value(id, v) => {
                            stack.layers[i].set_value(&id, v)?;
                            compile = false;
                        }
                        Mask(white) => stack.blank_mask(i, white)?,
                        ClearMask => {
                            stack.layers[i].mask = None;
                            stack.prune();
                        }
                        MaskFlag(flag, v) => {
                            if let Some(m) = &mut stack.layers[i].mask {
                                match flag {
                                    0 => m.invert = v,
                                    1 => m.composition = v,
                                    _ => m.effect = v,
                                }
                            }
                        }
                        _ => {}
                    }
                    Ok(())
                })();
                match result {
                    Ok(()) => {
                        self.designer.dirty = true;
                        if refresh {
                            self.refresh_painter();
                        }
                        if compile {
                            return self.compile_design();
                        }
                    }
                    Err(e) => self.status = format!("Could not edit layer: {e:#}"),
                }
            }
        }
        Task::none()
    }
    pub(super) fn layers_view<'a>(&'a self, stack: &'a Stack) -> Element<'a, Message> {
        use LayerMessage::*;
        let action = |m| Message::Designer(DesignerMessage::Layer(m));
        let enabled = !self.busy;
        let i = self.designer.layer.min(stack.layers.len() - 1);
        let l = &stack.layers[i];
        let tab = self.designer.layer_tab;
        let mut panel = column![
            row![
                text("Front to back").size(11),
                space().width(Fill),
                button("+ Image").on_press_maybe(
                    (enabled && stack.layers.len() < 6).then_some(action(Import(None)))
                )
            ]
            .align_y(iced::Center),
        ]
        .spacing(10);
        for (index, layer) in stack.layers.iter().enumerate().rev() {
            panel = panel.push(
                row![
                    checkbox(layer.visible).on_toggle(move |v| action(Visible(index, v))),
                    button(text(&layer.name).size(14))
                        .width(Fill)
                        .style(if i == index {
                            style::selected
                        } else {
                            style::quiet
                        })
                        .on_press_maybe(enabled.then_some(action(Select(index)))),
                ]
                .spacing(8)
                .align_y(iced::Center),
            );
        }
        panel = panel.push(
            row![
                button("Forward").on_press_maybe(
                    (enabled && i + 1 < stack.layers.len()).then_some(action(Move(true)))
                ),
                button("Back").on_press_maybe((enabled && i > 0).then_some(action(Move(false)))),
                button("Remove")
                    .on_press_maybe((enabled && stack.layers.len() > 1).then_some(action(Remove))),
            ]
            .spacing(2),
        );
        let tab_button = |label, value| {
            button(text(label).size(12))
                .width(Fill)
                .style(if tab == value {
                    style::selected
                } else {
                    style::quiet
                })
                .on_press(action(Tab(value)))
        };
        panel = panel.push(iced::widget::rule::horizontal(1)).push(
            row![
                tab_button("Adjust", LayerTab::Adjust),
                tab_button("Motion", LayerTab::Motion),
                tab_button("Mask", LayerTab::Mask)
            ]
            .spacing(4),
        );
        if tab == LayerTab::Adjust {
            panel = panel
                .push(text_input("Layer name", &l.name).on_input(move |v| action(Name(v))))
                .push(
                    pick_list(
                        wisp_ir::BlendMode::ALL.as_slice(),
                        Some(l.blend),
                        move |v| action(Blend(v)),
                    )
                    .width(Fill),
                );
        }
        if tab == LayerTab::Motion {
            panel = panel.push(
                pick_list(
                    crate::layers::Effect::ALL.as_slice(),
                    Some(l.effect),
                    move |v| action(Effect(v)),
                )
                .width(Fill),
            );
            if l.effect == crate::layers::Effect::None {
                panel = panel.push(text("Choose an effect to bring this layer to life.").size(13));
            }
        }
        for c in crate::layers::CONTROLS {
            let transform =
                ["opacity", "x", "y", "sx", "sy", "rotation", "px", "py"].contains(&c.0);
            if (tab == LayerTab::Adjust && !transform)
                || (tab == LayerTab::Motion && (transform || c.0 == "feather"))
                || (tab == LayerTab::Mask && c.0 != "feather")
            {
                continue;
            }
            let weather = matches!(
                l.effect,
                crate::layers::Effect::Droplets
                    | crate::layers::Effect::Rain
                    | crate::layers::Effect::Ripples
            );
            if (c.0 == "feather" && l.mask.is_none())
                || (["density", "size", "size_x", "size_y"].contains(&c.0) && !weather)
                || (c.0 == "rain_direction" && l.effect != crate::layers::Effect::Rain)
                || (["direction", "amplitude", "frequency"].contains(&c.0)
                    && !matches!(
                        l.effect,
                        crate::layers::Effect::Hair
                            | crate::layers::Effect::Clothing
                            | crate::layers::Effect::Wave
                            | crate::layers::Effect::Warp
                    ))
                || (c.0 == "speed"
                    && matches!(
                        l.effect,
                        crate::layers::Effect::None | crate::layers::Effect::Displacement
                    ))
                || (c.0 == "strength"
                    && !weather
                    && l.effect != crate::layers::Effect::Displacement)
            {
                continue;
            }
            panel = panel.push(
                column![
                    row![
                        text(c.1).size(12),
                        space().width(Fill),
                        text(format!("{:.2}", l.value(c.0))).size(12)
                    ]
                    .spacing(8),
                    slider(c.2..=c.3, l.value(c.0), move |v| action(Value(
                        c.0.into(),
                        v
                    )))
                    .step((c.3 - c.2) / 200.)
                    .on_release(action(Commit)),
                ]
                .spacing(7),
            );
        }
        if tab == LayerTab::Motion && l.effect == crate::layers::Effect::Displacement {
            panel = panel.push(
                button("Choose displacement map…")
                    .on_press_maybe(enabled.then_some(action(Import(Some((i, true)))))),
            );
        }
        if tab == LayerTab::Mask {
            panel = panel.push(text("Paint where this layer moves or appears.").size(13));
            if let Some(m) = &l.mask {
                panel = panel
                    .push(
                        button("Paint mask…")
                            .style(iced_button::primary)
                            .width(Fill)
                            .on_press(action(Expand(true))),
                    )
                    .push(
                        checkbox(m.invert)
                            .label("Invert")
                            .on_toggle(move |v| action(MaskFlag(0, v))),
                    )
                    .push(
                        checkbox(m.composition)
                            .label("Affect layer opacity")
                            .on_toggle(move |v| action(MaskFlag(1, v))),
                    )
                    .push(
                        checkbox(m.effect)
                            .label("Affect motion strength")
                            .on_toggle(move |v| action(MaskFlag(2, v))),
                    )
                    .push(
                        button("Fill white").on_press_maybe(enabled.then_some(action(Mask(true)))),
                    )
                    .push(
                        button("Clear to black")
                            .on_press_maybe(enabled.then_some(action(Mask(false)))),
                    )
                    .push(
                        button("Remove mask").on_press_maybe(enabled.then_some(action(ClearMask))),
                    );
            } else {
                panel = panel.push(
                    button("Create a mask")
                        .style(iced_button::primary)
                        .width(Fill)
                        .on_press_maybe(enabled.then_some(action(Mask(false)))),
                );
            }
            panel = panel
                .push(
                    button("Import mask…")
                        .on_press_maybe(enabled.then_some(action(Import(Some((i, false)))))),
                )
                .push(text("White reveals · Black anchors").size(11));
        }
        panel.into()
    }
    pub(super) fn expanded_painter(&self) -> Element<'_, Message> {
        let action = |m| Message::Designer(DesignerMessage::Layer(m));
        let canvas = canvas::Canvas::new(MaskCanvas {
            designer: &self.designer,
            enabled: !self.busy || self.designer.compiling,
        })
        .width(Fill)
        .height(Fill);
        if self.focus_mode {
            return iced::widget::stack![
                canvas,
                container(button("Show controls · F11").on_press(Message::Focus))
                    .padding(12)
                    .align_right(Fill),
            ]
            .into();
        }
        column![
            row![
                button("Done").on_press(action(LayerMessage::Expand(false))),
                text("Paint mask").size(15),
                space().width(Fill),
                button("Paint")
                    .style(if !self.designer.erase {
                        style::selected
                    } else {
                        style::quiet
                    })
                    .on_press(action(LayerMessage::Erase(false))),
                button("Erase")
                    .style(if self.designer.erase {
                        style::selected
                    } else {
                        style::quiet
                    })
                    .on_press(action(LayerMessage::Erase(true))),
                text("Size").size(12),
                slider(0.005..=0.25, self.designer.brush.max(0.005), move |v| {
                    action(LayerMessage::Brush(v))
                })
                .width(140)
                .step(0.005),
                button("Focus").on_press(Message::Focus),
            ]
            .spacing(10)
            .align_y(iced::Center)
            .padding([4, 8]),
            canvas,
            text("Teal shows the selected area · Live preview updates as you paint").size(11),
        ]
        .spacing(8)
        .height(Fill)
        .into()
    }
}
struct MaskCanvas<'a> {
    designer: &'a Designer,
    enabled: bool,
}
impl MaskCanvas<'_> {
    fn image_rect(&self, size: iced::Size) -> Rectangle {
        let [w, h] = self.designer.paint_dimensions;
        if w == 0 || h == 0 {
            return Rectangle::new(Point::ORIGIN, size);
        }
        let scale = (size.width / w as f32).min(size.height / h as f32);
        let fitted = iced::Size::new(w as f32 * scale, h as f32 * scale);
        Rectangle::new(
            Point::new(
                (size.width - fitted.width) * 0.5,
                (size.height - fitted.height) * 0.5,
            ),
            fitted,
        )
    }
}
#[derive(Default)]
struct PaintState {
    last: Option<[f32; 2]>,
}
impl canvas::Program<Message> for MaskCanvas<'_> {
    type State = PaintState;
    fn update(
        &self,
        state: &mut PaintState,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        let emit =
            |m| Some(Action::publish(Message::Designer(DesignerMessage::Layer(m))).and_capture());
        if matches!(
            event,
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
        ) && state.last.take().is_some()
        {
            return emit(LayerMessage::StrokeEnd);
        }
        if !self.enabled {
            state.last = None;
            return None;
        }
        let rect = self.image_rect(bounds.size());
        let p = cursor
            .position_in(bounds)
            .filter(|p| rect.contains(*p))
            .map(|p| [(p.x - rect.x) / rect.width, (p.y - rect.y) / rect.height]);
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = p?;
                state.last = Some(p);
                emit(LayerMessage::Paint(p, p))
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if state.last.is_some() => {
                let p = p?;
                let previous = state.last.replace(p).unwrap();
                emit(LayerMessage::Paint(previous, p))
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                Some(Action::request_redraw())
            }
            _ => None,
        }
    }
    fn draw(
        &self,
        _: &PaintState,
        renderer: &iced::Renderer,
        theme: &iced::Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        f.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            if theme.extended_palette().is_dark {
                Color::from_rgb8(18, 19, 21)
            } else {
                Color::from_rgb8(224, 223, 220)
            },
        );
        let rect = self.image_rect(bounds.size());
        if let Some(source) = &self.designer.paint_source {
            f.draw_image(rect, source);
        }
        if let Some(mask) = &self.designer.paint_mask {
            f.draw_image(rect, canvas::Image::new(mask.clone()));
        }
        if let Some(p) = cursor.position_in(bounds).filter(|p| rect.contains(*p)) {
            f.stroke(
                &Path::circle(
                    p,
                    self.designer.brush.max(0.005) * rect.width.min(rect.height),
                ),
                Stroke::default()
                    .with_color(Color::from_rgb(0.2, 0.9, 0.9))
                    .with_width(1.5),
            );
        }
        vec![f.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn brush_emits_continuous_layer_coordinates_and_commits_outside_canvas() {
        let designer = Designer {
            brush: 0.06,
            ..Default::default()
        };
        let canvas = MaskCanvas {
            designer: &designer,
            enabled: true,
        };
        let mut state = PaintState::default();
        let bounds = Rectangle {
            x: 10.,
            y: 20.,
            width: 200.,
            height: 100.,
        };
        use canvas::Program;
        let action = canvas
            .update(
                &mut state,
                &canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                bounds,
                mouse::Cursor::Available(Point::new(60., 45.)),
            )
            .unwrap();
        let _ = action;
        assert_eq!(state.last, Some([0.25, 0.25]));
        assert!(
            canvas
                .update(
                    &mut state,
                    &canvas::Event::Mouse(mouse::Event::CursorMoved {
                        position: Point::new(110., 70.)
                    }),
                    bounds,
                    mouse::Cursor::Available(Point::new(110., 70.))
                )
                .is_some()
        );
        assert_eq!(state.last, Some([0.5, 0.5]));
        assert!(
            canvas
                .update(
                    &mut state,
                    &canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                    bounds,
                    mouse::Cursor::Unavailable
                )
                .is_some()
        );
        assert!(state.last.is_none());
        let blocked = MaskCanvas {
            designer: &designer,
            enabled: false,
        };
        assert!(
            blocked
                .update(
                    &mut state,
                    &canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                    bounds,
                    mouse::Cursor::Available(Point::new(60., 45.))
                )
                .is_none()
        );
    }
}

#[cfg(test)]
mod live_paint_tests {
    use super::*;
    #[test]
    fn strokes_keep_source_visible_and_continue_during_throttled_compilation() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("image.png");
        std::fs::write(&source, wisp_runtime::EXAMPLE_IMAGE).unwrap();
        let mut draft = Draft::from_image(&source).unwrap();
        draft.enable_layers().blank_mask(0, false).unwrap();
        let wallpaper =
            Arc::new(Wallpaper::from_package(&draft.compile().unwrap().package).unwrap());
        let library = Library::open(Some(dir.path().join("library"))).unwrap();
        let (mut app, _) = App::new(library, Settings::default(), None, None);
        app.busy = false;
        let _ = app.designer_update(DesignerMessage::Opened(Ok(Some((Arc::new(draft), true)))));
        let _ = app.designer_update(DesignerMessage::Compiled(Ok((
            wallpaper.clone(),
            "ready".into(),
        ))));
        let now = Instant::now();
        app.designer.paint_last_compile = Some(now);
        let handle = app.designer.paint_source.clone();
        let asset = app.designer.paint_asset.clone().unwrap();
        let old_bytes = app
            .designer
            .draft
            .as_ref()
            .unwrap()
            .layers
            .as_ref()
            .unwrap()
            .assets[&asset]
            .bytes
            .clone();
        let _ = app.layer_update(LayerMessage::Paint([0.2, 0.2], [0.4, 0.4]));
        assert!(app.designer.paint_source == handle);
        assert!(app.designer.paint_dirty && app.designer.paint_pending && !app.busy);
        assert!(
            app.designer
                .paint_pixels
                .as_ref()
                .unwrap()
                .get_pixel(307, 307)
                .0[0]
                > 0
        );
        assert_eq!(
            old_bytes,
            app.designer
                .draft
                .as_ref()
                .unwrap()
                .layers
                .as_ref()
                .unwrap()
                .assets[&asset]
                .bytes
        );
        let _ = app.layer_update(LayerMessage::StrokeEnd);
        let _ = app.designer_update(DesignerMessage::PaintTick(now + Duration::from_secs(1)));
        assert!(!app.busy);
        let _ = app.designer_update(DesignerMessage::PaintTick(now + Duration::from_secs(2)));
        assert!(app.busy && app.designer.compiling);
        assert!(app.designer.paint_dirty && !app.designer.paint_pending);
        let _ = app.layer_update(LayerMessage::Paint([0.7, 0.7], [0.8, 0.8]));
        assert!(app.designer.paint_source == handle);
        assert!(app.designer.paint_dirty && app.designer.paint_pending);
        assert!(
            app.designer
                .paint_pixels
                .as_ref()
                .unwrap()
                .get_pixel(768, 768)
                .0[0]
                > 0
        );
        let _ = app.designer_update(DesignerMessage::Compiled(Ok((wallpaper, "ready".into()))));
        assert!(app.designer.paint_pending && app.designer.paint_dirty);
        app.flush_paint().unwrap();
        let saved = dir.path().join("saved");
        app.designer
            .draft
            .as_ref()
            .unwrap()
            .save_project(&saved)
            .unwrap();
        let reopened = Draft::open_project(&saved).unwrap();
        let pixels = ::image::load_from_memory(&reopened.layers.unwrap().assets[&asset].bytes)
            .unwrap()
            .into_luma8();
        assert!(pixels.get_pixel(307, 307).0[0] > 0 && pixels.get_pixel(768, 768).0[0] > 0);
    }
    #[test]
    fn expanded_painter_fits_aspect_and_maps_coordinates_without_letterbox_strokes() {
        use canvas::Program;
        let designer = Designer {
            paint_dimensions: [1600, 800],
            ..Default::default()
        };
        let canvas = MaskCanvas {
            designer: &designer,
            enabled: true,
        };
        let rect = canvas.image_rect(iced::Size::new(800., 800.));
        assert_eq!(
            rect,
            Rectangle {
                x: 0.,
                y: 200.,
                width: 800.,
                height: 400.
            }
        );
        let bounds = Rectangle::new(Point::ORIGIN, iced::Size::new(800., 800.));
        let press = canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let mut state = PaintState::default();
        assert!(
            canvas
                .update(
                    &mut state,
                    &press,
                    bounds,
                    mouse::Cursor::Available(Point::new(200., 100.))
                )
                .is_none()
        );
        assert!(
            canvas
                .update(
                    &mut state,
                    &press,
                    bounds,
                    mouse::Cursor::Available(Point::new(200., 300.))
                )
                .is_some()
        );
        assert_eq!(state.last, Some([0.25, 0.25]));
    }
}
