#[path = "audit_ui.rs"]
mod audit_ui;
#[path = "designer_ui.rs"]
mod designer_ui;
#[path = "graph_canvas.rs"]
mod graph_canvas;
#[path = "graph_ui.rs"]
mod graph_ui;
#[path = "icons.rs"]
mod icons;
#[path = "motion.rs"]
mod motion;
#[cfg(test)]
#[path = "slider_tests.rs"]
mod slider_tests;
#[path = "style.rs"]
mod style;
use crate::{
    daemon,
    library::{Item, Library, Settings},
    preview::{Preview, Status},
};
use anyhow::{Context, Result, ensure};
use designer_ui::{Designer, DesignerMessage};
use iced::{
    Element,
    Length::Fill,
    Subscription, Task,
    widget::{
        column, container, image, row, scrollable as iced_scrollable, space, text as iced_text,
    },
};
use icons::{Icon, icon_button};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use wisp_format::{Parameter, ParameterValue};
use wisp_host::{Assignment, Config, MonitorInfo};
use wisp_runtime::{Globals, Wallpaper};

fn button<'a>(content: impl Into<Element<'a, Message>>) -> motion::Button<'a, Message> {
    motion::Button::new(content)
        .padding([8, 12])
        .style(style::secondary)
}

fn text<'a>(content: impl AsRef<str>) -> iced::widget::Text<'a> {
    iced_text(content.as_ref().to_lowercase()).font(style::FONT)
}

fn text_input<'a>(placeholder: &str, value: &str) -> iced::widget::TextInput<'a, Message> {
    iced::widget::text_input(&placeholder.to_lowercase(), value)
        .font(style::FONT)
        .padding([8, 10])
        .style(style::input)
}

fn checkbox<'a>(checked: bool) -> iced::widget::Checkbox<'a, Message> {
    iced::widget::checkbox(checked)
        .size(15)
        .spacing(9)
        .style(style::check)
}

fn slider<'a, T>(
    range: std::ops::RangeInclusive<T>,
    value: T,
    on_change: impl Fn(T) -> Message + 'a,
) -> iced::widget::Slider<'a, T, Message>
where
    T: Copy + From<u8> + PartialOrd,
{
    iced::widget::slider(range, value, on_change).style(style::range)
}

// Keep the original option intact so display casing cannot alter a device ID or graph value.
#[derive(Clone, PartialEq)]
struct LowercaseOption<T>(T);
impl<T: ToString> std::fmt::Display for LowercaseOption<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.to_string().to_lowercase())
    }
}

fn pick_list<'a, T, L, V>(
    options: L,
    selected: Option<V>,
    on_selected: impl Fn(T) -> Message + 'a,
) -> iced::widget::PickList<
    'a,
    LowercaseOption<T>,
    Vec<LowercaseOption<T>>,
    LowercaseOption<T>,
    Message,
>
where
    T: ToString + PartialEq + Clone + 'a,
    L: std::borrow::Borrow<[T]>,
    V: std::borrow::Borrow<T>,
{
    iced::widget::pick_list(
        options
            .borrow()
            .iter()
            .cloned()
            .map(LowercaseOption)
            .collect::<Vec<_>>(),
        selected.map(|value| LowercaseOption(value.borrow().clone())),
        move |value| on_selected(value.0),
    )
    .font(style::FONT)
    .padding([8, 10])
    .style(style::choice)
    .menu_style(style::menu)
}

fn scrollable<'a>(
    content: impl Into<Element<'a, Message>>,
) -> iced::widget::Scrollable<'a, Message> {
    iced_scrollable(content)
        .style(style::scroll)
        .direction(iced_scrollable::Direction::Vertical(
            iced_scrollable::Scrollbar::new()
                .width(3)
                .scroller_width(3)
                .spacing(10),
        ))
}

pub struct LaunchOptions {
    pub data_dir: Option<PathBuf>,
    pub import: Option<PathBuf>,
    pub create: Option<PathBuf>,
    pub graph: Option<PathBuf>,
    pub check: bool,
    pub frames: Option<u64>,
    pub screenshot: Option<PathBuf>,
    pub screenshot_delay: Duration,
    pub audit_commands: Option<PathBuf>,
}
pub fn run(mut options: LaunchOptions) -> Result<()> {
    if options.import.as_ref().is_some_and(|p| is_image(p)) {
        options.create = options.import.take();
    }
    let library = Library::open(options.data_dir)?;
    let _lock = library.lock()?;
    let settings = library.settings()?;
    if options.check {
        if let Some(path) = &options.graph {
            let compiled = crate::graph::Document::open(path)?.compile()?;
            println!("Valid graph wallpaper: {} passes", compiled.report.passes);
        }
        if let Some(path) = &options.create {
            let draft = if path.is_dir() {
                crate::designer::Draft::open_project(path)?
            } else {
                crate::designer::Draft::from_image(path)?
            };
            let compiled = draft.compile()?;
            println!(
                "Valid Easy Designer wallpaper: {} passes",
                compiled.report.passes
            );
        }
        if let Some(path) = options.import {
            library.import(&path)?;
        }
        let (items, errors) = library.scan()?;
        ensure!(
            errors.is_empty(),
            "invalid library entries:\n{}",
            errors.join("\n")
        );
        library.config()?.validate()?;
        println!("Valid library: {} wallpapers", items.len());
        return Ok(());
    }
    let audit_commands = audit_ui::load(options.audit_commands.as_deref())
        .inspect_err(|error| eprintln!("AUDIT_FAILED: {error:#}"))?;
    let exit_failure = Arc::new(Mutex::new(None::<String>));
    let app_exit_failure = exit_failure.clone();
    iced::application(
        move || {
            let (mut app, task) = App::new(
                library.clone(),
                settings.clone(),
                options.import.clone(),
                options.frames,
            );
            app.pending_design = options.create.clone();
            app.pending_graph = options.graph.clone();
            app.final_screenshot = options.screenshot.clone();
            app.screenshot_delay = options.screenshot_delay;
            app.exit_failure = app_exit_failure.clone();
            app.audit_commands = audit_commands.clone();
            (app, task)
        },
        App::update,
        App::view,
    )
    .title("wisp")
    .settings(iced::Settings {
        default_text_size: 15.into(),
        default_font: style::FONT,
        fonts: vec![style::FONT_BYTES.into()],
        ..Default::default()
    })
    .theme(|app: &App| style::theme(app.settings.dark_theme))
    .subscription(App::subscription)
    .window(iced::window::Settings {
        size: iced::Size::new(1180.0, 820.0),
        min_size: Some(iced::Size::new(900.0, 620.0)),
        platform_specific: iced::window::settings::PlatformSpecific {
            #[cfg(target_os = "linux")]
            application_id: "org.wisp.Wisp".into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .run()?;
    if let Some(error) = exit_failure.lock().unwrap().take() {
        anyhow::bail!(error);
    }
    Ok(())
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Library,
    Designer,
    Graph,
    Import,
    Settings,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum LibraryPanel {
    Controls,
    Display,
    Details,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct MonitorChoice {
    id: Option<String>,
    label: String,
}
impl std::fmt::Display for MonitorChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.label.fmt(f)
    }
}
type Outcome<T> = std::result::Result<T, String>;
#[derive(Clone)]
enum Message {
    Designer(DesignerMessage),
    Graph(graph_ui::GraphMessage),
    Page(Page),
    Focus,
    Dismiss,
    LibraryPanel(LibraryPanel),
    Search(String),
    Select(String),
    Scanned(Outcome<(Vec<Item>, Vec<String>)>),
    Loaded(Outcome<(String, Arc<Wallpaper>, BTreeMap<String, ParameterValue>)>),
    Browse(bool),
    ImportPath(String),
    Import,
    Imported(Outcome<Option<Box<Item>>>),
    Export,
    ExportChosen(Option<PathBuf>),
    ExportTo(PathBuf),
    Exported(Outcome<bool>),
    RefreshMonitors,
    Monitors(Outcome<Vec<MonitorInfo>>),
    Monitor(MonitorChoice),
    Parameter(String, String),
    Boolean(String, bool),
    Float(String, f32),
    Color(String, usize, f32),
    Reset,
    Apply,
    Clear,
    Applied(Outcome<()>),
    Start,
    Started(Outcome<()>),
    DesktopFps(u32),
    PreviewFps(u32),
    Dark(bool),
    SaveSettings,
    AudioEnabled(bool),
    AudioDevice(AudioChoice),
    RefreshAudio,
    AudioDevices(Outcome<Vec<(String, String)>>),
    AudioGain(f32),
    PausePreview,
    Tick(Instant),
    Poll,
    AuditPoll,
    DaemonStatus(Outcome<bool>),
    Event(iced::Event),
    Screenshot(iced::window::Screenshot),
}
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Wisp event {:?}", std::mem::discriminant(self))
    }
}
#[derive(Clone, Debug, PartialEq)]
struct AudioChoice {
    id: Option<String>,
    name: String,
}
impl std::fmt::Display for AudioChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.name.fmt(f)
    }
}
struct App {
    audio_devices: Vec<AudioChoice>,
    audio: wisp_audio::Service,
    designer: Designer,
    graph: graph_ui::GraphEditor,
    pending_graph: Option<PathBuf>,
    pending_design: Option<PathBuf>,
    library: Library,
    settings: Settings,
    config: Config,
    items: Vec<Item>,
    thumbnails: BTreeMap<String, image::Handle>,
    page: Page,
    focus_mode: bool,
    library_panel: Option<LibraryPanel>,
    search: String,
    selected: Option<String>,
    preview: Option<Preview>,
    generation: u64,
    parameter_text: BTreeMap<String, String>,
    parameter_errors: BTreeMap<String, String>,
    monitors: Vec<MonitorChoice>,
    monitor: MonitorChoice,
    assignment_fps: u32,
    import_path: String,
    status: String,
    busy: bool,
    discovering: bool,
    monitor_error: Option<String>,
    daemon_polling: bool,
    daemon_running: bool,
    preview_paused: bool,
    focused: bool,
    last_tick: Instant,
    frame_limit: Option<u64>,
    final_screenshot: Option<PathBuf>,
    started: Instant,
    screenshot_delay: Duration,
    active_capture: Option<audit_ui::Capture>,
    startup_error: Option<String>,
    capture_operation_errors: BTreeMap<audit_ui::CaptureOperation, String>,
    exit_failure: Arc<Mutex<Option<String>>>,
    audit_commands: std::collections::VecDeque<audit_ui::Command>,
    audit_wait_until: Option<Instant>,
    audit_state_wait_started: Option<Instant>,
}
fn work<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
    message: impl FnOnce(Outcome<T>) -> Message + Send + 'static,
) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(job)
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| format!("{e:#}"))
        },
        message,
    )
}
impl App {
    fn new(
        library: Library,
        settings: Settings,
        import: Option<PathBuf>,
        frames: Option<u64>,
    ) -> (Self, Task<Message>) {
        let monitor = MonitorChoice {
            id: None,
            label: "All monitors".into(),
        };
        let config_result = library.config();
        let status = config_result
            .as_ref()
            .err()
            .map(|e| format!("Could not read assignments: {e:#}"))
            .unwrap_or_else(|| "Loading your library…".into());
        let loader = library.clone();
        let audio = wisp_audio::Service::new(
            config_result
                .as_ref()
                .map(|c| c.audio.clone())
                .unwrap_or_default(),
        );
        let config_result_error = config_result
            .as_ref()
            .err()
            .map(|error| format!("{error:#}"));
        let app = Self {
            audio,
            audio_devices: vec![AudioChoice {
                id: None,
                name: "System output".into(),
            }],
            designer: Designer::default(),
            graph: graph_ui::GraphEditor {
                zoom: 1.,
                ..Default::default()
            },
            pending_graph: None,
            pending_design: None,
            library,
            config: config_result.unwrap_or_default(),
            assignment_fps: settings.desktop_fps,
            settings,
            items: vec![],
            thumbnails: BTreeMap::new(),
            page: Page::Library,
            focus_mode: false,
            library_panel: None,
            search: String::new(),
            selected: None,
            preview: None,
            generation: 0,
            parameter_text: BTreeMap::new(),
            parameter_errors: BTreeMap::new(),
            monitors: vec![monitor.clone()],
            monitor,
            import_path: String::new(),
            status,
            busy: true,
            discovering: true,
            monitor_error: None,
            daemon_polling: false,
            daemon_running: false,
            preview_paused: false,
            focused: true,
            last_tick: Instant::now(),
            frame_limit: frames,
            final_screenshot: None,
            started: Instant::now(),
            screenshot_delay: Duration::from_secs(1),
            active_capture: None,
            startup_error: config_result_error,
            capture_operation_errors: BTreeMap::new(),
            exit_failure: Arc::new(Mutex::new(None)),
            audit_commands: Default::default(),
            audit_wait_until: None,
            audit_state_wait_started: None,
        };
        let scan = work(
            move || {
                let imported = import.map(|path| loader.import(&path)).transpose()?;
                let (mut items, errors) = loader.scan()?;
                if let Some(imported) = imported
                    && let Some(index) = items
                        .iter()
                        .position(|item| item.record.key == imported.record.key)
                {
                    items.swap(0, index);
                }
                Ok((items, errors))
            },
            Message::Scanned,
        );
        (
            app,
            Task::batch([scan, work(daemon::monitors, Message::Monitors)]),
        )
    }
    fn item(&self) -> Option<&Item> {
        self.items
            .iter()
            .find(|i| Some(&i.record.key) == self.selected.as_ref())
    }
    fn select(
        &mut self,
        key: String,
        overrides: BTreeMap<String, ParameterValue>,
    ) -> Task<Message> {
        self.busy = true;
        let library = self.library.clone();
        work(
            move || {
                let wallpaper = Wallpaper::from_package(&library.load(&key)?)?;
                let mut parameters: BTreeMap<_, _> = wallpaper
                    .parameters()
                    .iter()
                    .map(|p| (p.id.clone(), p.default.clone()))
                    .collect();
                for (id, value) in overrides {
                    wallpaper
                        .parameters()
                        .iter()
                        .find(|p| p.id == id)
                        .ok_or_else(|| anyhow::anyhow!("unknown saved parameter: {id}"))?
                        .validate_value(&value)?;
                    parameters.insert(id, value);
                }
                Ok((key, Arc::new(wallpaper), parameters))
            },
            Message::Loaded,
        )
    }
    fn export_to(&mut self, path: PathBuf) -> Task<Message> {
        let Some(key) = self.selected.clone() else {
            self.status = "Choose a wallpaper to export".into();
            return Task::none();
        };
        self.busy = true;
        let library = self.library.clone();
        work(
            move || library.export(&key, &path).map(|()| true),
            Message::Exported,
        )
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        use audit_ui::CaptureOperation;
        let capture_operation = match &message {
            Message::Scanned(Err(error)) => {
                Some((CaptureOperation::LibraryScan, Some(error.clone())))
            }
            Message::Scanned(Ok((_, errors))) => Some((
                CaptureOperation::LibraryScan,
                (!errors.is_empty()).then(|| errors.join("; ")),
            )),
            Message::Loaded(result) => Some((
                CaptureOperation::LibraryLoad,
                result.as_ref().err().cloned(),
            )),
            Message::Designer(DesignerMessage::Opened(Err(error))) => {
                Some((CaptureOperation::Designer, Some(error.clone())))
            }
            Message::Designer(DesignerMessage::Compiled(result)) => {
                Some((CaptureOperation::Designer, result.as_ref().err().cloned()))
            }
            Message::Graph(graph_ui::GraphMessage::Opened(Err(error))) => {
                Some((CaptureOperation::Graph, Some(error.clone())))
            }
            Message::Graph(graph_ui::GraphMessage::Compiled(revision, result))
                if *revision == self.graph.revision =>
            {
                Some((CaptureOperation::Graph, result.as_ref().err().cloned()))
            }
            _ => None,
        };
        if let Some((operation, error)) = capture_operation {
            self.record_capture_operation(operation, error);
        }
        match message {
            Message::Graph(message) => return self.graph_update(message),
            Message::Designer(message) => return self.designer_update(message),
            Message::Page(page) => {
                self.page = page;
                self.focus_mode = false;
                self.last_tick = Instant::now();
            }
            Message::Focus => self.focus_mode = !self.focus_mode,
            Message::LibraryPanel(panel) => {
                self.library_panel = (self.library_panel != Some(panel)).then_some(panel);
            }
            Message::Dismiss => {
                if self.focus_mode {
                    self.focus_mode = false;
                } else if self.page == Page::Designer && self.designer.paint_expanded {
                    self.designer.paint_expanded = false;
                } else if self.page == Page::Graph && self.graph.wire.is_some() {
                    self.graph.wire = None;
                } else {
                    self.designer.panel = None;
                    self.graph.panel = None;
                }
            }
            Message::Search(value) => self.search = value,
            Message::Scanned(result) => {
                self.busy = false;
                match result {
                    Ok((items, errors)) => {
                        for item in &items {
                            self.cache_thumbnail(item);
                        }
                        self.items = items;
                        self.status = if errors.is_empty() {
                            format!("{} wallpapers in your library", self.items.len())
                        } else {
                            format!("Some wallpapers could not be loaded: {}", errors.join("; "))
                        };
                        if let Some(path) = self.pending_graph.take() {
                            self.page = Page::Graph;
                            return self.graph_update(graph_ui::GraphMessage::Open(path));
                        }
                        if let Some(path) = self.pending_design.take() {
                            return self.designer_update(DesignerMessage::Open(path));
                        }
                        if let Some(item) = self.items.first() {
                            return self.select(item.record.key.clone(), BTreeMap::new());
                        }
                    }
                    Err(error) => self.status = error,
                }
            }
            Message::Select(key) if !self.busy => return self.select(key, BTreeMap::new()),
            Message::Loaded(result) => {
                self.busy = false;
                match result {
                    Ok((key, wallpaper, parameters)) => {
                        self.generation += 1;
                        self.parameter_text = parameters
                            .iter()
                            .map(|(id, value)| (id.clone(), value_text(value)))
                            .collect();
                        self.parameter_errors.clear();
                        self.selected = Some(key);
                        self.preview = Some(Preview {
                            generation: self.generation,
                            wallpaper,
                            parameters: Arc::new(parameters),
                            globals: Globals::new(640, 360),
                            status: Arc::new(Mutex::new(Status::default())),
                        });
                        self.last_tick = Instant::now();
                    }
                    Err(error) => self.status = format!("Could not select wallpaper: {error}"),
                }
            }
            Message::Browse(project) if !self.busy => {
                self.busy = true;
                let library = self.library.clone();
                return Task::perform(
                    async move {
                        let dialog = rfd::AsyncFileDialog::new().set_title(if project {
                            "import wisp project"
                        } else {
                            "import wallpaper"
                        });
                        let chosen = if project {
                            dialog.pick_folder().await
                        } else {
                            dialog
                                .add_filter("wisp wallpaper", &["wisp"])
                                .pick_file()
                                .await
                        };
                        let Some(file) = chosen else {
                            return Ok(None);
                        };
                        let path = file.path().to_owned();
                        tokio::task::spawn_blocking(move || library.import(&path))
                            .await
                            .map_err(|e| e.to_string())?
                            .map(|item| Some(Box::new(item)))
                            .map_err(|e| format!("{e:#}"))
                    },
                    Message::Imported,
                );
            }
            Message::ImportPath(value) => self.import_path = value,
            Message::Import if !self.busy && !self.import_path.trim().is_empty() => {
                let path = PathBuf::from(self.import_path.trim());
                if is_image(&path) {
                    return self.designer_update(DesignerMessage::Open(path));
                }
                self.busy = true;
                let library = self.library.clone();
                return work(
                    move || library.import(&path).map(|item| Some(Box::new(item))),
                    Message::Imported,
                );
            }
            Message::Imported(result) => {
                self.busy = false;
                match result {
                    Ok(Some(item)) => {
                        let key = item.record.key.clone();
                        self.cache_thumbnail(&item);
                        self.status = format!("Imported {}", item.manifest.name);
                        if !self.items.iter().any(|i| i.record.key == key) {
                            self.items.push(*item);
                            self.items.sort_by_key(|i| i.manifest.name.to_lowercase());
                        }
                        self.page = Page::Library;
                        return self.select(key, BTreeMap::new());
                    }
                    Ok(None) => {}
                    Err(error) => self.status = format!("Import failed: {error}"),
                }
            }
            Message::Export if !self.busy && self.selected.is_some() => {
                self.busy = true;
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("export wallpaper")
                            .add_filter("wisp wallpaper", &["wisp"])
                            .set_file_name("wallpaper.wisp")
                            .save_file()
                            .await
                            .map(|file| file.path().to_owned())
                    },
                    Message::ExportChosen,
                );
            }
            Message::ExportChosen(path) => {
                self.busy = false;
                if let Some(path) = path {
                    return self.export_to(path);
                }
                self.status = "Export cancelled".into();
            }
            Message::ExportTo(path) if !self.busy => return self.export_to(path),
            Message::Exported(result) => {
                self.busy = false;
                self.status = match result {
                    Ok(true) => "Wallpaper exported".into(),
                    Ok(false) => "Export cancelled".into(),
                    Err(error) => error,
                };
            }
            Message::RefreshMonitors if !self.discovering => {
                self.discovering = true;
                return work(daemon::monitors, Message::Monitors);
            }
            Message::Monitors(result) => {
                self.discovering = false;
                match result {
                    Ok(monitors) => {
                        self.monitor_error = None;
                        self.monitors.truncate(1);
                        for m in monitors {
                            self.monitors.push(MonitorChoice {
                                id: Some(m.id),
                                label: format!("{} · {} × {}", m.name, m.width, m.height),
                            });
                        }
                        if !self.monitors.iter().any(|m| m.id == self.monitor.id) {
                            self.monitors.push(self.monitor.clone());
                        }
                    }
                    Err(error) => {
                        self.monitor_error = Some(error.clone());
                        self.status = error;
                    }
                }
            }
            Message::Monitor(monitor) if !self.busy => {
                self.monitor = monitor;
                let assignment = self
                    .monitor
                    .id
                    .as_ref()
                    .and_then(|id| self.config.assignment(id))
                    .or_else(|| {
                        if self.monitor.id.is_none() {
                            self.config.default.as_ref()
                        } else {
                            None
                        }
                    });
                if let Some(assignment) = assignment {
                    self.assignment_fps = assignment.fps;
                    if let Some(item) = self.items.iter().find(|i| {
                        self.library.package_path(&i.record.key).ok().as_ref()
                            == Some(&assignment.package)
                    }) {
                        return self.select(item.record.key.clone(), assignment.parameters.clone());
                    }
                }
            }
            Message::Parameter(id, value) => self.set_parameter_text(id, value),
            Message::Boolean(id, value) => self.set_parameter_text(id, value.to_string()),
            Message::Float(id, value) => self.set_parameter_text(id, value.to_string()),
            Message::Color(id, component, value) => {
                if let Some(preview) = &self.preview
                    && let Some(ParameterValue::Color(color)) = preview.parameters.get(&id)
                    && component < 4
                {
                    let mut color = *color;
                    color[component] = value;
                    self.set_parameter_text(id, value_text(&ParameterValue::Color(color)));
                }
            }
            Message::Reset => {
                if let Some(preview) = &mut self.preview {
                    let defaults: BTreeMap<_, _> = preview
                        .wallpaper
                        .parameters()
                        .iter()
                        .map(|p| (p.id.clone(), p.default.clone()))
                        .collect();
                    self.parameter_text = defaults
                        .iter()
                        .map(|(id, value)| (id.clone(), value_text(value)))
                        .collect();
                    preview.parameters = Arc::new(defaults);
                    self.parameter_errors.clear();
                }
            }
            Message::Apply if !self.busy && self.parameter_errors.is_empty() => {
                if let Some(preview) = &self.preview {
                    let library = self.library.clone();
                    let key = self.selected.clone().unwrap();
                    let monitor = self.monitor.id.clone();
                    let parameters = (*preview.parameters).clone();
                    let fps = self.assignment_fps;
                    self.busy = true;
                    return work(
                        move || {
                            library.assign(
                                monitor.as_deref(),
                                Some(Assignment {
                                    package: library.package_path(&key)?,
                                    parameters,
                                    fps,
                                }),
                            )?;
                            daemon::start(&library)
                        },
                        Message::Applied,
                    );
                }
            }
            Message::Clear if !self.busy => {
                let library = self.library.clone();
                let monitor = self.monitor.id.clone();
                self.busy = true;
                return work(
                    move || library.assign(monitor.as_deref(), None),
                    Message::Applied,
                );
            }
            Message::Applied(result) => {
                self.busy = false;
                match self.library.config() {
                    Ok(config) => self.config = config,
                    Err(error) => self.status = format!("{error:#}"),
                }
                self.status = match result {
                    Ok(()) => {
                        "Saved monitor assignments. The running daemon reloads them automatically."
                            .into()
                    }
                    Err(error) => format!("Could not apply wallpaper: {error}"),
                };
            }
            Message::Start if !self.busy => {
                self.busy = true;
                let library = self.library.clone();
                return work(move || daemon::start(&library), Message::Started);
            }
            Message::Started(result) => {
                self.busy = false;
                self.status = match result {
                    Ok(()) => "Wallpaper daemon is running".into(),
                    Err(error) => error,
                };
            }
            Message::DesktopFps(fps) => {
                self.settings.desktop_fps = fps;
                self.assignment_fps = fps;
            }
            Message::PreviewFps(fps) => self.settings.preview_fps = fps,
            Message::Dark(dark) => self.settings.dark_theme = dark,
            Message::AudioEnabled(value) => {
                self.config.audio.enabled = value;
            }
            Message::AudioDevice(value) => {
                self.config.audio.device = value.id;
            }
            Message::RefreshAudio => {
                return work(wisp_audio::devices, Message::AudioDevices);
            }
            Message::AudioDevices(result) => match result {
                Ok(devices) => {
                    self.audio_devices = std::iter::once(AudioChoice {
                        id: None,
                        name: "System output".into(),
                    })
                    .chain(
                        devices
                            .into_iter()
                            .map(|(id, name)| AudioChoice { id: Some(id), name }),
                    )
                    .collect();
                }
                Err(error) => self.status = error,
            },
            Message::AudioGain(value) => {
                self.config.audio.gain = value;
            }
            Message::SaveSettings => {
                self.status = match self
                    .library
                    .save_settings(&self.settings)
                    .and_then(|()| self.library.save_config(&self.config))
                    .and_then(|()| self.audio.configure(self.config.audio.clone()))
                {
                    Ok(()) => "Settings saved".into(),
                    Err(error) => format!("Could not save settings: {error:#}"),
                }
            }
            Message::PausePreview => {
                self.preview_paused = !self.preview_paused;
                self.last_tick = Instant::now();
            }
            Message::Tick(now) => {
                let active = if self.page == Page::Graph {
                    &mut self.graph.preview
                } else if self.page == Page::Designer {
                    &mut self.designer.preview
                } else {
                    &mut self.preview
                };
                if let Some(preview) = active {
                    let delta = now.duration_since(self.last_tick).as_secs_f32().min(0.25);
                    preview.globals.set_audio(self.audio.bands().values());
                    preview.globals.delta_time = delta;
                    preview.globals.time += delta;
                    preview.globals.frame = preview.globals.frame.wrapping_add(1);
                    let status = preview.status.lock().unwrap();
                    if let Some(error) = &status.error {
                        self.status = error.clone();
                    }
                }
                self.last_tick = now;
            }
            Message::AuditPoll => return self.audit_poll(),
            Message::Screenshot(screenshot) => {
                let Some(capture) = self.active_capture.take() else {
                    return Task::none();
                };
                if capture.completion != audit_ui::CaptureCompletion::Checkpoint
                    && let Some(error) = self.capture_error()
                {
                    eprintln!("AUDIT_FAILED: {error}");
                    *self.exit_failure.lock().unwrap() = Some(error);
                }
                let path = capture.path;
                let result = (|| -> Result<()> {
                    use ::image::ImageEncoder;
                    let parent = path
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(std::path::Path::new("."));
                    let mut file = tempfile::NamedTempFile::new_in(parent)?;
                    ::image::codecs::png::PngEncoder::new(&mut file).write_image(
                        &screenshot.rgba,
                        screenshot.size.width,
                        screenshot.size.height,
                        ::image::ExtendedColorType::Rgba8,
                    )?;
                    file.persist_noclobber(&path)?;
                    Ok(())
                })();
                if let Err(error) = result {
                    eprintln!("AUDIT_FAILED: Could not save screenshot: {error:#}");
                    *self.exit_failure.lock().unwrap() =
                        Some(format!("Could not save screenshot: {error:#}"));
                    return iced::exit();
                }
                println!("Saved application screenshot: {}", path.display());
                if capture.completion == audit_ui::CaptureCompletion::Exit
                    || self.exit_failure.lock().unwrap().is_some()
                {
                    return iced::exit();
                }
            }
            Message::Poll if !self.daemon_polling => {
                self.daemon_polling = true;
                let library = self.library.clone();
                return work(move || daemon::running(&library), Message::DaemonStatus);
            }
            Message::DaemonStatus(result) => {
                self.daemon_polling = false;
                match result {
                    Ok(running) => {
                        if self.daemon_running && !running {
                            self.status = format!(
                                "Wallpaper daemon stopped. See {}",
                                self.library.root.join("daemon.log").display()
                            );
                        }
                        self.daemon_running = running;
                    }
                    Err(error) => self.status = error,
                }
            }
            Message::Event(iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key,
                ..
            })) if matches!(self.page, Page::Designer | Page::Graph) => {
                use iced::keyboard::{Key, key::Named};
                match key {
                    Key::Named(Named::F11) => return self.update(Message::Focus),
                    Key::Named(Named::Escape) => return self.update(Message::Dismiss),
                    _ => {}
                }
            }
            Message::Event(iced::Event::Window(iced::window::Event::Focused)) => {
                self.focused = true;
                self.last_tick = Instant::now();
            }
            Message::Event(iced::Event::Window(iced::window::Event::Unfocused)) => {
                self.focused = false
            }
            _ => {}
        }
        Task::none()
    }
    fn cache_thumbnail(&mut self, item: &Item) {
        if let Some((w, h, rgba)) = &item.thumbnail {
            self.thumbnails.insert(
                item.record.key.clone(),
                image::Handle::from_rgba(*w, *h, rgba.clone()),
            );
        }
    }
    fn set_parameter_text(&mut self, id: String, value: String) {
        self.parameter_text.insert(id.clone(), value.clone());
        let Some(preview) = &mut self.preview else {
            return;
        };
        let Some(parameter) = preview.wallpaper.parameters().iter().find(|p| p.id == id) else {
            return;
        };
        match parse_parameter(parameter, &value) {
            Ok(value) => {
                Arc::make_mut(&mut preview.parameters).insert(id.clone(), value);
                self.parameter_errors.remove(&id);
            }
            Err(error) => {
                self.parameter_errors.insert(id, format!("{error:#}"));
            }
        }
    }
    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            iced::time::every(Duration::from_secs(2)).map(|_| Message::Poll),
            iced::event::listen().map(Message::Event),
        ];
        if self.frame_limit.is_some()
            || self.final_screenshot.is_some()
            || self.active_capture.is_some()
            || !self.audit_commands.is_empty()
        {
            subscriptions
                .push(iced::time::every(Duration::from_millis(100)).map(|_| Message::AuditPoll));
        }
        if self.page == Page::Designer && self.designer.paint_pending {
            subscriptions.push(
                iced::time::every(Duration::from_millis(200))
                    .map(|now| Message::Designer(DesignerMessage::PaintTick(now))),
            );
        }
        if ((self.page == Page::Library && self.preview.is_some())
            || (self.page == Page::Designer && self.designer.preview.is_some())
            || (self.page == Page::Graph && self.graph.preview.is_some()))
            && !self.preview_paused
            && (self.focused || self.frame_limit.is_some() || !self.audit_commands.is_empty())
        {
            subscriptions.push(
                iced::time::every(Duration::from_secs_f64(
                    1.0 / f64::from(self.settings.preview_fps),
                ))
                .map(Message::Tick),
            );
        }
        Subscription::batch(subscriptions)
    }
    fn view(&self) -> Element<'_, Message> {
        let nav = |icon, label, page| {
            motion::Button::new(
                row![icons::glyph(icon, 15), text(label).size(14)]
                    .spacing(8)
                    .align_y(iced::Center),
            )
            .padding([8, 12])
            .selected(self.page == page)
            .style(if self.page == page {
                style::selected
            } else {
                style::quiet
            })
            .on_press(Message::Page(page))
        };
        let navigation = row![
            text("wisp").size(22),
            space().width(20),
            nav(Icon::Library, "Library", Page::Library),
            nav(Icon::Create, "Create", Page::Designer),
            nav(Icon::Graph, "Graph", Page::Graph),
            space().width(Fill),
            nav(Icon::Import, "Import", Page::Import),
            nav(Icon::Settings, "Settings", Page::Settings)
        ]
        .spacing(4)
        .align_y(iced::Center)
        .padding([12, 20]);
        let content = match self.page {
            Page::Library => self.library_view(),
            Page::Designer => self.designer_view(),
            Page::Graph => self.graph_view(),
            Page::Import => self.import_view(),
            Page::Settings => self.settings_view(),
        };
        let editing = matches!(self.page, Page::Designer | Page::Graph);
        let mut layout = column![].spacing(0);
        if !self.focus_mode || !editing {
            layout = layout
                .push(navigation)
                .push(iced::widget::rule::horizontal(1).style(style::divider));
        }
        layout = layout.push(
            container(motion::entrance(
                content,
                self.page as u64,
                iced::Vector::new(0., 12.),
            ))
            .padding(if editing { 8 } else { 24 })
            .height(Fill),
        );
        // Keep operational feedback available without turning it into another panel.
        if !self.focus_mode || !editing {
            layout = layout.push(
                container(
                    text(if self.busy {
                        "Working…"
                    } else {
                        &self.status
                    })
                    .size(12)
                    .style(|theme| iced_text::Style {
                        color: Some(style::muted(theme)),
                    }),
                )
                .padding([8, 20]),
            );
        }
        container(layout).width(Fill).height(Fill).into()
    }
    fn playback_button(&self) -> Element<'_, Message> {
        icon_button(
            if self.preview_paused {
                Icon::Play
            } else {
                Icon::Pause
            },
            if self.preview_paused {
                "Play preview"
            } else {
                "Pause preview"
            },
            Some(Message::PausePreview),
            false,
        )
    }
    fn wallpaper_controls(&self) -> Element<'_, Message> {
        let mut detail = column![].spacing(14);
        if let (Some(item), Some(preview)) = (self.item(), &self.preview) {
            if !item.parameters.is_empty() {
                detail = detail.push(
                    row![
                        text("Wallpaper controls").size(18),
                        space().width(Fill),
                        icon_button(
                            Icon::Reset,
                            "Reset wallpaper controls",
                            Some(Message::Reset),
                            false
                        )
                    ]
                    .spacing(10),
                );
                for parameter in &item.parameters {
                    let id = parameter.id.clone();
                    let value = preview.parameters.get(&id).unwrap_or(&parameter.default);
                    let label = id.replace('_', " ").to_lowercase();
                    if let ParameterValue::Boolean(value) = value {
                        detail = detail.push(
                            checkbox(*value)
                                .label(label)
                                .on_toggle(move |v| Message::Boolean(id.clone(), v)),
                        );
                    } else if let ParameterValue::Color(color) = value {
                        detail = detail.push(text(label));
                        let mut components = column![].spacing(10);
                        for (index, label) in
                            ["Red", "Green", "Blue", "Alpha"].into_iter().enumerate()
                        {
                            let id = id.clone();
                            components = components.push(
                                column![
                                    text(format!("{label} {:.2}", color[index])).size(12),
                                    slider(0.0..=1.0, color[index], move |value| {
                                        Message::Color(id.clone(), index, value)
                                    })
                                    .step(0.01)
                                ]
                                .spacing(6)
                                .width(Fill),
                            );
                        }
                        detail = detail.push(components);
                    } else {
                        let input = text_input(
                            "Value",
                            self.parameter_text
                                .get(&id)
                                .map(String::as_str)
                                .unwrap_or(""),
                        )
                        .on_input({
                            let id = id.clone();
                            move |v| Message::Parameter(id.clone(), v)
                        })
                        .width(90);
                        let mut control =
                            column![text(label).size(12), input.width(Fill)].spacing(10);
                        if let (ParameterValue::Float(value), Some(min), Some(max)) =
                            (value, parameter.min, parameter.max)
                            && min < max
                            && (max - min).is_finite()
                            && (max - min) < f64::from(f32::MAX)
                        {
                            control = control.push(
                                slider(min as f32..=max as f32, *value, move |v| {
                                    Message::Float(id.clone(), v)
                                })
                                .step(((max - min) / 200.0) as f32)
                                .width(Fill),
                            );
                        }
                        detail = detail.push(control);
                    }
                    if let Some(error) = self.parameter_errors.get(&parameter.id) {
                        detail = detail.push(text(error).size(12).style(style::error));
                    }
                }
            }
            if item.parameters.is_empty() {
                detail = detail.push(text("This wallpaper has no adjustable controls.").size(13));
            }
        }
        detail.into()
    }
    fn library_view(&self) -> Element<'_, Message> {
        if self.items.is_empty() {
            return container(
                column![
                    text("Your own little atmosphere.").size(32),
                    text("Create a living wallpaper or bring one into your library.").size(15),
                    space().height(12),
                    row![
                        button("create wallpaper")
                            .style(style::primary)
                            .on_press(Message::Page(Page::Designer)),
                        button("import…").on_press(Message::Page(Page::Import)),
                    ]
                    .spacing(12),
                ]
                .spacing(14)
                .align_x(iced::Center),
            )
            .center(Fill)
            .into();
        }
        let query = self.search.to_lowercase();
        let filtered: Vec<_> = self
            .items
            .iter()
            .filter(|i| {
                format!(
                    "{} {} {}",
                    i.manifest.name,
                    i.manifest.author,
                    i.manifest.tags.join(" ")
                )
                .to_lowercase()
                .contains(&query)
            })
            .collect();
        let mut cards = column![].spacing(12);
        if filtered.is_empty() {
            cards = cards.push(text("No matching wallpapers.").size(13));
        }
        for item in filtered {
            let mut card = column![].spacing(6);
            if let Some(handle) = self.thumbnails.get(&item.record.key) {
                card = card.push(
                    image(handle.clone())
                        .width(Fill)
                        .height(124)
                        .content_fit(iced::ContentFit::Cover),
                );
            }
            card = card.push(text(&item.manifest.name).size(15)).push(
                text(&item.manifest.author)
                    .size(12)
                    .style(|t| iced_text::Style {
                        color: Some(style::muted(t)),
                    }),
            );
            cards = cards.push(
                button(card)
                    .selected(self.selected.as_ref() == Some(&item.record.key))
                    .width(Fill)
                    .padding(10)
                    .style(if self.selected.as_ref() == Some(&item.record.key) {
                        style::selected
                    } else {
                        style::quiet
                    })
                    .on_press_maybe((!self.busy).then(|| Message::Select(item.record.key.clone()))),
            );
        }
        let sidebar = column![
            text_input("Find a wallpaper…", &self.search)
                .on_input(Message::Search)
                .size(13),
            scrollable(cards).height(Fill),
        ]
        .spacing(14)
        .width(220);
        let mut detail = column![].spacing(10).height(Fill);
        if let (Some(item), Some(preview)) = (self.item(), &self.preview) {
            let tool = |icon, label, panel| {
                icon_button(
                    icon,
                    label,
                    Some(Message::LibraryPanel(panel)),
                    self.library_panel == Some(panel),
                )
            };
            detail = detail.push(
                row![
                    text(&item.manifest.name).size(18).width(Fill),
                    tool(Icon::Adjust, "Adjust", LibraryPanel::Controls),
                    tool(Icon::Display, "Display", LibraryPanel::Display),
                    tool(Icon::Info, "Info", LibraryPanel::Details),
                    self.playback_button(),
                ]
                .spacing(4)
                .align_y(iced::Center),
            );
            let stage = container(preview.canvas())
                .padding(16)
                .width(Fill)
                .height(Fill)
                .style(style::stage);
            let mut workspace = row![stage].spacing(8).height(Fill);
            if let Some(panel) = self.library_panel {
                let title = match panel {
                    LibraryPanel::Controls => "Adjust",
                    LibraryPanel::Display => "Display",
                    LibraryPanel::Details => "About this wallpaper",
                };
                let content: Element<'_, Message> = match panel {
                    LibraryPanel::Controls => self.wallpaper_controls(),
                    LibraryPanel::Display => {
                        let mut content = column![
                            text("Display on").size(12),
                            pick_list(
                                self.monitors.as_slice(),
                                Some(&self.monitor),
                                Message::Monitor
                            )
                            .width(Fill),
                            button("refresh displays").on_press_maybe(
                                (!self.discovering).then_some(Message::RefreshMonitors)
                            ),
                            text("Frame limit").size(12),
                            pick_list(
                                &[15, 24, 30, 60, 90, 120, 144, 240][..],
                                Some(self.assignment_fps),
                                Message::DesktopFps
                            ),
                            text("Frames per second").size(12),
                            text(self.assignment_label()).size(12),
                            button("clear assignment")
                                .on_press_maybe((!self.busy).then_some(Message::Clear)),
                        ]
                        .spacing(14);
                        if let Some(error) = &self.monitor_error {
                            content = content.push(text(error).size(12));
                        }
                        content.into()
                    }
                    LibraryPanel::Details => column![
                        text(&item.manifest.description).size(14),
                        text(format!("By {}", item.manifest.author)).size(12),
                        text(item.manifest.tags.join(" · ")).size(12),
                        text(format!(
                            "{} · {} passes · {:.1} MiB",
                            item.record.source,
                            item.passes,
                            item.resource_bytes as f64 / 1048576.
                        ))
                        .size(12),
                        button("export wallpaper…")
                            .on_press_maybe((!self.busy).then_some(Message::Export)),
                    ]
                    .spacing(14)
                    .into(),
                };
                workspace = workspace.push(motion::entrance(
                    container(
                        column![
                            row![
                                text(title).size(16),
                                space().width(Fill),
                                icon_button(
                                    Icon::Close,
                                    "Close panel",
                                    Some(Message::LibraryPanel(panel)),
                                    false
                                )
                            ]
                            .align_y(iced::Center),
                            scrollable(content).height(Fill),
                        ]
                        .spacing(14),
                    )
                    .padding(14)
                    .width(280)
                    .height(Fill)
                    .style(style::panel),
                    panel as u64,
                    iced::Vector::new(24., 0.),
                ));
            }
            detail = detail.push(workspace).push(
                row![
                    text(format!("For {}", self.monitor.label))
                        .size(12)
                        .width(Fill),
                    button("apply wallpaper")
                        .style(style::primary)
                        .on_press_maybe(
                            (!self.busy
                                && self.parameter_errors.is_empty()
                                && preview.status.lock().unwrap().error.is_none())
                            .then_some(Message::Apply)
                        ),
                ]
                .align_y(iced::Center)
                .spacing(12),
            );
        } else {
            detail = detail.push(container(text("Choose a wallpaper.").size(24)).center(Fill));
        }
        row![sidebar, detail.width(Fill)]
            .spacing(24)
            .height(Fill)
            .into()
    }
    fn assignment_label(&self) -> String {
        let assignment = match &self.monitor.id {
            Some(id) => self.config.assignment(id),
            None => self.config.default.as_ref(),
        };
        let name = assignment
            .map(|a| {
                self.items
                    .iter()
                    .find(|i| {
                        self.library.package_path(&i.record.key).ok().as_ref() == Some(&a.package)
                    })
                    .map(|i| i.manifest.name.clone())
                    .unwrap_or_else(|| a.package.display().to_string())
            })
            .unwrap_or_else(|| "None".into());
        format!(
            "Saved: {name} · Daemon {}",
            if self.daemon_running {
                "running"
            } else {
                "stopped"
            }
        )
    }
    fn import_view(&self) -> Element<'_, Message> {
        container(
            column![
                text("Make room for something new").size(30),
                text(
                    "Import a portable .wisp wallpaper or compile a Wisp project into your library."
                ),
                row![
                    button("choose .wisp file")
                        .on_press_maybe((!self.busy).then_some(Message::Browse(false))),
                    button("choose project folder")
                        .on_press_maybe((!self.busy).then_some(Message::Browse(true)))
                ]
                .spacing(12),
                button("create from image").on_press(Message::Page(Page::Designer)),
                text("Or enter a file or project folder path"),
                text_input("/path/to/wallpaper.wisp", &self.import_path)
                    .on_input(Message::ImportPath)
                    .on_submit(Message::Import),
                button("import to library").on_press_maybe(
                    (!self.busy && !self.import_path.trim().is_empty()).then_some(Message::Import)
                ),
                text(
                    "Imports are validated before installation. Your original files stay in place."
                )
                .size(14)
            ]
            .spacing(20)
            .max_width(760),
        )
        .height(Fill)
        .into()
    }
    fn settings_view(&self) -> Element<'_, Message> {
        let status = self.audio.status();
        let bands = status.bands;
        let selected = self
            .audio_devices
            .iter()
            .find(|d| d.id == self.config.audio.device)
            .cloned()
            .unwrap_or_else(|| AudioChoice {
                id: self.config.audio.device.clone(),
                name: "Saved audio device".into(),
            });
        let content = column![
            text("Settings").size(30),
            text("Audio response").size(22),
            checkbox(self.config.audio.enabled)
                .label("enable audio capture for wallpapers and preview")
                .on_toggle(Message::AudioEnabled),
            text("Captures system output by default. An explicit input device can capture a microphone.").size(13),
            row![
                pick_list(self.audio_devices.clone(), Some(selected), Message::AudioDevice),
                button("refresh devices").on_press(Message::RefreshAudio)
            ].spacing(12),
            row![
                text("Sensitivity"),
                slider(0.0..=10.0, self.config.audio.gain, Message::AudioGain).step(0.1),
                text(format!("{:.1}", self.config.audio.gain))
            ].spacing(12),
            text(status.message).size(13),
            text(format!("Bass {:.0}% · Mid {:.0}% · Treble {:.0}% · Volume {:.0}%",
                bands.bass * 100., bands.mid * 100., bands.treble * 100., bands.volume * 100.)).size(13),
            text("Save settings to apply the source and sensitivity.").size(13),
            text("Rendering").size(22),
            row![text("Default desktop FPS").width(240),
                pick_list(&[15,24,30,60,90,120,144,240][..], Some(self.settings.desktop_fps), Message::DesktopFps)
            ].spacing(12),
            text("Used for new assignments. Apply a wallpaper to change a monitor's frame limit.").size(13),
            row![text("Live preview FPS").width(240),
                pick_list(&[15,24,30,60][..], Some(self.settings.preview_fps), Message::PreviewFps)
            ].spacing(12),
            text("Live preview pauses when this window loses focus or you leave a preview page.").size(13),
            checkbox(self.settings.dark_theme).label("dark appearance").on_toggle(Message::Dark),
            button("save settings").on_press(Message::SaveSettings),
            text("Desktop wallpapers").size(22),
            text(if self.daemon_running {"The wallpaper daemon is running independently."} else {"The wallpaper daemon is stopped."}),
            button("start wallpaper daemon").on_press_maybe((!self.busy).then_some(Message::Start)),
            text("Closing Wisp keeps your desktop wallpapers running. Use Clear in the Library to remove assignments."),
            text(format!("Library: {}", self.library.root.display())).size(13),
        ].spacing(18).max_width(850);
        container(scrollable(content)).height(Fill).into()
    }
}
fn value_text(value: &ParameterValue) -> String {
    match value {
        ParameterValue::Float(v) => v.to_string(),
        ParameterValue::Integer(v) => v.to_string(),
        ParameterValue::Boolean(v) => v.to_string(),
        ParameterValue::Color(v) | ParameterValue::Vector4(v) => serde_json::to_string(v).unwrap(),
        ParameterValue::Vector2(v) => serde_json::to_string(v).unwrap(),
        ParameterValue::Vector3(v) => serde_json::to_string(v).unwrap(),
    }
}
pub(crate) fn parse_parameter(parameter: &Parameter, text: &str) -> Result<ParameterValue> {
    let value = match parameter.default {
        ParameterValue::Float(_) => ParameterValue::Float(serde_json::from_str(text)?),
        ParameterValue::Integer(_) => ParameterValue::Integer(serde_json::from_str(text)?),
        ParameterValue::Boolean(_) => ParameterValue::Boolean(serde_json::from_str(text)?),
        ParameterValue::Color(_) => ParameterValue::Color(serde_json::from_str(text)?),
        ParameterValue::Vector2(_) => ParameterValue::Vector2(serde_json::from_str(text)?),
        ParameterValue::Vector3(_) => ParameterValue::Vector3(serde_json::from_str(text)?),
        ParameterValue::Vector4(_) => ParameterValue::Vector4(serde_json::from_str(text)?),
    };
    parameter.validate_value(&value)?;
    Ok(value)
}

fn is_image(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["png", "jpg", "jpeg", "webp"].contains(&e.to_ascii_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lowercase_choices_preserve_case_sensitive_values_and_identity() {
        let first = LowercaseOption(MonitorChoice {
            id: Some("DP-MixedCase".into()),
            label: "Studio Display".into(),
        });
        let second = LowercaseOption(MonitorChoice {
            id: Some("dp-mixedcase".into()),
            label: "studio display".into(),
        });
        assert_eq!(first.to_string(), "studio display");
        assert_eq!(first.to_string(), second.to_string());
        assert!(first != second);
        assert_eq!(first.0.id.as_deref(), Some("DP-MixedCase"));
        assert_eq!(first.0.label, "Studio Display");
    }

    #[test]
    fn escape_restores_focused_painting_before_closing_the_editor() {
        let temporary = tempfile::tempdir().unwrap();
        let library = Library::open(Some(temporary.path().to_owned())).unwrap();
        let (mut app, _) = App::new(library, Settings::default(), None, None);
        app.page = Page::Designer;
        app.designer.panel = Some(designer_ui::DesignerPanel::Layers);
        app.designer.paint_expanded = true;
        app.designer.paint_pending = true;
        app.designer.dirty = true;
        let _ = app.update(Message::Focus);
        assert!(app.focus_mode);
        let _ = app.update(Message::Dismiss);
        assert!(!app.focus_mode);
        assert!(app.designer.paint_expanded);
        assert!(app.designer.panel == Some(designer_ui::DesignerPanel::Layers));
        let _ = app.update(Message::Dismiss);
        assert!(!app.designer.paint_expanded);
        assert!(app.designer.panel.is_some());
        let _ = app.update(Message::Dismiss);
        assert!(app.designer.panel.is_none());
        assert!(app.designer.dirty && app.designer.paint_pending);
    }

    #[test]
    fn panels_toggle_during_compilation_without_changing_the_graph() {
        let temporary = tempfile::tempdir().unwrap();
        let library = Library::open(Some(temporary.path().to_owned())).unwrap();
        let (mut app, _) = App::new(library, Settings::default(), None, None);
        app.page = Page::Graph;
        app.graph.dirty = true;
        app.graph.wire = Some("source".into());
        let panel = graph_ui::GraphPanel::Nodes;
        let _ = app.update(Message::Graph(graph_ui::GraphMessage::Panel(panel)));
        assert!(app.graph.panel == Some(panel));
        let _ = app.update(Message::Dismiss);
        assert!(app.graph.wire.is_none());
        assert!(app.graph.panel == Some(panel));
        let _ = app.update(Message::Graph(graph_ui::GraphMessage::Panel(panel)));
        assert!(app.graph.panel.is_none());
        assert!(app.graph.dirty && app.busy);
        assert_eq!(app.graph.revision, 0);
    }

    #[test]
    fn invalid_edits_preserve_preview_values_and_reset_uses_package_defaults() {
        let temporary = tempfile::tempdir().unwrap();
        let library = Library::open(Some(temporary.path().to_owned())).unwrap();
        let item = library
            .import(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../examples/graph-fused"),
            )
            .unwrap();
        let wallpaper =
            Arc::new(Wallpaper::from_package(&library.load(&item.record.key).unwrap()).unwrap());
        let mut params: BTreeMap<_, _> = wallpaper
            .parameters()
            .iter()
            .map(|p| (p.id.clone(), p.default.clone()))
            .collect();
        let original = params["brightness"].clone();
        params.insert("brightness".into(), ParameterValue::Float(1.3));
        let (mut app, _) = App::new(library, Settings::default(), None, None);
        app.items.push(item.clone());
        let _ = app.update(Message::Loaded(Ok((
            item.record.key.clone(),
            wallpaper,
            params,
        ))));
        let _ = app.update(Message::Parameter("brightness".into(), "-100".into()));
        assert_eq!(
            app.preview.as_ref().unwrap().parameters["brightness"],
            ParameterValue::Float(1.3)
        );
        assert!(!app.parameter_errors.is_empty());
        let _ = app.update(Message::Parameter("brightness".into(), "0.5".into()));
        assert!(app.parameter_errors.is_empty());
        let _ = app.update(Message::Loaded(Err("bad replacement".into())));
        assert_eq!(app.selected.as_deref(), Some(item.record.key.as_str()));
        let _ = app.update(Message::Reset);
        assert_eq!(
            app.preview.as_ref().unwrap().parameters["brightness"],
            original
        );
    }
    #[test]
    fn parameter_inputs_keep_types_bounds_and_finite_components() {
        for value in [
            ParameterValue::Float(0.5),
            ParameterValue::Integer(-2),
            ParameterValue::Boolean(true),
            ParameterValue::Color([1.0; 4]),
            ParameterValue::Vector2([0.0, 1.0]),
            ParameterValue::Vector3([1.0, 2.0, 3.0]),
            ParameterValue::Vector4([1.0, 2.0, 3.0, 4.0]),
        ] {
            let parameter = Parameter {
                id: "test".into(),
                default: value.clone(),
                min: None,
                max: None,
            };
            assert_eq!(
                parse_parameter(&parameter, &value_text(&value)).unwrap(),
                value
            );
            assert!(parse_parameter(&parameter, "null").is_err());
            assert!(parse_parameter(&parameter, "NaN").is_err());
        }
        let color = Parameter {
            id: "color".into(),
            default: ParameterValue::Color([1.0; 4]),
            min: None,
            max: None,
        };
        assert!(parse_parameter(&color, "[2,0,0,1]").is_err());
    }
}
