#[path = "layer_ui.rs"]
pub(super) mod layer_ui;
use super::*;
use crate::designer::{CONTROLS, Draft, Preset};
use iced::widget::column;
use layer_ui::LayerMessage;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DesignerPanel {
    Layers,
    Project,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DesignEffect {
    Parallax,
    Pulse,
    Blur,
    Vignette,
}

impl DesignEffect {
    pub(super) const ALL: [Self; 4] = [Self::Vignette, Self::Blur, Self::Pulse, Self::Parallax];

    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Parallax => "parallax",
            Self::Pulse => "pulse",
            Self::Blur => "blur",
            Self::Vignette => "vignette",
        }
    }

    fn enabled(self, design: &crate::designer::Design) -> bool {
        match self {
            Self::Parallax => design.parallax,
            Self::Pulse => design.pulse,
            Self::Blur => design.blur,
            Self::Vignette => design.vignette,
        }
    }

    fn controls(self) -> &'static [&'static str] {
        match self {
            Self::Parallax => &["parallax"],
            Self::Pulse => &["pulse", "speed"],
            Self::Blur => &["radius"],
            Self::Vignette => &["vignette"],
        }
    }
}

impl std::fmt::Display for DesignEffect {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Parallax => "Mouse parallax",
            Self::Pulse => "Breathing light",
            Self::Blur => "Soft blur",
            Self::Vignette => "Vignette",
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum DesignerSelection {
    #[default]
    Image,
    Effect(DesignEffect),
    Color,
}

#[derive(Clone)]
pub(super) enum DesignerMessage {
    Panel(DesignerPanel),
    Select(DesignerSelection),
    Layer(LayerMessage),
    Browse(bool),
    Open(PathBuf),
    Opened(Outcome<Option<(Arc<Draft>, bool)>>),
    Name(String),
    Author(String),
    Preset(Preset),
    Toggle(DesignEffect, bool),
    Value(String, f32),
    Compiled(Outcome<(Arc<Wallpaper>, String)>),
    Save(bool),
    SaveTo(bool, PathBuf),
    Saved(Outcome<bool>),
    Install,
    PaintTick(Instant),
}
#[derive(Default)]
pub(super) struct Designer {
    pub panel: Option<DesignerPanel>,
    pub selection: DesignerSelection,
    pub layer_tab: layer_ui::LayerTab,
    pub draft: Option<Arc<Draft>>,
    pub preview: Option<Preview>,
    pub report: String,
    pub dirty: bool,
    pub valid: bool,
    pub layer: usize,
    pub brush: f32,
    pub erase: bool,
    pub paint_tool: crate::painting::PaintTool,
    pub airbrush_flow: f32,
    pub magic_wand: crate::painting::MagicWandOptions,
    pub lasso_vertices: Vec<[f32; 2]>,
    pub paint_session: u64,
    pub paint_source: Option<image::Handle>,
    pub paint_source_pixels: Option<::image::RgbaImage>,
    pub paint_mask: Option<image::Handle>,
    pub paint_overlay: Vec<u8>,
    pub paint_pixels: Option<::image::GrayImage>,
    pub paint_dimensions: [u32; 2],
    pub paint_asset: Option<String>,
    pub paint_dirty: bool,
    pub paint_pending: bool,
    pub paint_last_compile: Option<Instant>,
    pub paint_expanded: bool,
    pub compiling: bool,
    pub compile_pending: bool,
}
impl Designer {
    fn airbrush_flow(&self) -> f32 {
        if self.airbrush_flow == 0. {
            0.15
        } else {
            self.airbrush_flow
        }
    }
    fn paint_due(&self, now: Instant) -> bool {
        self.paint_pending
            && self
                .paint_last_compile
                .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(2))
    }
}
impl App {
    pub(super) fn designer_update(&mut self, message: DesignerMessage) -> Task<Message> {
        use DesignerMessage::*;
        match message {
            Panel(panel) => {
                self.designer.panel = (self.designer.panel != Some(panel)).then_some(panel);
            }
            Select(selection) if !self.busy => {
                if let Err(error) = self.flush_paint() {
                    self.status = format!("Could not preserve mask: {error:#}");
                    return Task::none();
                }
                self.designer.selection = selection;
                self.designer.panel = Some(DesignerPanel::Layers);
                self.designer.lasso_vertices.clear();
                self.designer.paint_session += 1;
            }
            Layer(m) => return self.layer_update(m),
            PaintTick(now) if self.designer.paint_due(now) && !self.busy => {
                return self.compile_design();
            }
            Browse(project) if !self.busy => {
                self.busy = true;
                return Task::perform(
                    async move {
                        let dialog = rfd::AsyncFileDialog::new().set_title(if project {
                            "Open Easy Designer project"
                        } else {
                            "Choose wallpaper image"
                        });
                        let file = if project {
                            dialog.pick_folder().await
                        } else {
                            dialog
                                .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                                .pick_file()
                                .await
                        };
                        let Some(file) = file else {
                            return Ok(None);
                        };
                        let path = file.path().to_owned();
                        tokio::task::spawn_blocking(move || {
                            if project {
                                Draft::open_project(&path)
                            } else {
                                Draft::from_image(&path)
                            }
                        })
                        .await
                        .map_err(|e| e.to_string())?
                        .map(|d| Some((Arc::new(d), project)))
                        .map_err(|e| format!("{e:#}"))
                    },
                    |r| Message::Designer(Opened(r)),
                );
            }
            Open(path) if !self.busy => {
                self.busy = true;
                return work(
                    move || {
                        let saved = path.is_dir();
                        let draft = if saved {
                            Draft::open_project(&path)?
                        } else {
                            Draft::from_image(&path)?
                        };
                        Ok(Some((Arc::new(draft), saved)))
                    },
                    |r| Message::Designer(Opened(r)),
                );
            }
            Opened(result) => {
                self.busy = false;
                match result {
                    Ok(Some((draft, saved))) => {
                        self.designer.layer = 0;
                        self.designer.selection = DesignerSelection::Image;
                        self.designer.panel = Some(DesignerPanel::Layers);
                        self.designer.brush = 0.06;
                        self.designer.draft = Some(draft);
                        self.refresh_painter();
                        self.designer.dirty = !saved;
                        self.page = Page::Designer;
                        return self.compile_design();
                    }
                    Ok(None) => {}
                    Err(e) => self.status = format!("Could not open design: {e}"),
                }
            }
            Name(value) if !self.busy => {
                if let Some(d) = &mut self.designer.draft {
                    Arc::make_mut(d).design.name = value;
                    self.designer.dirty = true;
                }
            }
            Author(value) if !self.busy => {
                if let Some(d) = &mut self.designer.draft {
                    Arc::make_mut(d).design.author = value;
                    self.designer.dirty = true;
                }
            }
            Preset(preset) if !self.busy => {
                if let Some(d) = &mut self.designer.draft {
                    Arc::make_mut(d).design.preset(preset);
                    self.designer.dirty = true;
                    return self.compile_design();
                }
            }
            Toggle(index, enabled) if !self.busy => {
                if let Some(d) = &mut self.designer.draft {
                    let d = &mut Arc::make_mut(d).design;
                    match index {
                        DesignEffect::Parallax => d.parallax = enabled,
                        DesignEffect::Pulse => d.pulse = enabled,
                        DesignEffect::Blur => d.blur = enabled,
                        DesignEffect::Vignette => d.vignette = enabled,
                    }
                    self.designer.dirty = true;
                    return self.compile_design();
                }
            }
            Value(id, value) if !self.busy || self.designer.compiling => {
                if let Some(control) = CONTROLS.iter().find(|c| c.id == id)
                    && value.is_finite()
                    && (control.min..=control.max).contains(&value)
                    && let Some(d) = &mut self.designer.draft
                {
                    Arc::make_mut(d).design.values.insert(id.clone(), value);
                    self.designer.dirty = true;
                    if let Some(preview) = &mut self.designer.preview
                        && preview.parameters.contains_key(&id)
                    {
                        Arc::make_mut(&mut preview.parameters)
                            .insert(id, ParameterValue::Float(value));
                    }
                }
            }
            Compiled(result) => {
                self.busy = false;
                self.designer.compiling = false;
                match result {
                    Ok((wallpaper, report)) => {
                        self.generation += 1;
                        let parameters = wallpaper
                            .parameters()
                            .iter()
                            .map(|p| {
                                let value = self
                                    .designer
                                    .draft
                                    .as_ref()
                                    .and_then(|draft| draft.design.values.get(&p.id))
                                    .map(|value| ParameterValue::Float(*value))
                                    .unwrap_or_else(|| p.default.clone());
                                (p.id.clone(), value)
                            })
                            .collect();
                        self.designer.preview = Some(Preview {
                            generation: self.generation,
                            wallpaper,
                            parameters: Arc::new(parameters),
                            globals: Globals::new(1, 1),
                            status: Arc::new(Mutex::new(Status::default())),
                        });
                        self.designer.report = report;
                        self.designer.valid = true;
                        self.status = if self.designer.paint_expanded {
                            "Preview updated. Painting remains live."
                        } else {
                            "Preview ready. Move the pointer over the image to try mouse movement."
                        }
                        .into();
                    }
                    Err(e) => {
                        self.designer.valid = false;
                        self.status =
                            format!("Design could not compile; previous preview retained: {e}");
                    }
                }
                if std::mem::take(&mut self.designer.compile_pending) {
                    return self.compile_design();
                }
            }
            SaveTo(project, path) if !self.busy => {
                if let Err(error) = self.flush_paint() {
                    self.status = format!("Could not save mask: {error:#}");
                    return Task::none();
                }
                if let Some(draft) = self.designer.draft.clone() {
                    self.busy = true;
                    return work(
                        move || save_to(&draft, &path, project),
                        move |result| {
                            if project {
                                Message::Designer(Saved(result))
                            } else {
                                Message::Exported(result)
                            }
                        },
                    );
                }
            }
            Save(project) if !self.busy => {
                if let Err(e) = self.flush_paint() {
                    self.status = format!("Could not save mask: {e:#}");
                    return Task::none();
                }
                if let Some(draft) = self.designer.draft.clone() {
                    self.busy = true;
                    return Task::perform(
                        async move {
                            let dialog = rfd::AsyncFileDialog::new().set_title(if project {
                                "Save a new project folder"
                            } else {
                                "Export wallpaper"
                            });
                            let chosen = if project {
                                dialog
                                    .set_file_name("my-wallpaper-project")
                                    .save_file()
                                    .await
                            } else {
                                dialog
                                    .add_filter("Wisp wallpaper", &["wisp"])
                                    .set_file_name("wallpaper.wisp")
                                    .save_file()
                                    .await
                            };
                            let Some(file) = chosen else {
                                return Ok(false);
                            };
                            let path = file.path().to_owned();
                            tokio::task::spawn_blocking(move || save_to(&draft, &path, project))
                                .await
                                .map_err(|e| e.to_string())?
                                .map_err(|e| format!("{e:#}"))
                        },
                        move |r| {
                            if project {
                                Message::Designer(Saved(r))
                            } else {
                                Message::Exported(r)
                            }
                        },
                    );
                }
            }
            Saved(result) => {
                self.busy = false;
                match result {
                    Ok(true) => {
                        self.designer.dirty = false;
                        self.status = "Editable project saved, including its source image.".into();
                    }
                    Ok(false) => self.status = "Save cancelled".into(),
                    Err(e) => self.status = format!("Could not save project: {e}"),
                }
            }
            Install if !self.busy => {
                if let Err(e) = self.flush_paint() {
                    self.status = format!("Could not save mask: {e:#}");
                    return Task::none();
                }
                if let Some(draft) = self.designer.draft.clone() {
                    self.busy = true;
                    let library = self.library.clone();
                    return work(
                        move || {
                            library
                                .install(&draft.compile()?.package, "Easy Designer")
                                .map(|i| Some(Box::new(i)))
                        },
                        Message::Imported,
                    );
                }
            }
            _ => {}
        }
        Task::none()
    }
    pub(super) fn compile_design(&mut self) -> Task<Message> {
        let Some(draft) = self.designer.draft.clone() else {
            return Task::none();
        };
        // A released slider must eventually render its latest edit without overlapping compiles.
        if self.designer.compiling {
            self.designer.compile_pending = true;
            return Task::none();
        }
        self.designer.paint_pending = false;
        self.designer.paint_last_compile = Some(Instant::now());
        self.designer.compiling = true;
        let paint = if self.designer.paint_dirty {
            self.designer
                .paint_asset
                .clone()
                .zip(self.designer.paint_pixels.clone())
        } else {
            None
        };
        self.busy = true;
        self.designer.valid = false;
        work(
            move || {
                let mut preview_draft = (*draft).clone();
                if let Some((id, pixels)) = paint {
                    let asset = crate::layers::encode(::image::DynamicImage::ImageLuma8(pixels))?;
                    if let Some(stack) = &mut preview_draft.layers {
                        Arc::make_mut(&mut stack.assets).insert(id, asset);
                    }
                }
                preview_draft.design.name = "Preview".into();
                preview_draft.design.author = "Preview".into();
                let compiled = preview_draft.compile()?;
                let r = compiled.report;
                let mut report = format!(
                    "{} passes · {} images ({:.1} MiB) · {:.1} MiB intermediate textures · {:.1} MiB persistent storage",
                    r.passes,
                    r.image_assets,
                    r.image_bytes as f64 / 1048576.,
                    r.intermediate_bytes as f64 / 1048576.,
                    r.storage_bytes as f64 / 1048576.
                );
                if r.resized_images > 0 {
                    report.push_str(&format!(
                        " · {} image(s) resized for playback; full-resolution originals preserved.",
                        r.resized_images
                    ));
                }
                if r.passes > 4
                    || r.image_bytes + r.intermediate_bytes + r.storage_bytes > 64 * 1024 * 1024
                {
                    report
                        .push_str(" · High cost: consider fewer layers or smaller source images.");
                }
                Ok((
                    Arc::new(Wallpaper::from_package(&compiled.package)?),
                    report,
                ))
            },
            |r| Message::Designer(DesignerMessage::Compiled(r)),
        )
    }
    pub(in crate::ui) fn designer_layers_view<'a>(
        &'a self,
        draft: &'a Draft,
    ) -> Element<'a, Message> {
        let action = Message::Designer;
        let enabled = !self.busy;
        let selection = self.designer.selection;
        let background = crate::layers::Layer::new("Background".into(), "background".into());
        let image_layers = draft
            .layers
            .as_ref()
            .map_or(std::slice::from_ref(&background), |stack| {
                stack.layers.as_slice()
            });
        let mut layers = column![
            row![
                text("Front to back").size(11),
                space().width(Fill),
                icon_button(
                    Icon::Create,
                    "Add image layer",
                    (enabled && image_layers.len() < 6).then_some(action(DesignerMessage::Layer(
                        LayerMessage::Import(crate::layers::ImportTarget::Layer)
                    ))),
                    false
                ),
            ]
            .align_y(iced::Center)
        ]
        .spacing(6);
        for effect in DesignEffect::ALL {
            layers = layers.push(
                row![
                    checkbox(effect.enabled(&draft.design))
                        .on_toggle_maybe(enabled.then_some(move |value| action(
                            DesignerMessage::Toggle(effect, value)
                        ))),
                    button(
                        row![
                            icons::glyph(Icon::Effects, 14),
                            text(effect.to_string()).size(14)
                        ]
                        .spacing(8)
                        .align_y(iced::Center)
                    )
                    .width(Fill)
                    .style(if selection == DesignerSelection::Effect(effect) {
                        style::selected
                    } else {
                        style::quiet
                    })
                    .on_press_maybe(enabled.then_some(action(DesignerMessage::Select(
                        DesignerSelection::Effect(effect)
                    )))),
                ]
                .spacing(8)
                .align_y(iced::Center),
            );
        }
        layers = layers.push(
            button(
                row![
                    icons::glyph(Icon::Adjust, 14),
                    text("Color adjustments").size(14)
                ]
                .spacing(8)
                .align_y(iced::Center),
            )
            .width(Fill)
            .style(if selection == DesignerSelection::Color {
                style::selected
            } else {
                style::quiet
            })
            .on_press_maybe(
                enabled.then_some(action(DesignerMessage::Select(DesignerSelection::Color))),
            ),
        );
        layers = layers.push(self.layer_rows_view(image_layers));
        let mut panel = column![].spacing(14);
        match selection {
            DesignerSelection::Image => {
                let index = self.designer.layer.min(image_layers.len() - 1);
                panel = panel.push(self.layer_settings_view(
                    &image_layers[index],
                    index,
                    image_layers.len(),
                ));
            }
            DesignerSelection::Effect(effect) => {
                panel = panel
                    .push(text(effect.to_string()).size(16))
                    .push(text("Applies to the whole scene").size(12));
                if !effect.enabled(&draft.design) {
                    panel = panel
                        .push(text("Hidden · enable this layer to preview the effect.").size(12));
                }
                panel = panel.push(
                    container(self.scene_controls_view(&draft.design, effect.controls()))
                        .padding(12)
                        .width(Fill)
                        .style(style::panel),
                );
            }
            DesignerSelection::Color => {
                panel = panel
                    .push(text("Color adjustments").size(16))
                    .push(text("Applies to the whole scene").size(12))
                    .push(
                        container(
                            self.scene_controls_view(&draft.design, &["brightness", "contrast"]),
                        )
                        .padding(12)
                        .width(Fill)
                        .style(style::panel),
                    );
            }
        }
        panel = panel.push(iced::widget::rule::horizontal(1)).push(
            pick_list(
                crate::designer::Preset::ALL.as_slice(),
                None::<crate::designer::Preset>,
                move |preset| action(DesignerMessage::Preset(preset)),
            )
            .placeholder("Scene preset…")
            .width(Fill),
        );
        column![
            scrollable(layers).height(300),
            iced::widget::rule::horizontal(1),
            scrollable(panel).height(Fill),
        ]
        .spacing(14)
        .height(Fill)
        .into()
    }

    fn scene_controls_view<'a>(
        &self,
        design: &'a crate::designer::Design,
        controls: &[&str],
    ) -> Element<'a, Message> {
        let mut panel = column![].spacing(14);
        for control in CONTROLS
            .iter()
            .filter(|control| controls.contains(&control.id))
        {
            panel = panel.push(
                column![
                    row![
                        text(control.label).size(12),
                        space().width(Fill),
                        text(format!("{:.2}", design.values[control.id])).size(12)
                    ],
                    slider(
                        control.min..=control.max,
                        design.values[control.id],
                        move |value| Message::Designer(DesignerMessage::Value(
                            control.id.into(),
                            value
                        ))
                    )
                    .step((control.max - control.min) / 200.),
                ]
                .spacing(8),
            );
        }
        panel.into()
    }

    pub(super) fn designer_view(&self) -> Element<'_, Message> {
        use DesignerMessage::*;
        use DesignerPanel as PanelKind;
        let action = |m| Message::Designer(m);
        if self.designer.paint_expanded {
            return self.expanded_painter();
        }
        let Some(draft) = &self.designer.draft else {
            let welcome = column![
                text("Start with an image.").size(32),
                text("Give it a little motion. Make it your own.")
                    .size(15)
                    .style(|t| text::Style {
                        color: Some(style::muted(t))
                    }),
                space().height(12),
                row![
                    button("Choose image…")
                        .style(iced_button::primary)
                        .on_press_maybe((!self.busy).then_some(action(Browse(false)))),
                    button("Open project…")
                        .on_press_maybe((!self.busy).then_some(action(Browse(true))))
                ]
                .spacing(12),
                text_input("Image or project folder", &self.import_path)
                    .on_input(Message::ImportPath)
                    .width(420),
                button("Open path").on_press_maybe(
                    (!self.busy && !self.import_path.trim().is_empty())
                        .then(|| action(Open(PathBuf::from(self.import_path.trim())))),
                ),
                text("PNG, JPEG or WebP").size(12).style(|t| text::Style {
                    color: Some(style::muted(t))
                }),
            ]
            .spacing(14)
            .align_x(iced::Center);
            return container(welcome).center(Fill).style(style::stage).into();
        };
        let d = &draft.design;
        let stage: Element<'_, Message> = if let Some(p) = &self.designer.preview {
            crate::preview::fitted(p.clone(), draft.dimensions)
        } else {
            container(text("Preparing canvas…").size(14))
                .center(Fill)
                .into()
        };
        let stage = container(stage)
            .padding(24)
            .width(Fill)
            .height(Fill)
            .style(style::stage);
        if self.focus_mode {
            return iced::widget::stack![
                stage,
                container(icon_button(
                    Icon::Unfocus,
                    "Show controls · Esc",
                    Some(Message::Dismiss),
                    false
                ))
                .padding(12)
                .align_right(Fill),
            ]
            .into();
        }
        let tool = |icon, label, panel| {
            icon_button(
                icon,
                label,
                Some(action(Panel(panel))),
                self.designer.panel == Some(panel),
            )
        };
        let toolbar = row![
            text(if d.name.is_empty() {
                "Untitled"
            } else {
                &d.name
            })
            .size(15),
            text(if self.designer.dirty { "· edited" } else { "" })
                .size(11)
                .style(|t| text::Style {
                    color: Some(style::muted(t))
                }),
            space().width(Fill),
            tool(Icon::Layers, "Layers", PanelKind::Layers),
            tool(Icon::Project, "Project", PanelKind::Project),
            self.playback_button(),
            icon_button(Icon::Focus, "Focus · F11", Some(Message::Focus), false),
        ]
        .align_y(iced::Center)
        .spacing(4)
        .padding([4, 8]);
        let mut workspace = row![stage].spacing(8).height(Fill);
        if let Some(panel) = self.designer.panel {
            let title = match panel {
                PanelKind::Layers => "Layers",
                PanelKind::Project => "Project",
            };
            let mut controls = column![].spacing(14);
            match panel {
                PanelKind::Layers => {}
                PanelKind::Project => {
                    controls = controls
                        .push(text("Name").size(12))
                        .push(
                            text_input("Wallpaper name", &d.name)
                                .on_input(move |v| action(Name(v))),
                        )
                        .push(text("Author").size(12))
                        .push(text_input("Author", &d.author).on_input(move |v| action(Author(v))));
                    if let Err(error) = d.validate() {
                        controls = controls.push(
                            text(error.to_string())
                                .size(12)
                                .color(iced::Color::from_rgb(0.9, 0.45, 0.45)),
                        );
                    }
                    let ready = !self.busy
                        && self.designer.valid
                        && d.validate().is_ok()
                        && self
                            .designer
                            .preview
                            .as_ref()
                            .is_some_and(|p| p.status.lock().unwrap().error.is_none());
                    controls = controls
                        .push(
                            button("Save project…")
                                .style(iced_button::primary)
                                .width(Fill)
                                .on_press_maybe(ready.then_some(action(Save(true)))),
                        )
                        .push(
                            button("Save to library")
                                .width(Fill)
                                .on_press_maybe(ready.then_some(action(Install))),
                        )
                        .push(
                            button("Export wallpaper…")
                                .width(Fill)
                                .on_press_maybe(ready.then_some(action(Save(false)))),
                        )
                        .push(iced::widget::rule::horizontal(1))
                        .push(
                            row![
                                button("New image…")
                                    .on_press_maybe((!self.busy).then_some(action(Browse(false)))),
                                button("Open…")
                                    .on_press_maybe((!self.busy).then_some(action(Browse(true)))),
                            ]
                            .spacing(4),
                        )
                        .push(
                            text_input("Image or project path", &self.import_path)
                                .on_input(Message::ImportPath)
                                .on_submit(action(Open(PathBuf::from(self.import_path.trim())))),
                        )
                        .push(
                            button("Open path").on_press_maybe(
                                (!self.busy && !self.import_path.trim().is_empty())
                                    .then(|| action(Open(PathBuf::from(self.import_path.trim())))),
                            ),
                        )
                        .push(
                            text(format!(
                                "Source · {} × {}",
                                draft.dimensions.0, draft.dimensions.1
                            ))
                            .size(11),
                        )
                        .push(text(&self.designer.report).size(11).style(|t| text::Style {
                            color: Some(style::muted(t)),
                        }));
                }
            }
            let controls: Element<'_, Message> = match panel {
                PanelKind::Layers => self.designer_layers_view(draft),
                PanelKind::Project => scrollable(controls).height(Fill).into(),
            };
            let inspector = column![
                row![
                    text(title).size(18),
                    space().width(Fill),
                    icon_button(
                        Icon::Close,
                        "Close panel",
                        Some(action(Panel(panel))),
                        false
                    )
                ]
                .align_y(iced::Center),
                controls,
            ]
            .spacing(16);
            workspace = workspace.push(motion::entrance(
                container(inspector)
                    .padding(16)
                    .width(316)
                    .height(Fill)
                    .style(style::panel),
                panel as u64,
                iced::Vector::new(24., 0.),
            ));
        }
        column![toolbar, workspace].spacing(6).height(Fill).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layered_app() -> (tempfile::TempDir, App) {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("image.png");
        std::fs::write(&source, wisp_runtime::EXAMPLE_IMAGE).unwrap();
        let mut draft = Draft::from_image(&source).unwrap();
        draft.enable_layers().blank_mask(0, false).unwrap();
        let (mut app, _) = App::new(
            Library::open(Some(temporary.path().join("library"))).unwrap(),
            Settings::default(),
            None,
            None,
        );
        app.busy = false;
        app.designer.draft = Some(Arc::new(draft));
        app.designer.brush = 0.06;
        app.refresh_painter();
        (temporary, app)
    }

    #[test]
    fn scene_layers_expose_every_scene_control_once() {
        let mut controls = vec!["brightness", "contrast"];
        for effect in DesignEffect::ALL {
            controls.extend_from_slice(effect.controls());
        }
        controls.sort_unstable();
        let mut expected: Vec<_> = CONTROLS.iter().map(|control| control.id).collect();
        expected.sort_unstable();
        assert_eq!(controls, expected);
    }

    #[test]
    fn selecting_scene_layers_preserves_painted_masks_and_returns_to_the_image() {
        let (_temporary, mut app) = layered_app();
        let _ = app.layer_update(LayerMessage::Paint([0.5, 0.5], [0.5, 0.5]));
        assert!(app.designer.paint_dirty);
        let _ = app.designer_update(DesignerMessage::Select(DesignerSelection::Effect(
            DesignEffect::Blur,
        )));
        assert_eq!(
            app.designer.selection,
            DesignerSelection::Effect(DesignEffect::Blur)
        );
        assert!(!app.designer.paint_dirty);
        let stack = app
            .designer
            .draft
            .as_ref()
            .unwrap()
            .layers
            .as_ref()
            .unwrap();
        let mask = stack.layers[0].mask.as_ref().unwrap();
        let pixels = wisp_compiler::decode_source_image(&stack.assets[&mask.asset].bytes)
            .unwrap()
            .into_luma8();
        assert_eq!(
            pixels.get_pixel(pixels.width() / 2, pixels.height() / 2).0[0],
            255
        );

        let _ = app.designer_update(DesignerMessage::Value("radius".into(), 3.));
        let _ = app.layer_update(LayerMessage::Select(0));
        assert_eq!(app.designer.selection, DesignerSelection::Image);
        assert_eq!(app.designer.layer, 0);
        assert_eq!(
            app.designer.draft.as_ref().unwrap().design.values["radius"],
            3.
        );
        assert_eq!(
            app.designer
                .paint_pixels
                .as_ref()
                .unwrap()
                .get_pixel(512, 512)
                .0[0],
            255
        );
    }

    #[test]
    fn failed_mask_flush_and_busy_edits_keep_the_current_selection() {
        let (_temporary, mut app) = layered_app();
        app.busy = true;
        let _ = app.designer_update(DesignerMessage::Select(DesignerSelection::Color));
        assert_eq!(app.designer.selection, DesignerSelection::Image);
        app.busy = false;
        app.designer.paint_dirty = true;
        app.designer.paint_asset = None;
        let _ = app.designer_update(DesignerMessage::Select(DesignerSelection::Color));
        assert_eq!(app.designer.selection, DesignerSelection::Image);
        assert!(app.designer.paint_dirty);
        assert!(app.status.contains("Could not preserve mask"));
    }

    #[test]
    fn importing_an_image_selects_it_even_when_a_scene_effect_was_selected() {
        let (temporary, mut app) = layered_app();
        app.designer.selection = DesignerSelection::Effect(DesignEffect::Vignette);
        let mut imported = app.designer.draft.as_ref().unwrap().as_ref().clone();
        imported
            .enable_layers()
            .import(
                &temporary.path().join("image.png"),
                crate::layers::ImportTarget::Layer,
            )
            .unwrap();
        let _ = app.layer_update(LayerMessage::Imported(Ok(Some(Arc::new(imported)))));
        assert_eq!(app.designer.selection, DesignerSelection::Image);
        assert_eq!(app.designer.layer, 1);
    }

    #[test]
    fn failed_replacements_keep_preview_and_live_edits_update_export_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let image = temp.path().join("image.png");
        std::fs::write(&image, wisp_runtime::EXAMPLE_IMAGE).unwrap();
        let draft = Arc::new(Draft::from_image(&image).unwrap());
        let wallpaper =
            Arc::new(Wallpaper::from_package(&draft.compile().unwrap().package).unwrap());
        let library = Library::open(Some(temp.path().join("library"))).unwrap();
        let (mut app, _) = App::new(library, Settings::default(), None, None);
        app.busy = false;
        let _ = app.designer_update(DesignerMessage::Opened(Ok(Some((draft, true)))));
        assert!(!app.designer.dirty);
        let _ = app.designer_update(DesignerMessage::Compiled(Ok((
            wallpaper,
            "one pass".into(),
        ))));
        let generation = app.designer.preview.as_ref().unwrap().generation;
        let _ = app.designer_update(DesignerMessage::Value("brightness".into(), 0.7));
        assert_eq!(
            app.designer.draft.as_ref().unwrap().design.values["brightness"],
            0.7
        );
        assert_eq!(
            app.designer.preview.as_ref().unwrap().parameters["brightness"],
            ParameterValue::Float(0.7)
        );
        let _ = app.designer_update(DesignerMessage::Value("brightness".into(), f32::NAN));
        assert_eq!(
            app.designer.draft.as_ref().unwrap().design.values["brightness"],
            0.7
        );
        let _ = app.designer_update(DesignerMessage::Opened(Err("broken image".into())));
        assert!(app.designer.valid);
        let _ = app.designer_update(DesignerMessage::Compiled(Err("shader failure".into())));
        assert!(!app.designer.valid);
        assert_eq!(
            app.designer.preview.as_ref().unwrap().generation,
            generation
        );
        assert!(app.designer.dirty);
        let _ = app.designer_update(DesignerMessage::Saved(Ok(false)));
        assert!(app.designer.dirty);
        let _ = app.designer_update(DesignerMessage::Saved(Err("disk full".into())));
        assert!(app.designer.dirty);
        let _ = app.designer_update(DesignerMessage::Saved(Ok(true)));
        assert!(!app.designer.dirty);
    }
}

fn save_to(draft: &Draft, path: &std::path::Path, project: bool) -> Result<bool> {
    if project {
        draft.save_project(path)?;
    } else {
        draft.export(path)?;
    }
    Ok(true)
}

#[cfg(test)]
mod path_tests {
    use super::*;
    #[test]
    fn path_saves_keep_ui_edits_and_never_replace_existing_projects() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("image.png");
        std::fs::write(&source, wisp_runtime::EXAMPLE_IMAGE).unwrap();
        let draft = Draft::from_image(&source).unwrap();
        let (mut app, _) = App::new(
            Library::open(Some(dir.path().join("library"))).unwrap(),
            Settings::default(),
            None,
            None,
        );
        app.busy = false;
        app.designer.draft = Some(Arc::new(draft));
        let _ = app.designer_update(DesignerMessage::Name("Audited edit".into()));
        let _ = app.designer_update(DesignerMessage::Value("brightness".into(), 0.6));
        let project = dir.path().join("project");
        let exported = dir.path().join("export.wisp");
        let draft = app.designer.draft.as_ref().unwrap();
        assert!(save_to(draft, &project, true).unwrap());
        assert!(save_to(draft, &exported, false).unwrap());
        let reopened = Draft::open_project(&project).unwrap();
        assert_eq!(reopened.design.name, "Audited edit");
        assert_eq!(reopened.design.values["brightness"], 0.6);
        assert!(save_to(draft, &project, true).is_err());
        assert!(save_to(draft, &exported, false).is_err());
        let _ = app.designer_update(DesignerMessage::Saved(Err("destination exists".into())));
        assert!(app.designer.dirty);
        let _ = app.designer_update(DesignerMessage::Saved(Ok(true)));
        assert!(!app.designer.dirty);
    }
}
