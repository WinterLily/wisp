#[path = "layer_ui.rs"]
mod layer_ui;
use super::*;
use crate::designer::{CONTROLS, Draft, Preset};
use iced::widget::column;
use layer_ui::LayerMessage;

#[derive(Clone)]
pub(super) enum DesignerMessage {
    Layer(LayerMessage),
    Browse(bool),
    Open(PathBuf),
    Opened(Outcome<Option<(Arc<Draft>, bool)>>),
    Name(String),
    Author(String),
    Preset(Preset),
    Toggle(usize, bool),
    Value(String, f32),
    Compiled(Outcome<(Arc<Wallpaper>, String)>),
    Save(bool),
    Saved(Outcome<bool>),
    Install,
    PaintTick(Instant),
}
#[derive(Default)]
pub(super) struct Designer {
    pub draft: Option<Arc<Draft>>,
    pub preview: Option<Preview>,
    pub report: String,
    pub dirty: bool,
    pub valid: bool,
    pub layer: usize,
    pub brush: f32,
    pub erase: bool,
    pub paint_source: Option<image::Handle>,
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
}
impl Designer {
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
                        0 => d.parallax = enabled,
                        1 => d.pulse = enabled,
                        2 => d.blur = enabled,
                        3 => d.vignette = enabled,
                        _ => return Task::none(),
                    }
                    self.designer.dirty = true;
                    return self.compile_design();
                }
            }
            Value(id, value) if !self.busy => {
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
                            .map(|p| (p.id.clone(), p.default.clone()))
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
                            tokio::task::spawn_blocking(move || {
                                if project {
                                    draft.save_project(&path)
                                } else {
                                    draft.export(&path)
                                }
                            })
                            .await
                            .map_err(|e| e.to_string())?
                            .map(|_| true)
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
    pub(super) fn designer_view(&self) -> Element<'_, Message> {
        if self.designer.paint_expanded {
            return self.expanded_painter();
        }
        use DesignerMessage::*;
        let action = |m| Message::Designer(m);
        let mut controls = column![
            text("Easy Designer").size(30),
            text("Layer images, paint motion masks, and add animated effects."),
            row![
                button("Choose image")
                    .on_press_maybe((!self.busy).then_some(action(Browse(false)))),
                button("Open draft").on_press_maybe((!self.busy).then_some(action(Browse(true))))
            ]
            .spacing(10),
            text_input("Image path or saved project folder", &self.import_path)
                .on_input(Message::ImportPath),
            button("Open path").on_press_maybe(
                (!self.busy && !self.import_path.trim().is_empty())
                    .then(|| action(Open(PathBuf::from(self.import_path.trim()))))
            )
        ]
        .spacing(14);
        if let Some(draft) = &self.designer.draft {
            let d = &draft.design;
            controls = controls.push(
                text(if self.designer.dirty {
                    "Draft · unsaved changes"
                } else {
                    "Draft saved"
                })
                .size(13),
            );
            controls = controls
                .push(text_input("Wallpaper name", &d.name).on_input(move |v| action(Name(v))));
            controls =
                controls.push(text_input("Author", &d.author).on_input(move |v| action(Author(v))));
            if let Some(stack) = &draft.layers {
                controls = controls.push(self.layers_view(stack));
            } else {
                controls =
                    controls.push(button("Add layers and effects").on_press_maybe(
                        (!self.busy).then_some(action(Layer(LayerMessage::Enable))),
                    ));
            }
            controls = controls.push(text("Whole scene effects").size(20));
            controls = controls.push(text("Start from a preset").size(18)).push(
                pick_list(
                    crate::designer::Preset::ALL.as_slice(),
                    None::<crate::designer::Preset>,
                    move |p| action(DesignerMessage::Preset(p)),
                )
                .placeholder("Choose a preset"),
            );
            for (index, label, value) in [
                (0, "Mouse parallax", d.parallax),
                (1, "Breathing light", d.pulse),
                (2, "Soft blur", d.blur),
                (3, "Vignette", d.vignette),
            ] {
                controls = controls.push(
                    checkbox(value)
                        .label(label)
                        .on_toggle(move |v| Message::Designer(Toggle(index, v))),
                );
            }
            for c in CONTROLS {
                if (c.id == "parallax" && !d.parallax)
                    || (["pulse", "speed"].contains(&c.id) && !d.pulse)
                    || (c.id == "radius" && !d.blur)
                    || (c.id == "vignette" && !d.vignette)
                {
                    continue;
                }
                controls = controls
                    .push(text(format!("{}  {:.2}", c.label, d.values[c.id])))
                    .push(
                        slider(c.min..=c.max, d.values[c.id], move |v| {
                            Message::Designer(Value(c.id.into(), v))
                        })
                        .step((c.max - c.min) / 200.),
                    );
            }
            if let Err(error) = d.validate() {
                controls = controls
                    .push(text(error.to_string()).color(iced::Color::from_rgb(1., 0.4, 0.4)));
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
                    button("Save editable project…")
                        .on_press_maybe(ready.then_some(action(Save(true)))),
                )
                .push(button("Save to library").on_press_maybe(ready.then_some(action(Install))))
                .push(button("Export .wisp…").on_press_maybe(ready.then_some(action(Save(false)))));
        }
        let mut preview = column![
            row![
                text("Live preview").size(24),
                space().width(Fill),
                button(if self.preview_paused { "Play" } else { "Pause" })
                    .on_press(Message::PausePreview)
            ]
            .spacing(10)
        ]
        .spacing(16);
        if let Some(p) = &self.designer.preview {
            preview = preview
                .push(iced::widget::Shader::new(p.clone()).width(Fill).height(380))
                .push(text(&self.designer.report).size(13));
            if let Some(d) = &self.designer.draft {
                preview = preview.push(text(format!("Source: {} × {} · Controls become wallpaper defaults when saved or exported.", d.dimensions.0, d.dimensions.1)).size(13));
            }
        } else {
            preview = preview.push(text("Choose a PNG, JPEG or WebP image to begin."));
        }
        row![
            scrollable(controls).width(FillPortion(2)),
            scrollable(preview).width(FillPortion(3))
        ]
        .spacing(24)
        .height(Fill)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
