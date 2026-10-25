use super::*;
use crate::graph::{CATALOG, Document, ports};
use iced::{
    Point, Vector,
    widget::{canvas, column},
};
use wisp_ir::Op;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GraphPanel {
    Nodes,
    Properties,
    Preview,
    Project,
}

#[derive(Clone)]
pub(super) enum GraphMessage {
    Panel(GraphPanel),
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
    SaveTo(bool, PathBuf),
    AssetPath(PathBuf),
    Saved(Outcome<bool>),
    Install,
}
#[derive(Default)]
pub(super) struct GraphEditor {
    pub panel: Option<GraphPanel>,
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
        self.graph.pan = Vector::new(
            (width - (max_x - min_x) * self.graph.zoom) * 0.5 - min_x * self.graph.zoom,
            (height - (max_y - min_y) * self.graph.zoom) * 0.5 - min_y * self.graph.zoom,
        );
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
            Panel(panel) => {
                self.graph.panel = (self.graph.panel != Some(panel)).then_some(panel);
            }
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
                let first_layout =
                    self.graph.canvas_size.width <= 0. || self.graph.canvas_size.height <= 0.;
                self.graph.canvas_size = size;
                if first_layout {
                    self.graph_fit();
                }
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
            SaveTo(project, path) => {
                if let Some(doc) = self.graph.document.clone() {
                    self.busy = true;
                    return work(
                        move || save_to(&doc, &path, project),
                        move |result| {
                            if project {
                                Message::Graph(Saved(result))
                            } else {
                                Message::Exported(result)
                            }
                        },
                    );
                }
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
                            tokio::task::spawn_blocking(move || save_to(&doc, &path, project))
                                .await
                                .map_err(|e| e.to_string())?
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
            AssetPath(path) => {
                if let Some(doc) = self.graph.document.clone() {
                    self.busy = true;
                    return work(
                        move || import_image(&doc, &path),
                        |result| Message::Graph(AssetChosen(result)),
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
    fn graph_properties(&self) -> Element<'_, Message> {
        use GraphMessage::*;
        let action = |m| Message::Graph(m);
        let enabled = !self.busy;
        let mut right = column![].spacing(12);
        if let Some(doc) = &self.graph.document {
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
                            icon_button(
                                Icon::Disconnect,
                                "Disconnect input",
                                enabled.then(|| action(Disconnect(id.clone(), (*port).into()))),
                                false
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
                right = right.push(icon_button(
                    Icon::Remove,
                    "Delete node",
                    enabled.then_some(action(Delete)),
                    false,
                ));
            } else {
                right = right.push(text("Select a node to edit its properties."));
            }
        }
        right.into()
    }
    pub(super) fn graph_view(&self) -> Element<'_, Message> {
        use GraphMessage::*;
        use GraphPanel as PanelKind;
        let action = |m| Message::Graph(m);
        let enabled = !self.busy;
        let Some(doc) = &self.graph.document else {
            return container(
                column![
                    text("Make room for an idea.").size(32),
                    text("Build a wallpaper, one node at a time.").size(15),
                    space().height(12),
                    row![
                        button("New graph")
                            .style(iced_button::primary)
                            .on_press_maybe(enabled.then_some(action(New))),
                        button("Open project…").on_press_maybe(enabled.then_some(action(Browse))),
                    ]
                    .spacing(12),
                    text_input("Project folder", &self.import_path)
                        .on_input(Message::ImportPath)
                        .width(420),
                    button("Open path").on_press_maybe(
                        (!self.busy && !self.import_path.trim().is_empty())
                            .then(|| action(Open(PathBuf::from(self.import_path.trim())))),
                    ),
                    button("Continue from Designer").on_press_maybe(
                        (enabled && self.designer.draft.is_some()).then_some(action(FromEasy))
                    ),
                ]
                .spacing(14)
                .align_x(iced::Center),
            )
            .center(Fill)
            .style(style::stage)
            .into();
        };
        let canvas = canvas(super::graph_canvas::GraphCanvas(&self.graph, self.busy))
            .width(Fill)
            .height(Fill);
        if self.focus_mode {
            return iced::widget::stack![
                canvas,
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
                self.graph.panel == Some(panel),
            )
        };
        let toolbar = row![
            text(&doc.project.name).size(15),
            space().width(Fill),
            tool(Icon::Add, "Add node", PanelKind::Nodes),
            tool(Icon::Adjust, "Properties", PanelKind::Properties),
            tool(Icon::Display, "Preview", PanelKind::Preview),
            tool(Icon::Project, "Project", PanelKind::Project),
            icon_button(Icon::Focus, "Focus · F11", Some(Message::Focus), false),
        ]
        .spacing(4)
        .align_y(iced::Center)
        .padding([4, 8]);
        let canvas: Element<'_, Message> = if self.graph.panel != Some(PanelKind::Preview)
            && let Some(p) = &self.graph.preview
        {
            iced::widget::stack![
                canvas,
                container(
                    container(p.view(135.))
                        .width(240)
                        .padding(6)
                        .style(style::panel)
                )
                .padding(16)
                .align_right(Fill)
                .align_bottom(Fill),
            ]
            .into()
        } else {
            canvas.into()
        };
        let mut stage = column![canvas].spacing(8).height(Fill);
        if !self.graph.diagnostic.is_empty() {
            stage = stage.push(
                scrollable(
                    text(&self.graph.diagnostic)
                        .size(12)
                        .color(iced::Color::from_rgb(0.9, 0.5, 0.45)),
                )
                .height(64),
            );
        }
        stage = stage.push(
            row![
                icon_button(
                    Icon::Arrange,
                    "Arrange nodes",
                    enabled.then_some(action(Arrange)),
                    false
                ),
                icon_button(Icon::Focus, "Fit graph to view", Some(action(Fit)), false),
                text(format!("{:.0}%", self.graph.zoom * 100.)).size(11),
                space().width(Fill),
                text(if self.graph.wire.is_some() {
                    "Choose an input · Right-click to cancel"
                } else {
                    "Drag to pan · Scroll to zoom"
                })
                .size(11),
            ]
            .spacing(8)
            .align_y(iced::Center),
        );
        let mut workspace = row![stage].spacing(8).height(Fill);
        if let Some(panel) = self.graph.panel {
            let title = match panel {
                PanelKind::Nodes => "Add a node",
                PanelKind::Properties => "Properties",
                PanelKind::Preview => "Live preview",
                PanelKind::Project => "Project",
            };
            let mut content = column![].spacing(14);
            match panel {
                PanelKind::Nodes => {
                    content = content.push(
                        text_input("Find a node…", &self.graph.search)
                            .on_input(move |s| action(Search(s))),
                    );
                    let query = self.graph.search.to_lowercase();
                    let matches: Vec<_> = CATALOG
                        .iter()
                        .filter(|k| k.to_lowercase().contains(&query))
                        .collect();
                    if matches.is_empty() {
                        content = content.push(text("No matching nodes.").size(13));
                    }
                    for kind in matches {
                        content = content.push(
                            button(*kind)
                                .width(Fill)
                                .on_press_maybe(enabled.then(|| action(Add((*kind).into())))),
                        );
                    }
                }
                PanelKind::Properties => content = content.push(self.graph_properties()),
                PanelKind::Preview => {
                    if let Some(p) = &self.graph.preview {
                        content = content
                            .push(p.view(220.))
                            .push(self.playback_button())
                            .push(text(&self.graph.report).size(11));
                    } else {
                        content = content.push(text("Preparing preview…").size(13));
                    }
                }
                PanelKind::Project => {
                    content = content
                        .push(text("Name").size(12))
                        .push(
                            text_input("Name", &doc.project.name)
                                .on_input(move |v| action(Metadata(false, v))),
                        )
                        .push(text("Author").size(12))
                        .push(
                            text_input("Author", &doc.project.author)
                                .on_input(move |v| action(Metadata(true, v))),
                        )
                        .push(
                            text(if self.graph.dirty {
                                "Unsaved changes"
                            } else {
                                "Project saved"
                            })
                            .size(11),
                        )
                        .push(
                            button("Save project…")
                                .style(iced_button::primary)
                                .width(Fill)
                                .on_press_maybe(enabled.then_some(action(Save(true)))),
                        )
                        .push(button("Export wallpaper…").width(Fill).on_press_maybe(
                            (enabled && self.graph.valid).then_some(action(Save(false))),
                        ))
                        .push(button("Save to library").width(Fill).on_press_maybe(
                            (enabled && self.graph.valid).then_some(action(Install)),
                        ))
                        .push(
                            button("Add image asset…")
                                .width(Fill)
                                .on_press_maybe(enabled.then_some(action(Asset))),
                        )
                        .push(iced::widget::rule::horizontal(1))
                        .push(
                            row![
                                button("New").on_press_maybe(enabled.then_some(action(New))),
                                button("Open…").on_press_maybe(enabled.then_some(action(Browse)))
                            ]
                            .spacing(8),
                        )
                        .push(button("From Designer").on_press_maybe(
                            (enabled && self.designer.draft.is_some()).then_some(action(FromEasy)),
                        ))
                        .push(
                            text_input("Project folder", &self.import_path)
                                .on_input(Message::ImportPath),
                        )
                        .push(
                            button("Open path").on_press_maybe(
                                (enabled && !self.import_path.trim().is_empty())
                                    .then(|| action(Open(PathBuf::from(self.import_path.trim())))),
                            ),
                        );
                }
            }
            workspace = workspace.push(motion::entrance(
                container(
                    column![
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
                        scrollable(content).height(Fill),
                    ]
                    .spacing(16),
                )
                .padding(16)
                .width(300)
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
    fn initial_layout_fits_the_actual_canvas_without_resetting_later_navigation() {
        let mut app = app();
        app.graph_fit();
        let fallback_pan = app.graph.pan;
        let _ = app.graph_update(GraphMessage::CanvasSize(iced::Size::new(1200., 900.)));
        assert_ne!(app.graph.pan, fallback_pan);
        app.graph.pan = Vector::new(100., 80.);
        app.graph.zoom = 0.6;
        let _ = app.graph_update(GraphMessage::CanvasSize(iced::Size::new(900., 620.)));
        assert_eq!(app.graph.pan, Vector::new(100., 80.));
        assert_eq!(app.graph.zoom, 0.6);
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

fn save_to(doc: &Document, path: &std::path::Path, project: bool) -> Result<bool> {
    if project {
        doc.save(path)?;
    } else {
        doc.export(path)?;
    }
    Ok(true)
}
fn import_image(doc: &Document, path: &std::path::Path) -> Result<Option<Arc<Document>>> {
    let mut doc = doc.clone();
    doc.import_image(path)?;
    Ok(Some(Arc::new(doc)))
}

#[cfg(test)]
mod path_tests {
    use super::*;
    #[test]
    fn path_import_and_save_preserve_editable_graph_and_recover_after_failure() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("image.png");
        std::fs::write(&source, wisp_runtime::EXAMPLE_IMAGE).unwrap();
        let doc = Document::default();
        let imported = import_image(&doc, &source).unwrap().unwrap();
        assert_eq!(imported.assets.len(), doc.assets.len() + 1);
        assert!(import_image(&imported, &dir.path().join("missing")).is_err());
        let project = dir.path().join("graph");
        let exported = dir.path().join("graph.wisp");
        assert!(save_to(&imported, &project, true).unwrap());
        assert!(save_to(&imported, &exported, false).unwrap());
        let reopened = Document::open(&project).unwrap();
        assert_eq!(reopened.graph.nodes.len(), imported.graph.nodes.len());
        assert_eq!(reopened.assets.len(), imported.assets.len());
        assert!(save_to(&imported, &project, true).is_err());
        assert!(save_to(&imported, &exported, false).is_err());
    }
}
