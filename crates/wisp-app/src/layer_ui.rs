use super::*;
use crate::layers::Stack;
use crate::painting::PaintTool;
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
    Tool(PaintTool),
    Flow(f32),
    WandTolerance(u8),
    WandAllMatching(bool),
    WandSelect([f32; 2]),
    LassoPoint([f32; 2]),
    LassoFinish,
    LassoCancel,
    LassoUndo,
    Erase(bool),
    Paint([f32; 2], [f32; 2]),
    StrokeEnd,
    Expand(bool),
}
impl App {
    pub(super) fn refresh_painter(&mut self) {
        self.designer.lasso_vertices.clear();
        self.designer.paint_session += 1;
        self.designer.paint_source = None;
        self.designer.paint_source_pixels = None;
        self.designer.paint_dimensions = [0, 0];
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
                let source = if source.width() > 2048 || source.height() > 2048 {
                    source.thumbnail(2048, 2048)
                } else {
                    source
                }
                .into_rgba8();
                self.designer.paint_source = Some(image::Handle::from_rgba(
                    source.width(),
                    source.height(),
                    source.as_raw().clone(),
                ));
                self.designer.paint_source_pixels = Some(source);
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
            Paint(..)
                | StrokeEnd
                | Brush(_)
                | Tool(_)
                | Flow(_)
                | WandTolerance(_)
                | WandAllMatching(_)
                | WandSelect(_)
                | Erase(_)
                | Expand(_)
                | LassoPoint(_)
                | LassoFinish
                | LassoCancel
                | LassoUndo
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
            Expand(value) => {
                self.designer.paint_expanded = value;
                self.designer.paint_session += 1;
            }
            StrokeEnd => {}
            Tool(tool) => {
                self.designer.paint_tool = tool;
                self.designer.lasso_vertices.clear();
                self.designer.paint_session += 1;
            }
            Flow(value) => self.designer.airbrush_flow = value.clamp(0.01, 0.5),
            WandTolerance(value) => self.designer.magic_wand.tolerance = value,
            WandAllMatching(value) => self.designer.magic_wand.all_matching = value,
            LassoPoint(point) => {
                if self.designer.paint_tool == PaintTool::PolygonalLasso
                    && point
                        .into_iter()
                        .all(|v| v.is_finite() && (0.0..=1.0).contains(&v))
                {
                    self.designer.lasso_vertices.push(point);
                }
            }
            LassoCancel => self.designer.lasso_vertices.clear(),
            LassoUndo => {
                self.designer.lasso_vertices.pop();
            }
            Paint(..) | LassoFinish | WandSelect(_) => {
                let flow = self.designer.airbrush_flow();
                if let Some(pixels) = &mut self.designer.paint_pixels {
                    let [w, h] = self.designer.paint_dimensions;
                    let value = if self.designer.erase { 0 } else { 255 };
                    let result = match message {
                        Paint(from, to) if self.designer.paint_tool.is_brush() => {
                            crate::painting::paint_brush(
                                pixels,
                                from,
                                to,
                                self.designer.brush.max(0.005),
                                value,
                                w.max(1) as f32 / h.max(1) as f32,
                                (self.designer.paint_tool == PaintTool::Airbrush).then_some(flow),
                            )
                        }
                        LassoFinish
                            if self.designer.paint_tool == PaintTool::PolygonalLasso
                                && self.designer.lasso_vertices.len() >= 3 =>
                        {
                            crate::painting::fill_polygon(
                                pixels,
                                &self.designer.lasso_vertices,
                                value,
                            )
                        }
                        WandSelect(point) if self.designer.paint_tool == PaintTool::MagicWand => {
                            let Some(source) = &self.designer.paint_source_pixels else {
                                self.status = "Magic wand needs a source image.".into();
                                return Task::none();
                            };
                            crate::painting::fill_magic_wand(
                                pixels,
                                source,
                                point,
                                self.designer.magic_wand,
                                value,
                            )
                        }
                        _ => return Task::none(),
                    };
                    let region = match result {
                        Ok(region) => region,
                        Err(e) => {
                            self.status = format!("Could not paint mask: {e:#}");
                            return Task::none();
                        }
                    };
                    if region == [0; 4] {
                        return Task::none();
                    }
                    self.designer.lasso_vertices.clear();
                    self.designer.paint_dirty = true;
                    self.designer.paint_pending = true;
                    self.designer.dirty = true;
                    self.refresh_mask_overlay(Some(region));
                }
            }
            Brush(v) => self.designer.brush = v.clamp(0.005, 0.25),
            Erase(v) => {
                self.designer.erase = v;
                self.designer.paint_session += 1;
            }
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
    fn paint_controls(&self) -> Element<'_, Message> {
        use LayerMessage::*;
        let action = |message| Message::Designer(DesignerMessage::Layer(message));
        let mut tools = row![pick_list(
            PaintTool::ALL.as_slice(),
            Some(self.designer.paint_tool),
            move |tool| action(Tool(tool))
        )]
        .spacing(12)
        .align_y(iced::Center);
        if self.designer.paint_tool.is_brush() {
            tools = tools.push(text("Size").size(12)).push(
                slider(0.005..=0.25, self.designer.brush.max(0.005), move |value| {
                    action(Brush(value))
                })
                .width(140)
                .step(0.005),
            );
            if self.designer.paint_tool == PaintTool::Airbrush {
                tools = tools
                    .push(
                        text(format!("Flow {:.0}%", self.designer.airbrush_flow() * 100.)).size(12),
                    )
                    .push(
                        slider(0.01..=0.5, self.designer.airbrush_flow(), move |value| {
                            action(Flow(value))
                        })
                        .width(140)
                        .step(0.01),
                    );
            }
        }
        if self.designer.paint_tool == PaintTool::MagicWand {
            tools = tools
                .push(text(format!("Tolerance {}", self.designer.magic_wand.tolerance)).size(12))
                .push(
                    slider(0..=255, self.designer.magic_wand.tolerance, move |value| {
                        action(WandTolerance(value))
                    })
                    .width(140),
                )
                .push(
                    checkbox(self.designer.magic_wand.all_matching)
                        .label("All matching colors")
                        .on_toggle(move |value| action(WandAllMatching(value))),
                );
        }
        let mut controls = column![tools].spacing(8).padding([0, 8]);
        if self.designer.paint_tool == PaintTool::PolygonalLasso {
            controls = controls
                .push(text("Click corners; click the first point or Enter to fill. Backspace undoes; Esc cancels.").size(11))
                .push(row![
                    button("Fill polygon").on_press_maybe((self.designer.lasso_vertices.len() >= 3).then_some(action(LassoFinish))),
                    button("Cancel polygon").on_press_maybe((!self.designer.lasso_vertices.is_empty()).then_some(action(LassoCancel)))
                ].spacing(8));
        }
        if self.designer.paint_tool == PaintTool::MagicWand {
            controls = controls.push(text("Click the source to select a connected color region. Increase tolerance to include similar colors; use Erase to subtract.").size(11));
        }
        controls.into()
    }
    fn paint_canvas(&self, height: iced::Length) -> Element<'_, Message> {
        // The renderer batches images above paths within a canvas, so guides need their own layer.
        iced::widget::stack![
            canvas::Canvas::new(MaskBackground(&self.designer))
                .width(Fill)
                .height(height),
            canvas::Canvas::new(MaskCanvas {
                designer: &self.designer,
                enabled: !self.busy || self.designer.compiling,
            })
            .width(Fill)
            .height(height)
        ]
        .into()
    }
    pub(super) fn expanded_painter(&self) -> Element<'_, Message> {
        let action = |m| Message::Designer(DesignerMessage::Layer(m));
        let canvas = self.paint_canvas(Fill);
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
                button("Focus").on_press(Message::Focus),
            ]
            .spacing(10)
            .align_y(iced::Center)
            .padding([4, 8]),
            self.paint_controls(),
            canvas,
            text("Teal shows the selected area · Live preview updates as you paint").size(11),
        ]
        .spacing(8)
        .height(Fill)
        .into()
    }
}
struct MaskBackground<'a>(&'a Designer);
impl canvas::Program<Message> for MaskBackground<'_> {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        theme: &iced::Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            if theme.extended_palette().is_dark {
                Color::from_rgb8(18, 19, 21)
            } else {
                Color::from_rgb8(224, 223, 220)
            },
        );
        let rect = fitted_image_rect(self.0.paint_dimensions, bounds.size());
        if let Some(source) = &self.0.paint_source {
            frame.draw_image(rect, source);
        }
        if let Some(mask) = &self.0.paint_mask {
            frame.draw_image(rect, canvas::Image::new(mask.clone()));
        }
        vec![frame.into_geometry()]
    }
}
struct MaskCanvas<'a> {
    designer: &'a Designer,
    enabled: bool,
}
impl MaskCanvas<'_> {
    fn image_rect(&self, size: iced::Size) -> Rectangle {
        fitted_image_rect(self.designer.paint_dimensions, size)
    }
}
fn fitted_image_rect([w, h]: [u32; 2], size: iced::Size) -> Rectangle {
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
#[derive(Default)]
struct PaintState {
    last: Option<[f32; 2]>,
    next_spray: Option<Instant>,
    session: u64,
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
        if state.session != self.designer.paint_session {
            *state = PaintState {
                session: self.designer.paint_session,
                ..Default::default()
            };
        }
        if matches!(event, canvas::Event::Window(iced::window::Event::Unfocused)) {
            state.last = None;
            state.next_spray = None;
            return None;
        }
        if matches!(
            event,
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
        ) && state.last.take().is_some()
        {
            state.next_spray = None;
            return emit(LayerMessage::StrokeEnd);
        }
        if !self.enabled {
            state.last = None;
            state.next_spray = None;
            return None;
        }
        let rect = self.image_rect(bounds.size());
        let p = cursor
            .position_in(bounds)
            .filter(|p| rect.contains(*p))
            .map(|p| [(p.x - rect.x) / rect.width, (p.y - rect.y) / rect.height]);
        if self.designer.paint_tool == PaintTool::MagicWand {
            return match event {
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    emit(LayerMessage::WandSelect(p?))
                }
                canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                    Some(Action::request_redraw())
                }
                _ => None,
            };
        }
        if self.designer.paint_tool == PaintTool::PolygonalLasso {
            return match event {
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    let point = p?;
                    let vertices = &self.designer.lasso_vertices;
                    if vertices.len() >= 3
                        && ((point[0] - vertices[0][0]) * rect.width)
                            .hypot((point[1] - vertices[0][1]) * rect.height)
                            <= 8.
                    {
                        emit(LayerMessage::LassoFinish)
                    } else {
                        emit(LayerMessage::LassoPoint(point))
                    }
                }
                canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. })
                    if !self.designer.lasso_vertices.is_empty() =>
                {
                    use iced::keyboard::{Key, key::Named};
                    match key {
                        Key::Named(Named::Enter) => emit(LayerMessage::LassoFinish),
                        Key::Named(Named::Escape) => emit(LayerMessage::LassoCancel),
                        Key::Named(Named::Backspace) => emit(LayerMessage::LassoUndo),
                        _ => None,
                    }
                }
                canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                    Some(Action::request_redraw())
                }
                _ => None,
            };
        }
        const SPRAY_INTERVAL: Duration = Duration::from_millis(33);
        let airbrush = self.designer.paint_tool == PaintTool::Airbrush;
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = p?;
                state.last = Some(p);
                state.next_spray = airbrush.then(|| Instant::now() + SPRAY_INTERVAL);
                emit(LayerMessage::Paint(p, p))
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if state.last.is_some() => {
                let p = p?;
                let previous = state.last.replace(p).unwrap();
                state.next_spray = airbrush.then(|| Instant::now() + SPRAY_INTERVAL);
                emit(LayerMessage::Paint(previous, p))
            }
            canvas::Event::Window(iced::window::Event::RedrawRequested(now))
                if airbrush && state.last.is_some() =>
            {
                let next = state.next_spray?;
                if *now < next {
                    Some(Action::request_redraw_at(next))
                } else {
                    state.next_spray = Some(*now + SPRAY_INTERVAL);
                    if let Some(p) = p {
                        state.last = Some(p);
                        emit(LayerMessage::Paint(p, p))
                    } else {
                        Some(Action::request_redraw_at(*now + SPRAY_INTERVAL))
                    }
                }
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
        _: &iced::Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let rect = self.image_rect(bounds.size());
        let color = Color::from_rgb(0.2, 0.9, 0.9);
        if self.designer.paint_tool == PaintTool::PolygonalLasso {
            let screen = |point: [f32; 2]| {
                Point::new(
                    rect.x + point[0] * rect.width,
                    rect.y + point[1] * rect.height,
                )
            };
            let vertices = &self.designer.lasso_vertices;
            if let Some(first) = vertices.first() {
                let outline = Path::new(|builder| {
                    builder.move_to(screen(*first));
                    for point in &vertices[1..] {
                        builder.line_to(screen(*point));
                    }
                    if let Some(point) = cursor
                        .position_in(bounds)
                        .filter(|point| rect.contains(*point))
                    {
                        builder.line_to(point);
                    }
                    builder.close();
                });
                f.stroke(
                    &outline,
                    Stroke::default().with_color(Color::BLACK).with_width(3.),
                );
                f.stroke(
                    &outline,
                    Stroke::default().with_color(color).with_width(1.5),
                );
                for (index, point) in vertices.iter().enumerate() {
                    f.fill(
                        &Path::circle(screen(*point), if index == 0 { 5. } else { 3. }),
                        color,
                    );
                }
            }
        } else if let Some(p) = cursor.position_in(bounds).filter(|p| rect.contains(*p)) {
            if self.designer.paint_tool == PaintTool::MagicWand {
                let crosshair = Path::new(|builder| {
                    builder.move_to(Point::new(p.x - 7., p.y));
                    builder.line_to(Point::new(p.x + 7., p.y));
                    builder.move_to(Point::new(p.x, p.y - 7.));
                    builder.line_to(Point::new(p.x, p.y + 7.));
                });
                f.stroke(
                    &crosshair,
                    Stroke::default().with_color(Color::BLACK).with_width(3.),
                );
                f.stroke(
                    &crosshair,
                    Stroke::default().with_color(color).with_width(1.5),
                );
                return vec![f.into_geometry()];
            }
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
    fn wand_clicks_map_to_source_coordinates_and_ignore_drags_and_letterboxing() {
        use canvas::Program;
        let designer = Designer {
            paint_tool: PaintTool::MagicWand,
            paint_dimensions: [400, 200],
            ..Default::default()
        };
        let mut state = PaintState::default();
        let press = canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        assert!(
            canvas_message(
                &designer,
                &mut state,
                press.clone(),
                Some(Point::new(100., 25.))
            )
            .is_none()
        );
        assert!(matches!(
            canvas_message(
                &designer,
                &mut state,
                press.clone(),
                Some(Point::new(50., 75.))
            ),
            Some(LayerMessage::WandSelect([0.25, 0.25]))
        ));
        assert!(state.last.is_none() && state.next_spray.is_none());
        assert!(
            canvas_message(
                &designer,
                &mut state,
                canvas::Event::Mouse(mouse::Event::CursorMoved {
                    position: Point::new(70., 75.)
                }),
                Some(Point::new(70., 75.))
            )
            .is_none()
        );
        assert!(
            canvas_message(
                &designer,
                &mut state,
                canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                None
            )
            .is_none()
        );
        let blocked = MaskCanvas {
            designer: &designer,
            enabled: false,
        };
        assert!(
            blocked
                .update(
                    &mut state,
                    &press,
                    Rectangle::new(Point::ORIGIN, iced::Size::new(200., 200.)),
                    mouse::Cursor::Available(Point::new(50., 75.))
                )
                .is_none()
        );
    }

    fn canvas_message(
        designer: &Designer,
        state: &mut PaintState,
        event: canvas::Event,
        position: Option<Point>,
    ) -> Option<LayerMessage> {
        use canvas::Program;
        let canvas = MaskCanvas {
            designer,
            enabled: true,
        };
        let bounds = Rectangle::new(Point::ORIGIN, iced::Size::new(200., 200.));
        let action = canvas.update(
            state,
            &event,
            bounds,
            position
                .map(mouse::Cursor::Available)
                .unwrap_or(mouse::Cursor::Unavailable),
        )?;
        match action.into_inner().0 {
            Some(Message::Designer(DesignerMessage::Layer(message))) => Some(message),
            _ => None,
        }
    }

    #[test]
    fn airbrush_sprays_while_held_and_stops_on_release_focus_loss_or_context_change() {
        let mut designer = Designer {
            paint_tool: PaintTool::Airbrush,
            ..Default::default()
        };
        let mut state = PaintState::default();
        let point = Point::new(100., 100.);
        let press = canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        assert!(matches!(
            canvas_message(&designer, &mut state, press.clone(), Some(point)),
            Some(LayerMessage::Paint([0.5, 0.5], [0.5, 0.5]))
        ));
        let next = state.next_spray.unwrap();
        assert!(
            canvas_message(
                &designer,
                &mut state,
                canvas::Event::Window(iced::window::Event::RedrawRequested(
                    next - Duration::from_millis(1)
                )),
                Some(point)
            )
            .is_none()
        );
        assert!(matches!(
            canvas_message(
                &designer,
                &mut state,
                canvas::Event::Window(iced::window::Event::RedrawRequested(next)),
                Some(point)
            ),
            Some(LayerMessage::Paint(..))
        ));
        let next = state.next_spray.unwrap();
        assert!(
            canvas_message(
                &designer,
                &mut state,
                canvas::Event::Window(iced::window::Event::RedrawRequested(next)),
                None
            )
            .is_none()
        );
        assert!(matches!(
            canvas_message(
                &designer,
                &mut state,
                canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                None
            ),
            Some(LayerMessage::StrokeEnd)
        ));
        assert!(state.last.is_none() && state.next_spray.is_none());
        canvas_message(&designer, &mut state, press.clone(), Some(point));
        canvas_message(
            &designer,
            &mut state,
            canvas::Event::Window(iced::window::Event::Unfocused),
            Some(point),
        );
        assert!(state.last.is_none() && state.next_spray.is_none());
        canvas_message(&designer, &mut state, press, Some(point));
        designer.paint_session += 1;
        assert!(
            canvas_message(
                &designer,
                &mut state,
                canvas::Event::Window(iced::window::Event::RedrawRequested(
                    Instant::now() + Duration::from_secs(1)
                )),
                Some(point)
            )
            .is_none()
        );
        assert!(state.last.is_none() && state.next_spray.is_none());
    }

    #[test]
    fn lasso_places_vertices_without_brush_strokes_and_closes_near_first_corner() {
        let mut designer = Designer {
            paint_tool: PaintTool::PolygonalLasso,
            paint_dimensions: [400, 200],
            ..Default::default()
        };
        let mut state = PaintState::default();
        let press = canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        assert!(
            canvas_message(
                &designer,
                &mut state,
                press.clone(),
                Some(Point::new(20., 20.))
            )
            .is_none()
        );
        for point in [
            Point::new(20., 60.),
            Point::new(180., 60.),
            Point::new(100., 140.),
        ] {
            let Some(LayerMessage::LassoPoint(point)) =
                canvas_message(&designer, &mut state, press.clone(), Some(point))
            else {
                panic!("expected a lasso vertex")
            };
            designer.lasso_vertices.push(point);
            assert!(state.last.is_none());
            assert!(
                canvas_message(
                    &designer,
                    &mut state,
                    canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                    None
                )
                .is_none()
            );
        }
        assert_eq!(designer.lasso_vertices[0], [0.1, 0.1]);
        assert!(matches!(
            canvas_message(&designer, &mut state, press, Some(Point::new(22., 62.))),
            Some(LayerMessage::LassoFinish)
        ));
        for (key, expected) in [
            (iced::keyboard::key::Named::Enter, LayerMessage::LassoFinish),
            (
                iced::keyboard::key::Named::Escape,
                LayerMessage::LassoCancel,
            ),
            (
                iced::keyboard::key::Named::Backspace,
                LayerMessage::LassoUndo,
            ),
        ] {
            let event = canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(key),
                modified_key: iced::keyboard::Key::Named(key),
                physical_key: iced::keyboard::key::Physical::Unidentified(
                    iced::keyboard::key::NativeCode::Unidentified,
                ),
                location: iced::keyboard::Location::Standard,
                modifiers: iced::keyboard::Modifiers::empty(),
                text: None,
                repeat: false,
            });
            let message = canvas_message(&designer, &mut state, event, None).unwrap();
            assert_eq!(
                std::mem::discriminant(&message),
                std::mem::discriminant(&expected)
            );
        }
    }

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
    fn magic_wand_uses_source_colors_preserves_edits_and_saves_the_mask() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("regions.png");
        ::image::RgbaImage::from_fn(100, 50, |x, _| {
            ::image::Rgba([if x < 30 || x >= 70 { 180 } else { 20 }, 40, 60, 255])
        })
        .save(&source)
        .unwrap();
        let mut draft = Draft::from_image(&source).unwrap();
        draft.enable_layers().blank_mask(0, false).unwrap();
        let library = Library::open(Some(dir.path().join("library"))).unwrap();
        let (mut app, _) = App::new(library, Settings::default(), None, None);
        app.designer.draft = Some(Arc::new(draft));
        app.refresh_painter();
        assert_eq!(
            app.designer
                .paint_source_pixels
                .as_ref()
                .unwrap()
                .dimensions(),
            (100, 50)
        );
        app.busy = true;
        app.designer.compiling = true;
        let _ = app.layer_update(LayerMessage::Tool(PaintTool::MagicWand));
        let _ = app.layer_update(LayerMessage::WandTolerance(0));
        let _ = app.layer_update(LayerMessage::WandSelect([0.1, 0.5]));
        let pixel = |app: &App, x| {
            app.designer
                .paint_pixels
                .as_ref()
                .unwrap()
                .get_pixel(x, 512)
                .0[0]
        };
        assert_eq!(
            (pixel(&app, 100), pixel(&app, 512), pixel(&app, 900)),
            (255, 0, 0)
        );
        assert!(app.designer.paint_dirty && app.designer.paint_pending && app.designer.dirty);
        assert!(app.designer.paint_overlay[(512 * 1024 + 100) * 4 + 3] > 0);
        let _ = app.layer_update(LayerMessage::WandAllMatching(true));
        let _ = app.layer_update(LayerMessage::WandSelect([0.1, 0.5]));
        assert_eq!(
            (pixel(&app, 100), pixel(&app, 512), pixel(&app, 900)),
            (255, 0, 255)
        );
        let _ = app.layer_update(LayerMessage::WandAllMatching(false));
        let _ = app.layer_update(LayerMessage::Erase(true));
        let _ = app.layer_update(LayerMessage::WandSelect([0.1, 0.5]));
        assert_eq!(
            (pixel(&app, 100), pixel(&app, 512), pixel(&app, 900)),
            (0, 0, 255)
        );
        let expected = app.designer.paint_pixels.clone().unwrap();
        let _ = app.layer_update(LayerMessage::Paint([0.5; 2], [0.5; 2]));
        let _ = app.layer_update(LayerMessage::WandSelect([f32::NAN, 0.]));
        assert_eq!(app.designer.paint_pixels.as_ref(), Some(&expected));
        let asset = app.designer.paint_asset.clone().unwrap();
        app.flush_paint().unwrap();
        let saved = dir.path().join("saved");
        app.designer
            .draft
            .as_ref()
            .unwrap()
            .save_project(&saved)
            .unwrap();
        let reopened = Draft::open_project(&saved).unwrap();
        assert_eq!(
            ::image::load_from_memory(&reopened.layers.unwrap().assets[&asset].bytes)
                .unwrap()
                .into_luma8(),
            expected
        );
        let _ = app.layer_update(LayerMessage::Tool(PaintTool::HardRound));
        let _ = app.layer_update(LayerMessage::WandSelect([0.5; 2]));
        assert_eq!(app.designer.paint_pixels.as_ref(), Some(&expected));
        app.designer.draft = None;
        app.refresh_painter();
        assert!(app.designer.paint_source_pixels.is_none());
        assert_eq!(app.designer.paint_dimensions, [0, 0]);
    }

    #[test]
    fn additional_tools_preserve_live_edits_cancel_safely_and_survive_project_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("image.png");
        std::fs::write(&source, wisp_runtime::EXAMPLE_IMAGE).unwrap();
        let mut draft = Draft::from_image(&source).unwrap();
        draft.enable_layers().blank_mask(0, false).unwrap();
        let library = Library::open(Some(dir.path().join("library"))).unwrap();
        let (mut app, _) = App::new(library, Settings::default(), None, None);
        app.designer.draft = Some(Arc::new(draft));
        app.refresh_painter();
        app.busy = true;
        app.designer.compiling = true;
        let vertices = [[0.1, 0.1], [0.4, 0.1], [0.4, 0.4], [0.1, 0.4]];
        let _ = app.layer_update(LayerMessage::Tool(PaintTool::PolygonalLasso));
        for point in vertices {
            let _ = app.layer_update(LayerMessage::LassoPoint(point));
        }
        let _ = app.layer_update(LayerMessage::LassoUndo);
        assert_eq!(app.designer.lasso_vertices.len(), 3);
        assert!(!app.designer.paint_dirty);
        let _ = app.layer_update(LayerMessage::LassoCancel);
        assert!(app.designer.lasso_vertices.is_empty() && !app.designer.paint_dirty);
        let _ = app.layer_update(LayerMessage::LassoFinish);
        assert!(!app.designer.paint_dirty);
        for point in vertices {
            let _ = app.layer_update(LayerMessage::LassoPoint(point));
        }
        let _ = app.layer_update(LayerMessage::LassoFinish);
        assert!(app.designer.lasso_vertices.is_empty());
        assert!(app.designer.paint_dirty && app.designer.paint_pending && app.designer.dirty);
        assert_eq!(
            app.designer
                .paint_pixels
                .as_ref()
                .unwrap()
                .get_pixel(200, 200)
                .0[0],
            255
        );
        let _ = app.layer_update(LayerMessage::Erase(true));
        for point in [[0.15, 0.15], [0.25, 0.15], [0.2, 0.25]] {
            let _ = app.layer_update(LayerMessage::LassoPoint(point));
        }
        let _ = app.layer_update(LayerMessage::LassoFinish);
        assert_eq!(
            app.designer
                .paint_pixels
                .as_ref()
                .unwrap()
                .get_pixel(205, 184)
                .0[0],
            0
        );
        let _ = app.layer_update(LayerMessage::LassoPoint([0.8, 0.8]));
        let _ = app.layer_update(LayerMessage::Tool(PaintTool::Airbrush));
        assert!(app.designer.lasso_vertices.is_empty());
        let _ = app.layer_update(LayerMessage::Erase(false));
        let _ = app.layer_update(LayerMessage::Flow(0.03));
        let _ = app.layer_update(LayerMessage::Paint([0.75; 2], [0.75; 2]));
        let first = app
            .designer
            .paint_pixels
            .as_ref()
            .unwrap()
            .get_pixel(768, 768)
            .0[0];
        assert!(first > 0 && first <= 8);
        let _ = app.layer_update(LayerMessage::Paint([0.75; 2], [0.75; 2]));
        let second = app
            .designer
            .paint_pixels
            .as_ref()
            .unwrap()
            .get_pixel(768, 768)
            .0[0];
        assert!(second > first && second < 255);
        let _ = app.layer_update(LayerMessage::Erase(true));
        let _ = app.layer_update(LayerMessage::Paint([0.75; 2], [0.75; 2]));
        assert!(
            app.designer
                .paint_pixels
                .as_ref()
                .unwrap()
                .get_pixel(768, 768)
                .0[0]
                < second
        );
        let expected = app.designer.paint_pixels.clone().unwrap();
        let asset = app.designer.paint_asset.clone().unwrap();
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
        assert_eq!(pixels, expected);
        let _ = app.layer_update(LayerMessage::Tool(PaintTool::PolygonalLasso));
        let _ = app.layer_update(LayerMessage::LassoPoint([0.8, 0.8]));
        app.refresh_painter();
        assert!(app.designer.lasso_vertices.is_empty());
    }

    fn painter_screenshot(
        app: &App,
        renderer: &mut iced_wgpu::Renderer,
        theme: &iced::Theme,
    ) -> Vec<u8> {
        use iced_wgpu::core::{
            Layout, Renderer as _, layout, renderer as core_renderer, widget::Tree,
        };
        let size = iced::Size::new(128., 128.);
        let bounds = Rectangle::new(Point::ORIGIN, size);
        let mut painter = app.paint_canvas(Fill);
        let mut tree = Tree::new(painter.as_widget());
        let layout = painter.as_widget_mut().layout(
            &mut tree,
            renderer,
            &layout::Limits::new(iced::Size::ZERO, size),
        );
        renderer.reset(bounds);
        painter.as_widget().draw(
            &tree,
            renderer,
            theme,
            &core_renderer::Style {
                text_color: Color::WHITE,
            },
            Layout::new(&layout),
            mouse::Cursor::Unavailable,
            &bounds,
        );
        let viewport =
            iced_wgpu::graphics::Viewport::with_physical_size(iced::Size::new(128, 128), 1.);
        renderer.screenshot(&viewport, Color::BLACK)
    }

    #[test]
    #[ignore = "requires a working graphics adapter"]
    fn continuous_strokes_render_the_latest_mask_on_every_frame() {
        use canvas::Program;

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let gpu = pollster::block_on(wisp_runtime::Gpu::new(&instance, None)).unwrap();
        let engine = iced_wgpu::Engine::new(
            &gpu.adapter,
            gpu.device.clone(),
            gpu.queue.clone(),
            wgpu::TextureFormat::Rgba8UnormSrgb,
            None,
            iced_wgpu::graphics::Shell::headless(),
        );
        let mut renderer = iced_wgpu::Renderer::new(engine, iced::Font::DEFAULT, iced::Pixels(16.));
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(Some(dir.path().join("library"))).unwrap();
        let (mut app, _) = App::new(library, Settings::default(), None, None);
        app.busy = false;
        app.designer = Designer {
            brush: 0.06,
            paint_dimensions: [1024, 1024],
            paint_source: Some(image::Handle::from_rgba(1, 1, vec![60, 70, 80, 255])),
            paint_pixels: Some(::image::GrayImage::new(1024, 1024)),
            ..Default::default()
        };
        app.refresh_mask_overlay(None);
        let size = iced::Size::new(128., 128.);
        let bounds = Rectangle::new(Point::ORIGIN, size);
        let mut state = PaintState::default();

        for erase in [false, true] {
            let _ = app.layer_update(LayerMessage::Erase(erase));
            for step in 0..12 {
                let position = Point::new(16. + step as f32 * 8., 64.);
                let event = canvas::Event::Mouse(if step == 0 {
                    mouse::Event::ButtonPressed(mouse::Button::Left)
                } else {
                    mouse::Event::CursorMoved { position }
                });
                let action = MaskCanvas {
                    designer: &app.designer,
                    enabled: true,
                }
                .update(
                    &mut state,
                    &event,
                    bounds,
                    mouse::Cursor::Available(position),
                )
                .unwrap();
                let Message::Designer(DesignerMessage::Layer(message)) =
                    action.into_inner().0.unwrap()
                else {
                    panic!("expected a paint message");
                };
                let _ = app.layer_update(message);
                let pixels = painter_screenshot(&app, &mut renderer, &iced::Theme::Dark);
                for painted_step in 0..=step {
                    let offset = (64 * 128 + 16 + painted_step * 8) * 4;
                    let pixel = &pixels[offset..offset + 4];
                    if erase {
                        assert!(pixel[1].abs_diff(70) <= 1, "erase frame {step}: {pixel:?}");
                    } else {
                        assert!(pixel[1] > 100, "paint frame {step}: {pixel:?}");
                    }
                }
            }
            let _ = MaskCanvas {
                designer: &app.designer,
                enabled: true,
            }
            .update(
                &mut state,
                &canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                bounds,
                mouse::Cursor::Unavailable,
            );
            assert!(state.last.is_none());
        }
    }

    #[test]
    #[ignore = "requires a working graphics adapter"]
    fn lasso_outlines_remain_visible_above_images_in_both_themes() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let gpu = pollster::block_on(wisp_runtime::Gpu::new(&instance, None)).unwrap();
        let engine = iced_wgpu::Engine::new(
            &gpu.adapter,
            gpu.device.clone(),
            gpu.queue.clone(),
            wgpu::TextureFormat::Rgba8UnormSrgb,
            None,
            iced_wgpu::graphics::Shell::headless(),
        );
        let mut renderer = iced_wgpu::Renderer::new(engine, iced::Font::DEFAULT, iced::Pixels(16.));
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(Some(dir.path().join("library"))).unwrap();
        let (mut app, _) = App::new(library, Settings::default(), None, None);
        app.designer = Designer {
            paint_dimensions: [1024, 1024],
            paint_source: Some(image::Handle::from_rgba(1, 1, vec![60, 70, 80, 255])),
            paint_pixels: Some(::image::GrayImage::new(1024, 1024)),
            paint_tool: PaintTool::PolygonalLasso,
            lasso_vertices: vec![[0.25, 0.25], [0.75, 0.25], [0.75, 0.75]],
            ..Default::default()
        };
        app.refresh_mask_overlay(None);
        for theme in [iced::Theme::Dark, iced::Theme::Light] {
            let pixels = painter_screenshot(&app, &mut renderer, &theme);
            let outline = (48 * 128 + 96) * 4;
            assert!(
                pixels[outline + 1] > 100,
                "outline must remain above the image"
            );
            let interior = (48 * 128 + 64) * 4;
            assert_eq!(&pixels[interior..interior + 4], &[60, 70, 80, 255]);
            let corner = (32 * 128 + 32) * 4;
            assert!(
                pixels[corner + 1] > 100,
                "the first corner must remain visible"
            );
        }
    }

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
