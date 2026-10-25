use super::*;
use crate::graph::{CATALOG, Document, ports};
use iced::{
    Point, Vector,
    widget::{canvas, column},
};
use wisp_ir::Op;

#[derive(Clone)]
pub(super) enum GraphMessage {
    New,
    FromEasy,
    Browse,
    Open(PathBuf),
    Opened(Outcome<Option<Arc<Document>>>),
    Search(String),
    Add(String),
    Select(String),
    Move(String, [f32; 2]),
    Pan(Vector),
    Zoom(f32, Point),
    Arrange,
    Fit,
    CanvasSize(iced::Size),
    Wire(String),
    Socket(String, String),
    Disconnect(String, String),
    CancelWire,
    Delete,
    Output,
    FieldEdit(String, String),
    Field(String, String),
    ValueType(String),
    Metadata(bool, String),
    ParameterBound(bool, String),
    Asset,
    AssetChosen(Outcome<Option<Arc<Document>>>),
    Compiled(u64, Outcome<(Arc<Wallpaper>, String)>),
    Save(bool),
    Saved(Outcome<bool>),
    Install,
}
#[derive(Default)]
pub(super) struct GraphEditor {
    pub document: Option<Arc<Document>>,
    pub preview: Option<Preview>,
    pub selected: Option<String>,
    pub wire: Option<String>,
    pub pan: Vector,
    pub zoom: f32,
    pub canvas_size: iced::Size,
    pub search: String,
    pub fields: BTreeMap<String, String>,
    pub diagnostic: String,
    pub report: String,
    pub revision: u64,
    pub valid: bool,
    pub dirty: bool,
}
impl App {
    fn graph_fit(&mut self) {
        let Some(doc) = &self.graph.document else {
            return;
        };
        if doc.layout.positions.is_empty() {
            return;
        }
        let min_x = doc
            .layout
            .positions
            .values()
            .map(|p| p[0])
            .fold(f32::INFINITY, f32::min);
        let min_y = doc
            .layout
            .positions
            .values()
            .map(|p| p[1])
            .fold(f32::INFINITY, f32::min);
        let max_x = doc
            .layout
            .positions
            .values()
            .map(|p| p[0] + 206.)
            .fold(f32::NEG_INFINITY, f32::max);
        let max_y = doc
            .layout
            .positions
            .values()
            .map(|p| p[1] + 140.)
            .fold(f32::NEG_INFINITY, f32::max);
        let size = self.graph.canvas_size;
        let width = if size.width > 0. { size.width } else { 650. };
        let height = if size.height > 0. { size.height } else { 400. };
        self.graph.zoom = ((width - 40.) / (max_x - min_x))
            .min((height - 40.) / (max_y - min_y))
            .clamp(0.1, 1.);
        self.graph.pan = Vector::new(20. - min_x * self.graph.zoom, 20. - min_y * self.graph.zoom);
    }
    fn graph_compile(&mut self) -> Task<Message> {
        let Some(doc) = self.graph.document.clone() else {
            return Task::none();
        };
        self.graph.revision += 1;
        let revision = self.graph.revision;
        self.graph.valid = false;
        self.busy = true;
        work(
            move || {
                let compiled = doc.compile()?;
                let r = compiled.report;
                Ok((
                    Arc::new(Wallpaper::from_package(&compiled.package)?),
                    format!(
                        "{} passes · {} live nodes · {} pruned · {:.1} MiB images · {:.1} MiB targets · {:.1} MiB persistent state{}",
                        r.passes,
                        r.live_nodes,
                        r.eliminated_nodes,
                        r.image_bytes as f64 / 1048576.,
                        r.intermediate_bytes as f64 / 1048576.,
                        r.storage_bytes as f64 / 1048576.,
                        if r.resized_images > 0 {
                            format!(
                                " · {} image(s) resized for playback; full-resolution originals preserved.",
                                r.resized_images
                            )
                        } else {
                            String::new()
                        }
                    ),
                ))
            },
            move |r| Message::Graph(GraphMessage::Compiled(revision, r)),
        )
    }
    fn graph_fields(&mut self) {
        self.graph.fields.clear();
        if let Some(doc) = &self.graph.document
            && let Some(id) = &self.graph.selected
            && let Some(n) = doc.node(id)
        {
            let op = serde_json::to_value(&n.op).unwrap();
            for (key, value) in op.as_object().unwrap() {
                if key == "type" || ports(&n.op).contains(&key.as_str()) {
                    continue;
                }
                let text = if key == "value" {
                    value_text(&serde_json::from_value::<ParameterValue>(value.clone()).unwrap())
                } else {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string())
                };
                self.graph.fields.insert(key.clone(), text);
            }
            if let Op::Parameter { parameter } = &n.op
                && let Some(p) = doc.graph.parameters.iter().find(|p| p.id == *parameter)
            {
                self.graph
                    .fields
                    .insert("default".into(), value_text(&p.default));
                self.graph.fields.insert(
                    "min".into(),
                    p.min.map(|v| v.to_string()).unwrap_or_default(),
                );
                self.graph.fields.insert(
                    "max".into(),
                    p.max.map(|v| v.to_string()).unwrap_or_default(),
                );
            }
        }
    }
    pub(super) fn graph_update(&mut self, message: GraphMessage) -> Task<Message> {
        use GraphMessage::*;
        let imported = matches!(&message, AssetChosen(_));
        match message {
            Compiled(revision, result) => {
                if revision != self.graph.revision {
                    return Task::none();
                }
                self.busy = false;
                match result {
                    Ok((wallpaper, report)) => {
                        self.generation += 1;
                        self.graph.preview = Some(Preview {
                            generation: self.generation,
                            parameters: Arc::new(
                                wallpaper
                                    .parameters()
                                    .iter()
                                    .map(|p| (p.id.clone(), p.default.clone()))
                                    .collect(),
                            ),
                            wallpaper,
                            globals: Globals::new(1, 1),
                            status: Arc::new(Mutex::new(Status::default())),
                        });
                        self.graph.report = report;
                        self.graph.valid = true;
                        self.graph.diagnostic.clear();
                        self.status = "Graph compiled".into();
                    }
                    Err(e) => {
                        self.graph.diagnostic = e;
                        self.status = "Graph needs attention; previous preview retained".into();
                    }
                }
            }
            Opened(result) | AssetChosen(result) => {
                self.busy = false;
                match result {
                    Ok(Some(doc)) => {
                        self.graph.document = Some(doc);
                        self.graph.zoom = 0.7;
                        self.graph.pan = Vector::new(12., 12.);
                        self.graph.selected = None;
                        self.graph.wire = None;
                        self.graph.dirty = imported;
                        self.page = Page::Graph;
                        self.graph_fit();
                        return self.graph_compile();
                    }
                    Ok(None) => {}
                    Err(e) => self.status = e,
                }
            }
            Saved(result) => {
                self.busy = false;
                match result {
                    Ok(true) => {
                        self.graph.dirty = false;
                        self.status = "Graph project saved".into();
                    }
                    Ok(false) => {}
                    Err(e) => self.status = e,
                }
            }
            Metadata(author, value) if !self.busy => {
                if let Some(doc) = &mut self.graph.document {
                    let doc = Arc::make_mut(doc);
                    if author {
                        doc.project.author = value;
                    } else {
                        doc.project.name = value;
                    }
                    self.graph.dirty = true;
                }
            }
            FieldEdit(key, value) => {
                self.graph.fields.insert(key, value);
            }
            Search(s) => self.graph.search = s,
            Select(id) => {
                self.graph.selected = Some(id);
                self.graph_fields();
            }
            Fit => self.graph_fit(),
            CanvasSize(size) => {
                self.graph.canvas_size = size;
            }
            Pan(delta) => self.graph.pan += delta,
            Zoom(delta, cursor) => {
                let old = self.graph.zoom.max(0.1);
                self.graph.zoom = (old * (1. + delta * 0.1)).clamp(0.1, 1.8);
                self.graph.pan = Vector::new(
                    cursor.x - (cursor.x - self.graph.pan.x) * self.graph.zoom / old,
                    cursor.y - (cursor.y - self.graph.pan.y) * self.graph.zoom / old,
                );
            }
            CancelWire => self.graph.wire = None,
            Move(id, p) if !self.busy => {
                if p.iter().all(|v| v.is_finite() && v.abs() <= 100000.)
                    && let Some(doc) = &mut self.graph.document
                {
                    Arc::make_mut(doc).layout.positions.insert(id, p);
                    self.graph.dirty = true;
                }
            }
            _ if self.busy => {}
            New => {
                self.graph.document = Some(Arc::new(Document::default()));
                self.graph.selected = None;
                self.graph.wire = None;
                self.graph.zoom = 1.;
                self.graph.pan = Vector::default();
                self.graph.dirty = true;
                return self.graph_compile();
            }
            FromEasy => {
                if let Err(e) = self.flush_paint() {
                    self.status = format!("Could not preserve painted mask: {e:#}");
                    return Task::none();
                }
                if let Some(draft) = self.designer.draft.clone() {
                    self.busy = true;
                    return work(
                        move || Document::from_easy(&draft).map(|d| Some(Arc::new(d))),
                        |r| Message::Graph(AssetChosen(r)),
                    );
                }
            }
            Open(path) => {
                self.busy = true;
                return work(
                    move || Document::open(&path).map(|d| Some(Arc::new(d))),
                    |r| Message::Graph(Opened(r)),
                );
            }
            Browse => {
                self.busy = true;
                return Task::perform(
                    async {
                        let Some(file) = rfd::AsyncFileDialog::new()
                            .set_title("Open graph project")
                            .pick_folder()
                            .await
                        else {
                            return Ok(None);
                        };
                        let path = file.path().to_owned();
                        tokio::task::spawn_blocking(move || Document::open(&path))
                            .await
                            .map_err(|e| e.to_string())?
                            .map(|d| Some(Arc::new(d)))
                            .map_err(|e| format!("{e:#}"))
                    },
                    |r| Message::Graph(Opened(r)),
                );
            }
            Save(project) => {
                if let Some(doc) = self.graph.document.clone() {
                    self.busy = true;
                    return Task::perform(
                        async move {
                            let Some(file) = rfd::AsyncFileDialog::new()
                                .set_title(if project {
                                    "Save new graph project folder"
                                } else {
                                    "Export wallpaper"
                                })
                                .set_file_name(if project {
                                    "graph-project"
                                } else {
                                    "graph.wisp"
                                })
                                .save_file()
                                .await
                            else {
                                return Ok(false);
                            };
                            let path = file.path().to_owned();
                            tokio::task::spawn_blocking(move || {
                                if project {
                                    doc.save(&path)
                                } else {
                                    doc.export(&path)
                                }
                            })
                            .await
                            .map_err(|e| e.to_string())?
                            .map(|_| true)
                            .map_err(|e| format!("{e:#}"))
                        },
                        move |r| {
                            if project {
                                Message::Graph(Saved(r))
                            } else {
                                Message::Exported(r)
                            }
                        },
                    );
                }
            }
            Install => {
                if let Some(doc) = self.graph.document.clone() {
                    self.busy = true;
                    let library = self.library.clone();
                    return work(
                        move || {
                            library
                                .install(&doc.compile()?.package, "Visual Graph")
                                .map(|i| Some(Box::new(i)))
                        },
                        Message::Imported,
                    );
                }
            }
            Asset => {
                if let Some(doc) = self.graph.document.clone() {
                    self.busy = true;
                    return Task::perform(
                        async move {
                            let Some(file) = rfd::AsyncFileDialog::new()
                                .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                                .pick_file()
                                .await
                            else {
                                return Ok(None);
                            };
                            let path = file.path().to_owned();
                            tokio::task::spawn_blocking(move || {
                                let mut d = (*doc).clone();
                                d.import_image(&path)?;
                                Ok::<_, anyhow::Error>(Some(Arc::new(d)))
                            })
                            .await
                            .map_err(|e| e.to_string())?
                            .map_err(|e| format!("{e:#}"))
                        },
                        |r| Message::Graph(AssetChosen(r)),
                    );
                }
            }
            Wire(id) => {
                self.graph.wire = Some(id);
                self.status = "Choose an input socket. Right-click to cancel.".into();
            }
            Arrange => {
                if let Some(d) = &mut self.graph.document {
                    Arc::make_mut(d).arrange();
                    self.graph.pan = Vector::new(12., 12.);
                    self.graph.dirty = true;
                }
            }
            message => {
                let Some(doc) = &mut self.graph.document else {
                    return Task::none();
                };
                let mut candidate = (**doc).clone();
                let selected = self.graph.selected.clone().unwrap_or_default();
                let result = (|| -> Result<()> {
                    match message {
                        Add(kind) => {
                            let id = candidate.add(
                                &kind,
                                [
                                    (80. - self.graph.pan.x) / self.graph.zoom.max(0.1),
                                    (60. - self.graph.pan.y) / self.graph.zoom.max(0.1),
                                ],
                            )?;
                            self.graph.selected = Some(id);
                        }
                        Socket(target, port) => {
                            let source =
                                self.graph.wire.as_ref().ok_or_else(|| {
                                    anyhow::anyhow!("Choose an output socket first")
                                })?;
                            candidate.connect(source, &target, &port)?;
                            self.graph.wire = None;
                        }
                        Disconnect(id, port) => candidate.set_field(&id, &port, "".into())?,
                        Delete => {
                            candidate.remove(&selected);
                            self.graph.selected = None;
                            self.graph.wire = None;
                        }
                        Output => {
                            ensure!(
                                matches!(
                                    candidate
                                        .node(&selected)
                                        .context("select an Output node")?
                                        .op,
                                    Op::Output { .. }
                                ),
                                "select an Output node"
                            );
                            candidate.graph.output = selected.clone();
                        }
                        ValueType(kind) => {
                            let value = match kind.as_str() {
                                "Float" => ParameterValue::Float(1.),
                                "Integer" => ParameterValue::Integer(1),
                                "Boolean" => ParameterValue::Boolean(true),
                                "Vector2" => ParameterValue::Vector2([0., 0.]),
                                "Vector3" => ParameterValue::Vector3([0., 0., 0.]),
                                "Vector4" => ParameterValue::Vector4([0., 0., 0., 0.]),
                                _ => ParameterValue::Color([1., 1., 1., 1.]),
                            };
                            match candidate
                                .node(&selected)
                                .context("select a value node")?
                                .op
                                .clone()
                            {
                                Op::Parameter { parameter } => {
                                    let p = candidate
                                        .graph
                                        .parameters
                                        .iter_mut()
                                        .find(|p| p.id == parameter)
                                        .context("unknown parameter")?;
                                    p.default = value;
                                    p.min = None;
                                    p.max = None;
                                }
                                Op::Constant { .. } => candidate.set_field(
                                    &selected,
                                    "value",
                                    serde_json::to_value(value)?,
                                )?,
                                _ => {}
                            }
                        }
                        ParameterBound(max, value) => {
                            self.graph
                                .fields
                                .insert(if max { "max" } else { "min" }.into(), value.clone());
                            let Op::Parameter { parameter } = candidate
                                .node(&selected)
                                .context("missing node")?
                                .op
                                .clone()
                            else {
                                return Ok(());
                            };
                            let p = candidate
                                .graph
                                .parameters
                                .iter_mut()
                                .find(|p| p.id == parameter)
                                .context("unknown parameter")?;
                            let bound = if value.trim().is_empty() {
                                None
                            } else {
                                Some(value.parse::<f64>()?)
                            };
                            if max {
                                p.max = bound
                            } else {
                                p.min = bound
                            }
                            p.validate()?;
                        }
                        Field(key, value) => {
                            self.graph.fields.insert(key.clone(), value.clone());
                            let op = candidate
                                .node(&selected)
                                .context("missing node")?
                                .op
                                .clone();
                            if key == "value" || key == "default" {
                                let template = match &op {
                                    Op::Constant { value } => value.clone(),
                                    Op::Parameter { parameter } => candidate
                                        .graph
                                        .parameters
                                        .iter()
                                        .find(|p| p.id == *parameter)
                                        .context("unknown parameter")?
                                        .default
                                        .clone(),
                                    _ => anyhow::bail!("not a value node"),
                                };
                                let parsed = parse_parameter(
                                    &Parameter {
                                        id: selected.clone(),
                                        default: template,
                                        min: None,
                                        max: None,
                                    },
                                    &value,
                                )?;
                                if let Op::Parameter { parameter } = op {
                                    let p = candidate
                                        .graph
                                        .parameters
                                        .iter_mut()
                                        .find(|p| p.id == parameter)
                                        .unwrap();
                                    p.validate_value(&parsed)?;
                                    p.default = parsed;
                                } else {
                                    candidate.set_field(
                                        &selected,
                                        &key,
                                        serde_json::to_value(parsed)?,
                                    )?;
                                }
                            } else {
                                let old = serde_json::to_value(&op)?;
                                candidate.set_field(
                                    &selected,
                                    &key,
                                    if !old[&key].is_string() {
                                        serde_json::from_str(&value)?
                                    } else {
                                        value.into()
                                    },
                                )?;
                            }
                        }
                        _ => {}
                    }
                    Ok(())
                })();
                match result {
                    Ok(()) => {
                        self.graph.document = Some(Arc::new(candidate));
                        self.graph.dirty = true;
                        self.graph_fields();
                        return self.graph_compile();
                    }
                    Err(e) => {
                        self.graph.diagnostic = format!("{e:#}");
                        self.status = "Edit rejected; previous graph retained".into();
                    }
                }
            }
        }
        Task::none()
    }
    pub(super) fn graph_view(&self) -> Element<'_, Message> {
        use GraphMessage::*;
        let action = |m| Message::Graph(m);
        let enabled = !self.busy;
        let mut left = column![
            text("Visual Graph").size(24),
            row![
                button("New").on_press_maybe(enabled.then_some(action(New))),
                button("Open…").on_press_maybe(enabled.then_some(action(Browse)))
            ]
            .spacing(6),
            text_input("Project folder", &self.import_path).on_input(Message::ImportPath),
            button("Open path").on_press_maybe(
                (enabled && !self.import_path.trim().is_empty())
                    .then(|| action(Open(PathBuf::from(self.import_path.trim()))))
            ),
            button("From Easy Designer").on_press_maybe(
                (enabled && self.designer.draft.is_some()).then_some(action(FromEasy))
            ),
            text_input("Search nodes…", &self.graph.search).on_input(move |s| action(Search(s)))
        ]
        .spacing(8);
        let mut results = column![].spacing(3);
        for kind in CATALOG
            .iter()
            .filter(|k| k.to_lowercase().contains(&self.graph.search.to_lowercase()))
        {
            results = results.push(button(*kind).width(Fill).on_press_maybe(
                (enabled && self.graph.document.is_some()).then(|| action(Add((*kind).into()))),
            ));
        }
        left = left.push(scrollable(results).height(Fill));
        let mut right = column![text("Properties").size(20)].spacing(8);
        if let Some(doc) = &self.graph.document {
            right = right
                .push(
                    text_input("Name", &doc.project.name)
                        .on_input(move |v| action(Metadata(false, v))),
                )
                .push(
                    text_input("Author", &doc.project.author)
                        .on_input(move |v| action(Metadata(true, v))),
                );
            if let Some(id) = &self.graph.selected
                && let Some(n) = doc.node(id)
            {
                right =
                    right.push(text(format!("{} · {}", n.id, crate::graph::label(&n.op))).size(16));
                for (port, source) in ports(&n.op).iter().zip(n.op.inputs()) {
                    right = right.push(
                        row![
                            text(format!(
                                "{port}: {}",
                                if source.is_empty() {
                                    "unconnected"
                                } else {
                                    source
                                }
                            ))
                            .size(12),
                            button("×").on_press_maybe(
                                enabled.then(|| action(Disconnect(id.clone(), (*port).into())))
                            )
                        ]
                        .spacing(4),
                    );
                }
                if matches!(n.op, Op::Constant { .. } | Op::Parameter { .. }) {
                    let types = [
                        "Float", "Integer", "Boolean", "Color", "Vector2", "Vector3", "Vector4",
                    ];
                    right = right.push(
                        pick_list(types, None::<&str>, move |v| action(ValueType(v.into())))
                            .placeholder("Value type"),
                    );
                }
                for (key, value) in &self.graph.fields {
                    let key = key.clone();
                    right = right.push(text(key.clone()).size(12));
                    if key == "asset" {
                        right = right.push(pick_list(
                            doc.assets.keys().cloned().collect::<Vec<_>>(),
                            Some(value.clone()),
                            move |v| action(Field("asset".into(), v)),
                        ));
                    } else {
                        let apply = if key == "min" || key == "max" {
                            action(ParameterBound(key == "max", value.clone()))
                        } else {
                            action(Field(key.clone(), value.clone()))
                        };
                        right = right.push(
                            row![
                                text_input(&key, value)
                                    .on_input(move |v| action(FieldEdit(key.clone(), v)))
                                    .on_submit(apply.clone()),
                                button("Set").on_press_maybe(enabled.then_some(apply))
                            ]
                            .spacing(4),
                        );
                    }
                }
                if matches!(n.op, Op::Output { .. }) {
                    right = right.push(
                        button("Use as output").on_press_maybe(enabled.then_some(action(Output))),
                    );
                }
                right = right
                    .push(button("Delete node").on_press_maybe(enabled.then_some(action(Delete))));
            } else {
                right = right.push(text("Select a node to edit its properties."));
            }
            right = right
                .push(button("Add image asset…").on_press_maybe(enabled.then_some(action(Asset))));
            right = right.push(
                text(if self.graph.dirty {
                    "Draft · unsaved changes"
                } else {
                    "Draft saved"
                })
                .size(12),
            );
            right = right.push(
                button("Save project…").on_press_maybe(enabled.then_some(action(Save(true)))),
            );
            right = right.push(
                button("Export .wisp…")
                    .on_press_maybe((enabled && self.graph.valid).then_some(action(Save(false)))),
            );
            right = right.push(
                button("Save to library")
                    .on_press_maybe((enabled && self.graph.valid).then_some(action(Install))),
            );
        }
        let mut center = column![
            row![
                button("Arrange").on_press_maybe(enabled.then_some(action(Arrange))),
                button("Fit").on_press(action(Fit)),
                text(format!("{:.0}%", self.graph.zoom * 100.)),
                text("Drag nodes · scroll to zoom · drag background to pan").size(12)
            ]
            .spacing(8),
            canvas(super::graph_canvas::GraphCanvas(&self.graph, self.busy))
                .width(Fill)
                .height(Fill),
            text(if self.graph.wire.is_some() {
                "Select an input socket to connect; right-click cancels."
            } else {
                "Connect an output dot to an input dot. Select a node to disconnect inputs."
            })
            .size(12)
        ]
        .spacing(8);
        if let Some(p) = &self.graph.preview {
            center = center.push(
                row![
                    iced::widget::Shader::new(p.clone()).width(Fill).height(180),
                    column![
                        button(if self.preview_paused { "Play" } else { "Pause" })
                            .on_press(Message::PausePreview),
                        text(&self.graph.report).size(12)
                    ]
                    .width(145)
                ]
                .spacing(8),
            );
        }
        if !self.graph.diagnostic.is_empty() {
            center = center.push(
                scrollable(
                    text(&self.graph.diagnostic)
                        .size(13)
                        .color(iced::Color::from_rgb(1., 0.55, 0.5)),
                )
                .height(65),
            );
        }
        row![
            left.width(190),
            center.width(Fill),
            scrollable(right).width(240)
        ]
        .spacing(14)
        .height(Fill)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> App {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _) = App::new(
            Library::open(Some(dir.path().join("library"))).unwrap(),
            Settings::default(),
            None,
            None,
        );
        app.busy = false;
        app.graph.document = Some(Arc::new(Document::default()));
        app
    }
    #[test]
    fn invalid_and_stale_compiles_keep_last_preview_and_values_apply_explicitly() {
        let mut app = app();
        let wallpaper = Arc::new(
            Wallpaper::from_package(
                &app.graph
                    .document
                    .as_ref()
                    .unwrap()
                    .compile()
                    .unwrap()
                    .package,
            )
            .unwrap(),
        );
        let _ = app.graph_update(GraphMessage::Compiled(
            0,
            Ok((wallpaper, "one pass".into())),
        ));
        let generation = app.graph.preview.as_ref().unwrap().generation;
        let _ = app.graph_update(GraphMessage::Select("color".into()));
        let _ = app.graph_update(GraphMessage::FieldEdit("value".into(), "[0.2,".into()));
        assert_eq!(app.graph.fields["value"], "[0.2,");
        assert!(!app.busy);
        let _ = app.graph_update(GraphMessage::Field("value".into(), "[0.2,".into()));
        assert!(!app.graph.diagnostic.is_empty());
        assert_eq!(app.graph.preview.as_ref().unwrap().generation, generation);
        let _ = app.graph_update(GraphMessage::Field(
            "value".into(),
            "[0.2,0.5,0.8,1]".into(),
        ));
        assert!(app.busy);
        let revision = app.graph.revision;
        let _ = app.graph_update(GraphMessage::Compiled(revision - 1, Err("stale".into())));
        assert!(app.busy);
        let _ = app.graph_update(GraphMessage::Compiled(revision, Err("node failure".into())));
        assert!(!app.graph.valid);
        assert!(!app.busy);
        assert_eq!(app.graph.preview.as_ref().unwrap().generation, generation);
        assert!(app.graph.dirty);
        let _ = app.graph_update(GraphMessage::Saved(Ok(false)));
        assert!(app.graph.dirty);
        let _ = app.graph_update(GraphMessage::Saved(Err("disk full".into())));
        assert!(app.graph.dirty);
        let _ = app.graph_update(GraphMessage::Saved(Ok(true)));
        assert!(!app.graph.dirty);
    }
}
