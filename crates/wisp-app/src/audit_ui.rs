use super::*;
use designer_ui::{
    DesignEffect,
    layer_ui::{LayerMessage, MaskOption},
};
use graph_ui::GraphMessage;
use serde::Deserialize;
use std::{collections::VecDeque, path::Path};

#[derive(Clone, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Command {
    Page {
        value: String,
    },
    Wait {
        milliseconds: u64,
    },
    WaitFor {
        state: serde_json::Map<String, serde_json::Value>,
        #[serde(default = "default_state_wait_timeout_ms")]
        timeout_ms: u64,
    },
    Screenshot {
        path: PathBuf,
    },
    Assert {
        state: serde_json::Value,
    },
    Finish {},
    PausePreview {},
    Focus {},
    Dismiss {},
    DesignerOpen {
        path: PathBuf,
    },
    DesignerName {
        value: String,
    },
    DesignerAuthor {
        value: String,
    },
    DesignerEffect {
        effect: String,
        enabled: bool,
    },
    DesignerValue {
        id: String,
        value: f32,
    },
    DesignerPanel {
        value: String,
    },
    DesignerSave {
        path: PathBuf,
    },
    DesignerExport {
        path: PathBuf,
    },
    LayerEnable {},
    LayerSelect {
        index: usize,
    },
    LayerImport {
        path: PathBuf,
        kind: String,
        #[serde(default)]
        index: usize,
    },
    LayerValue {
        id: String,
        value: f32,
    },
    LayerMask {
        white: bool,
    },
    LayerMaskFlag {
        flag: String,
        value: bool,
    },
    LayerPaint {
        from: [f32; 2],
        to: [f32; 2],
    },
    LayerStrokeEnd {},
    LayerErase {
        value: bool,
    },
    LayerExpand {
        value: bool,
    },
    GraphNew {},
    GraphPanel {
        value: String,
    },
    GraphOpen {
        path: PathBuf,
    },
    GraphSave {
        path: PathBuf,
    },
    GraphExport {
        path: PathBuf,
    },
    GraphImage {
        path: PathBuf,
    },
    GraphSelect {
        id: String,
    },
    GraphAdd {
        id: String,
    },
    GraphField {
        id: String,
        value: String,
    },
    GraphFieldEdit {
        id: String,
        value: String,
    },
    GraphWire {
        id: String,
    },
    GraphSocket {
        id: String,
        input: String,
    },
    GraphDisconnect {
        id: String,
        input: String,
    },
    GraphMove {
        id: String,
        position: [f32; 2],
    },
    GraphDelete {},
    GraphOutput {},
    GraphArrange {},
    LibraryParameter {
        id: String,
        value: String,
    },
    LibraryFloat {
        id: String,
        value: f32,
    },
    LibraryBoolean {
        id: String,
        value: bool,
    },
    LibraryReset {},
    LibraryExport {
        path: PathBuf,
    },
}

const MAX_AUDIT_BYTES: u64 = 1024 * 1024;
const MAX_AUDIT_COMMANDS: usize = 1024;

fn default_state_wait_timeout_ms() -> u64 {
    10_000
}

fn validate_state_expectations(
    expected: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    for (key, value) in expected {
        let valid = match key.as_str() {
            "page" => value.as_str().is_some_and(|page| {
                matches!(
                    page,
                    "library" | "designer" | "graph" | "import" | "settings"
                )
            }),
            "busy" | "paused" | "focus" | "designer_dirty" | "designer_valid" | "graph_dirty"
            | "graph_valid" | "preview_error" | "preview_ready" | "paint_dirty" => {
                value.is_boolean()
            }
            "parameter_errors" => value.as_u64().is_some(),
            "graph_nodes" => value.is_null() || value.as_u64().is_some(),
            "designer_name" => value.is_null() || value.is_string(),
            "status" | "graph_diagnostic" | "status_contains" => value.is_string(),
            _ => anyhow::bail!("unknown state assertion key: {key}"),
        };
        ensure!(valid, "invalid state expectation for {key}: {value}");
    }
    Ok(())
}

fn assert_state_matches(
    state: &serde_json::Value,
    expected: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    for (key, value) in expected {
        if key == "status_contains" {
            let status = state["status"].as_str().unwrap_or_default();
            ensure!(
                status.contains(value.as_str().context("status_contains must be text")?),
                "status assertion failed: {status}"
            );
        } else {
            ensure!(
                state.get(key).is_some() && state.get(key) == Some(value),
                "state assertion {key}: expected {value}, got {}",
                state.get(key).unwrap_or(&serde_json::Value::Null)
            );
        }
    }
    Ok(())
}

pub(super) fn load(path: Option<&Path>) -> Result<VecDeque<Command>> {
    let Some(path) = path else {
        return Ok(VecDeque::new());
    };
    let commands: VecDeque<Command> =
        serde_json::from_slice(&wisp_io::read_bounded(path, MAX_AUDIT_BYTES)?)
            .context("invalid audit command file")?;
    ensure!(
        !commands.is_empty() && commands.len() <= MAX_AUDIT_COMMANDS,
        "audit requires 1–1024 commands"
    );
    ensure!(
        matches!(commands.back(), Some(Command::Finish {}))
            && commands
                .iter()
                .filter(|command| matches!(command, Command::Finish {}))
                .count()
                == 1,
        "audit sequence must contain exactly one finish at the end"
    );
    let mut total_wait = 0u64;
    for command in &commands {
        match command {
            Command::Wait { milliseconds }
            | Command::WaitFor {
                timeout_ms: milliseconds,
                ..
            } => {
                ensure!(
                    *milliseconds <= 30_000,
                    "audit waits cannot exceed 30000 milliseconds"
                );
                total_wait += milliseconds;
                ensure!(
                    total_wait <= 90_000,
                    "total audit waits cannot exceed 90000 milliseconds"
                );
                if let Command::WaitFor { state, .. } = command {
                    validate_state_expectations(state)?;
                }
            }
            Command::DesignerValue { value, .. }
            | Command::LayerValue { value, .. }
            | Command::LibraryFloat { value, .. } => {
                ensure!(value.is_finite(), "audit numeric values must be finite");
            }
            Command::GraphMove { position, .. } | Command::LayerPaint { from: position, .. } => {
                ensure!(
                    position.iter().all(|value| value.is_finite()),
                    "audit coordinates must be finite"
                );
                if let Command::LayerPaint { to, .. } = command {
                    ensure!(
                        to.iter().all(|value| value.is_finite()),
                        "audit coordinates must be finite"
                    );
                }
            }
            _ => {}
        }
    }
    Ok(commands)
}

impl Command {
    fn message(self) -> Result<Message> {
        use Command::*;
        let designer = |message| Message::Designer(message);
        let layer = |message| designer(DesignerMessage::Layer(message));
        let graph = |message| Message::Graph(message);
        Ok(match self {
            Page { value } => Message::Page(match value.as_str() {
                "library" => super::Page::Library,
                "designer" => super::Page::Designer,
                "graph" => super::Page::Graph,
                "import" => super::Page::Import,
                "settings" => super::Page::Settings,
                _ => anyhow::bail!("unknown page: {value}"),
            }),
            PausePreview {} => Message::PausePreview {},
            Focus {} => Message::Focus {},
            Dismiss {} => Message::Dismiss {},
            DesignerOpen { path } => designer(DesignerMessage::Open(path)),
            DesignerName { value } => designer(DesignerMessage::Name(value)),
            DesignerAuthor { value } => designer(DesignerMessage::Author(value)),
            DesignerEffect { effect, enabled } => designer(DesignerMessage::Toggle(
                match effect.as_str() {
                    "pulse" => DesignEffect::Pulse,
                    "blur" => DesignEffect::Blur,
                    "vignette" => DesignEffect::Vignette,
                    _ => anyhow::bail!("unknown designer effect: {effect}"),
                },
                enabled,
            )),
            DesignerValue { id, value } => designer(DesignerMessage::Value(id, value)),
            DesignerPanel { value } => designer(DesignerMessage::Panel(match value.as_str() {
                "layers" => designer_ui::DesignerPanel::Layers,
                "effects" => designer_ui::DesignerPanel::Effects,
                "project" => designer_ui::DesignerPanel::Project,
                _ => anyhow::bail!("unknown designer panel: {value}"),
            })),
            DesignerSave { path } => designer(DesignerMessage::SaveTo(true, path)),
            DesignerExport { path } => designer(DesignerMessage::SaveTo(false, path)),
            LayerEnable {} => layer(LayerMessage::Enable),
            LayerSelect { index } => layer(LayerMessage::Select(index)),
            LayerImport { path, kind, index } => layer(LayerMessage::ImportPath(
                match kind.as_str() {
                    "layer" => crate::layers::ImportTarget::Layer,
                    "mask" => crate::layers::ImportTarget::Mask(index),
                    "displacement" => crate::layers::ImportTarget::Displacement(index),
                    _ => anyhow::bail!("unknown layer import kind: {kind}"),
                },
                path,
            )),
            LayerValue { id, value } => layer(LayerMessage::Value(id.as_str().try_into()?, value)),
            LayerMask { white } => layer(LayerMessage::Mask(white)),
            LayerMaskFlag { flag, value } => layer(LayerMessage::MaskFlag(
                match flag.as_str() {
                    "invert" => MaskOption::Invert,
                    "composition" => MaskOption::Composition,
                    "effect" => MaskOption::Effect,
                    _ => anyhow::bail!("unknown mask flag: {flag}"),
                },
                value,
            )),
            LayerPaint { from, to } => layer(LayerMessage::Paint(from, to)),
            LayerStrokeEnd {} => layer(LayerMessage::StrokeEnd),
            LayerErase { value } => layer(LayerMessage::Erase(value)),
            LayerExpand { value } => layer(LayerMessage::Expand(value)),
            GraphNew {} => graph(GraphMessage::New),
            GraphPanel { value } => graph(GraphMessage::Panel(match value.as_str() {
                "nodes" => graph_ui::GraphPanel::Nodes,
                "properties" => graph_ui::GraphPanel::Properties,
                "preview" => graph_ui::GraphPanel::Preview,
                "project" => graph_ui::GraphPanel::Project,
                _ => anyhow::bail!("unknown graph panel: {value}"),
            })),
            GraphOpen { path } => graph(GraphMessage::Open(path)),
            GraphSave { path } => graph(GraphMessage::SaveTo(true, path)),
            GraphExport { path } => graph(GraphMessage::SaveTo(false, path)),
            GraphImage { path } => graph(GraphMessage::AssetPath(path)),
            GraphSelect { id } => graph(GraphMessage::Select(id)),
            GraphAdd { id } => graph(GraphMessage::Add(id)),
            GraphField { id, value } => graph(GraphMessage::Field(id, value)),
            GraphFieldEdit { id, value } => graph(GraphMessage::FieldEdit(id, value)),
            GraphWire { id } => graph(GraphMessage::Wire(id)),
            GraphSocket { id, input } => graph(GraphMessage::Socket(id, input)),
            GraphDisconnect { id, input } => graph(GraphMessage::Disconnect(id, input)),
            GraphMove { id, position } => graph(GraphMessage::Move(id, position)),
            GraphDelete {} => graph(GraphMessage::Delete),
            GraphOutput {} => graph(GraphMessage::Output),
            GraphArrange {} => graph(GraphMessage::Arrange),
            LibraryParameter { id, value } => Message::Parameter(id, value),
            LibraryFloat { id, value } => Message::Float(id, value),
            LibraryBoolean { id, value } => Message::Boolean(id, value),
            LibraryReset {} => Message::Reset,
            LibraryExport { path } => Message::ExportTo(path),
            _ => anyhow::bail!("audit flow command cannot be dispatched as an edit"),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum CaptureOperation {
    LibraryScan,
    LibraryLoad,
    Designer,
    Graph,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CaptureCompletion {
    Checkpoint,
    AuditFinish,
    Exit,
}

pub(super) struct Capture {
    pub(super) path: PathBuf,
    pub(super) started: Instant,
    pub(super) completion: CaptureCompletion,
}

impl App {
    fn active_preview(&self) -> Option<&Preview> {
        match self.page {
            Page::Designer => self.designer.preview.as_ref(),
            Page::Graph => self.graph.preview.as_ref(),
            _ => self.preview.as_ref(),
        }
    }
    pub(super) fn record_capture_operation(
        &mut self,
        operation: CaptureOperation,
        error: Option<String>,
    ) {
        if let Some(error) = error {
            self.capture_operation_errors.insert(operation, error);
        } else {
            self.capture_operation_errors.remove(&operation);
        }
    }
    pub(super) fn capture_error(&self) -> Option<String> {
        self.startup_error
            .clone()
            .or_else(|| self.capture_operation_errors.values().next().cloned())
            .or_else(|| {
                self.active_preview()
                    .and_then(|preview| preview.status.lock().unwrap().error.clone())
            })
    }
    fn capture(&mut self, path: PathBuf, completion: CaptureCompletion) -> Task<Message> {
        self.active_capture = Some(Capture {
            path,
            started: Instant::now(),
            completion,
        });
        iced::window::latest()
            .and_then(iced::window::screenshot)
            .map(Message::Screenshot)
    }
    fn fail_audit(&mut self, error: impl Into<String>) -> Task<Message> {
        let error = error.into();
        eprintln!("AUDIT_FAILED: {error}");
        *self.exit_failure.lock().unwrap() = Some(error);
        if self.active_capture.is_none()
            && let Some(path) = self.final_screenshot.take()
        {
            self.capture(path, CaptureCompletion::Exit)
        } else {
            iced::exit()
        }
    }
    fn smoke_ready(&self, frames: u64) -> bool {
        self.active_preview().is_some_and(|preview| {
            let status = preview.status.lock().unwrap();
            status.error.is_none()
                && status.rendered_generation == Some(preview.generation)
                && status.frames >= frames
        })
    }
    fn state(&self) -> serde_json::Value {
        let preview = self
            .active_preview()
            .map(|preview| (preview.generation, preview.status.lock().unwrap()));
        serde_json::json!({
            "page": match self.page { Page::Library => "library", Page::Designer => "designer", Page::Graph => "graph", Page::Import => "import", Page::Settings => "settings" },
            "busy": self.busy, "paused": self.preview_paused, "focus": self.focus_mode,
            "designer_dirty": self.designer.dirty, "designer_valid": self.designer.valid,
            "graph_dirty": self.graph.dirty, "graph_valid": self.graph.valid,
            "parameter_errors": self.parameter_errors.len(), "status": self.status,
            "graph_diagnostic": self.graph.diagnostic,
            "preview_error": preview.as_ref().is_some_and(|(_, status)| status.error.is_some()),
            "preview_ready": preview.as_ref().is_some_and(|(generation, status)| status.rendered_generation == Some(*generation) && status.error.is_none()),
            "designer_name": self.designer.draft.as_ref().map(|draft| &draft.design.name),
            "graph_nodes": self.graph.document.as_ref().map(|doc| doc.graph.nodes.len()),
            "paint_dirty": self.designer.paint_dirty,
        })
    }
    fn assert_state(&self, expected: &serde_json::Value) -> Result<()> {
        assert_state_matches(
            &self.state(),
            expected
                .as_object()
                .context("assert state must be an object")?,
        )
    }
    pub(super) fn audit_poll(&mut self) -> Task<Message> {
        if let Some(capture) = &self.active_capture {
            if capture.started.elapsed() > Duration::from_secs(10) {
                return self.fail_audit("application screenshot timed out");
            }
            return Task::none();
        }
        let timed_out = self.started.elapsed() > Duration::from_secs(45);
        let capture_error = self.capture_error();
        if self.frame_limit.is_some() {
            if let Some(error) = capture_error.clone() {
                return self.fail_audit(error);
            }
            if self
                .frame_limit
                .is_some_and(|frames| self.smoke_ready(frames))
                && self.audit_commands.is_empty()
            {
                println!("Rendered {} live preview frames", self.frame_limit.unwrap());
                self.frame_limit = None;
                return if let Some(path) = self.final_screenshot.take() {
                    self.capture(path, CaptureCompletion::Exit)
                } else {
                    iced::exit()
                };
            }
            if timed_out {
                return self.fail_audit(format!("Wisp smoke test timed out: {}", self.status));
            }
        }
        if !self.audit_commands.is_empty() {
            if self.started.elapsed() > Duration::from_secs(120) {
                return self.fail_audit(format!("audit commands timed out: {}", self.status));
            }
            if self
                .audit_wait_until
                .is_some_and(|until| Instant::now() < until)
            {
                return Task::none();
            }
            if let Some(Command::WaitFor { state, timeout_ms }) = self.audit_commands.front() {
                if let Err(error) = validate_state_expectations(state) {
                    return self.fail_audit(format!("{error:#}"));
                }
                let current_state = self.state();
                let started = self.audit_state_wait_started.get_or_insert_with(|| {
                    println!("AUDIT_STATE {current_state}");
                    Instant::now()
                });
                match assert_state_matches(&current_state, state) {
                    Ok(()) => {
                        self.audit_commands.pop_front();
                        self.audit_state_wait_started = None;
                    }
                    Err(error) if started.elapsed() >= Duration::from_millis(*timeout_ms) => {
                        return self.fail_audit(format!(
                            "wait_for timed out after {timeout_ms} milliseconds: {error:#}; current state: {current_state}"
                        ));
                    }
                    Err(_) => {}
                }
                return Task::none();
            }
            if self.busy {
                return Task::none();
            }
            let command = self.audit_commands.pop_front().unwrap();
            println!("AUDIT_STATE {}", self.state());
            match command {
                Command::Wait { milliseconds } => {
                    self.audit_wait_until =
                        Some(Instant::now() + Duration::from_millis(milliseconds))
                }
                Command::Screenshot { path } => {
                    return self.capture(path, CaptureCompletion::Checkpoint);
                }
                Command::Assert { state } => {
                    if let Err(error) = self.assert_state(&state) {
                        return self.fail_audit(format!("{error:#}"));
                    }
                }
                Command::Finish {} => {
                    if let Some(frames) = self.frame_limit {
                        if !self.smoke_ready(frames) {
                            self.audit_commands.push_front(Command::Finish {});
                            return Task::none();
                        }
                        println!("Rendered {frames} live preview frames");
                        self.frame_limit = None;
                    }
                    if let Some(path) = self.final_screenshot.take() {
                        self.audit_commands.push_front(Command::Finish {});
                        return self.capture(path, CaptureCompletion::AuditFinish);
                    }
                    println!("AUDIT_COMPLETE {}", self.state());
                    return iced::exit();
                }
                command => match command.message() {
                    Ok(message) => return self.update(message),
                    Err(error) => return self.fail_audit(format!("{error:#}")),
                },
            }
            return Task::none();
        }
        if self.frame_limit.is_none()
            && self.final_screenshot.is_some()
            && self.started.elapsed() >= self.screenshot_delay
            && (!self.busy || timed_out)
        {
            if let Some(error) = capture_error {
                *self.exit_failure.lock().unwrap() = Some(error);
            }
            if timed_out && self.busy {
                *self.exit_failure.lock().unwrap() =
                    Some(format!("startup timed out: {}", self.status));
            }
            let path = self.final_screenshot.take().unwrap();
            return self.capture(path, CaptureCompletion::Exit);
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _) = App::new(
            Library::open(Some(dir.path().join("library"))).unwrap(),
            Settings::default(),
            None,
            None,
        );
        app.busy = false;
        (dir, app)
    }
    #[test]
    fn erase_commands_route_to_the_same_paint_controls_as_the_ui() {
        let (_dir, mut app) = app();
        for erase in [true, false] {
            let command: Command = serde_json::from_value(serde_json::json!({
                "command": "layer_erase", "value": erase,
            }))
            .unwrap();
            let _ = app.update(command.message().unwrap());
            assert_eq!(app.designer.erase, erase);
        }
    }
    #[test]
    fn scripts_reject_unbounded_files_excess_commands_and_waits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("commands.json");
        for invalid in ["[]".to_owned(), r#"[{"command":"wait","milliseconds":30001},{"command":"finish"}]"#.to_owned(),
            r#"[{"command":"wait","milliseconds":30000},{"command":"wait","milliseconds":30000},{"command":"wait","milliseconds":30000},{"command":"wait","milliseconds":1},{"command":"finish"}]"#.to_owned(),
            r#"[{"command":"wait","milliseconds":0}]"#.to_owned(),
            format!("[{}]", vec![r#"{"command":"finish"}"#; MAX_AUDIT_COMMANDS+1].join(",")),
            " ".repeat(MAX_AUDIT_BYTES as usize + 1)] {
            std::fs::write(&path, invalid).unwrap();
            assert!(load(Some(&path)).is_err());
        }
        assert!(load(Some(dir.path())).is_err());
        std::fs::write(
            &path,
            r#"[{"command":"wait","milliseconds":0},{"command":"finish"}]"#,
        )
        .unwrap();
        assert_eq!(load(Some(&path)).unwrap().len(), 2);
        assert!(load(None).unwrap().is_empty());
    }
    #[test]
    fn state_wait_schema_defaults_and_shared_wait_budget_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("commands.json");
        let default_wait = serde_json::json!({"command": "wait_for", "state": {}});
        let command: Command = serde_json::from_value(default_wait.clone()).unwrap();
        assert!(matches!(
            command,
            Command::WaitFor {
                timeout_ms: 10_000,
                ..
            }
        ));
        for (milliseconds, valid) in [(20_000, true), (20_001, false)] {
            std::fs::write(
                &path,
                serde_json::to_vec(&serde_json::json!([
                    {"command": "wait", "milliseconds": 30_000},
                    {"command": "wait_for", "state": {"preview_ready": true}, "timeout_ms": 30_000},
                    default_wait,
                    {"command": "wait", "milliseconds": milliseconds},
                    {"command": "finish"}
                ]))
                .unwrap(),
            )
            .unwrap();
            let result = load(Some(&path));
            if valid {
                assert_eq!(result.unwrap().len(), 5);
            } else {
                assert!(result.err().unwrap().to_string().contains("90000"));
            }
        }
        for invalid in [
            serde_json::json!({"command": "wait_for", "state": {}, "timeout_ms": 30_001}),
            serde_json::json!({"command": "wait_for", "state": {"unknown": false}}),
            serde_json::json!({"command": "wait_for", "state": {"preview_ready": "true"}}),
            serde_json::json!({"command": "wait_for", "state": {"status_contains": 1}}),
            serde_json::json!({"command": "wait_for", "state": {"page": "unknown"}}),
            serde_json::json!({"command": "wait_for", "state": {"graph_nodes": -1}}),
            serde_json::json!({"command": "wait_for", "state": []}),
            serde_json::json!({"command": "wait_for", "state": null}),
            serde_json::json!({"command": "wait_for", "timeout_ms": 1}),
            serde_json::json!({"command": "wait_for", "state": {}, "timeout_ms": -1}),
            serde_json::json!({"command": "wait_for", "state": {}, "timeout_ms": 0.5}),
            serde_json::json!({"command": "wait_for", "state": {}, "typo": 1}),
        ] {
            std::fs::write(
                &path,
                serde_json::to_vec(&serde_json::json!([invalid, {"command": "finish"}])).unwrap(),
            )
            .unwrap();
            assert!(load(Some(&path)).is_err());
        }
    }
    #[test]
    fn state_wait_immediately_matches_without_dispatching_the_next_command() {
        let (_dir, mut app) = app();
        app.busy = true;
        app.audit_commands = serde_json::from_value(serde_json::json!([
            {"command": "wait_for", "state": app.state(), "timeout_ms": 0},
            {"command": "pause_preview"}
        ]))
        .unwrap();
        let _ = app.audit_poll();
        assert_eq!(app.audit_commands.len(), 1);
        assert!(app.audit_state_wait_started.is_none());
        assert!(!app.preview_paused);
        assert!(app.exit_failure.lock().unwrap().is_none());
    }
    #[test]
    fn state_wait_polls_until_current_generation_and_status_match() {
        let (_dir, mut app) = app();
        let wallpaper = Arc::new(
            Wallpaper::from_package(&crate::graph::Document::default().compile().unwrap().package)
                .unwrap(),
        );
        app.preview = Some(Preview {
            generation: 7,
            wallpaper,
            parameters: Default::default(),
            globals: Globals::new(1, 1),
            status: Default::default(),
        });
        let status = app.preview.as_ref().unwrap().status.clone();
        status.lock().unwrap().rendered_generation = Some(6);
        app.status = "compiling".into();
        app.audit_commands = serde_json::from_str(
            r#"[
                {"command":"wait_for","state":{"preview_ready":true,"status_contains":"ready"}},
                {"command":"pause_preview"}
            ]"#,
        )
        .unwrap();
        let _ = app.audit_poll();
        let started = app.audit_state_wait_started.unwrap();
        assert_eq!(app.audit_commands.len(), 2);
        assert!(!app.preview_paused);
        {
            let mut status = status.lock().unwrap();
            status.rendered_generation = Some(7);
            status.error = Some("replacement rejected".into());
        }
        app.status = "ready".into();
        let _ = app.audit_poll();
        assert_eq!(app.audit_commands.len(), 2);
        assert_eq!(app.audit_state_wait_started, Some(started));
        status.lock().unwrap().error = None;
        app.status = "compiling".into();
        let _ = app.audit_poll();
        assert_eq!(app.audit_commands.len(), 2);
        app.status = "preview ready".into();
        let _ = app.audit_poll();
        assert_eq!(app.audit_commands.len(), 1);
        assert!(app.audit_state_wait_started.is_none());
        assert!(!app.preview_paused);
        let _ = app.audit_poll();
        assert!(app.preview_paused);
        assert!(app.exit_failure.lock().unwrap().is_none());
    }
    #[test]
    fn state_wait_timeout_fails_with_the_mismatch_and_blocks_later_commands_even_when_busy() {
        let (_dir, mut app) = app();
        app.busy = true;
        app.audit_commands = serde_json::from_str(
            r#"[
                {"command":"wait_for","state":{"preview_ready":true},"timeout_ms":100},
                {"command":"pause_preview"}
            ]"#,
        )
        .unwrap();
        let _ = app.audit_poll();
        assert!(app.audit_state_wait_started.is_some());
        assert!(app.exit_failure.lock().unwrap().is_none());
        app.audit_state_wait_started = Some(Instant::now() - Duration::from_millis(101));
        let _ = app.audit_poll();
        let failure = app.exit_failure.lock().unwrap().clone().unwrap();
        assert!(failure.contains("wait_for timed out after 100 milliseconds"));
        assert!(failure.contains("preview_ready: expected true, got false"));
        assert!(failure.contains("current state:"));
        assert_eq!(app.audit_commands.len(), 2);
        assert!(!app.preview_paused);
    }
    #[test]
    fn state_wait_invalid_expectations_fail_before_starting_the_timeout() {
        for state in [
            serde_json::json!({"unknown": false}),
            serde_json::json!({"preview_ready": "true"}),
            serde_json::json!({"status_contains": false}),
            serde_json::json!({"designer_name": 1}),
            serde_json::json!({"parameter_errors": null}),
        ] {
            let (_dir, mut app) = app();
            app.busy = true;
            app.audit_commands = serde_json::from_value(serde_json::json!([
                {"command": "wait_for", "state": state, "timeout_ms": 30_000},
                {"command": "pause_preview"}
            ]))
            .unwrap();
            let _ = app.audit_poll();
            assert!(app.audit_state_wait_started.is_none());
            assert!(app.exit_failure.lock().unwrap().is_some());
            assert_eq!(app.audit_commands.len(), 2);
            assert!(!app.preview_paused);
        }
    }
    #[test]
    fn unfocused_audits_subscribe_preview_ticks_while_honoring_explicit_pause() {
        let wallpaper = Arc::new(
            Wallpaper::from_package(&crate::graph::Document::default().compile().unwrap().package)
                .unwrap(),
        );
        for page in [Page::Library, Page::Designer, Page::Graph] {
            let (_dir, mut app) = app();
            app.page = page;
            let preview = Some(Preview {
                generation: 1,
                wallpaper: wallpaper.clone(),
                parameters: Default::default(),
                globals: Globals::new(1, 1),
                status: Default::default(),
            });
            match page {
                Page::Library => app.preview = preview,
                Page::Designer => app.designer.preview = preview,
                Page::Graph => app.graph.preview = preview,
                _ => unreachable!(),
            }
            let _ = app.update(Message::Event(iced::Event::Window(
                iced::window::Event::Unfocused,
            )));
            let background_subscriptions = app.subscription().units();
            let _ = app.update(Message::Event(iced::Event::Window(
                iced::window::Event::Focused,
            )));
            assert_eq!(app.subscription().units(), background_subscriptions + 1);
            let _ = app.update(Message::Event(iced::Event::Window(
                iced::window::Event::Unfocused,
            )));
            app.audit_commands =
                serde_json::from_str(r#"[{"command":"wait_for","state":{"preview_ready":true}}]"#)
                    .unwrap();
            let _ = app.update(Message::PausePreview);
            let audit_subscriptions = app.subscription().units();
            assert_eq!(audit_subscriptions, background_subscriptions + 1);
            let _ = app.update(Message::PausePreview);
            assert_eq!(app.subscription().units(), audit_subscriptions + 1);
            app.audit_commands.clear();
            assert_eq!(app.subscription().units(), background_subscriptions);
        }
    }
    #[test]
    fn scripts_reject_finish_before_the_last_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("commands.json");
        for invalid in [
            r#"[{"command":"finish"},{"command":"assert","state":{"paused":true}},{"command":"finish"}]"#,
            r#"[{"command":"finish"},{"command":"finish"}]"#,
            r#"[{"command":"finish"},{"command":"assert","state":{"paused":true}}]"#,
        ] {
            std::fs::write(&path, invalid).unwrap();
            let error = load(Some(&path)).err().unwrap();
            assert!(error.to_string().contains("exactly one finish at the end"));
        }
        std::fs::write(&path, r#"[{"command":"finish"}]"#).unwrap();
        assert_eq!(load(Some(&path)).unwrap().len(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn audit_fifo_is_rejected_without_opening_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("commands.fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        assert!(load(Some(&path)).is_err());
    }
    #[test]
    fn empty_and_paused_screenshots_do_not_require_wallpaper_ticks() {
        let (_dir, mut app) = app();
        app.final_screenshot = Some("empty.png".into());
        app.screenshot_delay = Duration::ZERO;
        app.preview_paused = true;
        let _ = app.audit_poll();
        assert!(app.active_capture.is_some());
        assert!(app.exit_failure.lock().unwrap().is_none());
    }
    #[test]
    fn failed_startup_captures_the_error_screen_and_retains_failure_verdict() {
        let (_dir, mut app) = app();
        let _ = app.update(Message::Designer(DesignerMessage::Opened(Err(
            "missing source".into(),
        ))));
        app.final_screenshot = Some("error.png".into());
        app.screenshot_delay = Duration::ZERO;
        let _ = app.audit_poll();
        assert!(app.active_capture.is_some());
        assert!(
            app.exit_failure
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("missing source")
        );
    }
    #[test]
    fn smoke_requires_the_requested_generation_without_replacement_errors() {
        let (_dir, mut app) = app();
        let doc = crate::graph::Document::default();
        let wallpaper = Arc::new(Wallpaper::from_package(&doc.compile().unwrap().package).unwrap());
        app.preview = Some(Preview {
            generation: 7,
            wallpaper,
            parameters: Default::default(),
            globals: Globals::new(1, 1),
            status: Default::default(),
        });
        let status = app.preview.as_ref().unwrap().status.clone();
        status.lock().unwrap().frames = 10;
        assert!(!app.smoke_ready(3));
        status.lock().unwrap().rendered_generation = Some(6);
        assert!(!app.smoke_ready(3));
        status.lock().unwrap().rendered_generation = Some(7);
        assert!(app.smoke_ready(3));
        status.lock().unwrap().error = Some("replacement rejected".into());
        assert!(!app.smoke_ready(3));
        app.frame_limit = Some(3);
        let _ = app.audit_poll();
        assert!(
            app.exit_failure
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("replacement rejected")
        );
    }
    #[test]
    fn audit_queue_uses_real_updates_waits_for_busy_work_and_rejects_bad_assertions() {
        let (_dir, mut app) = app();
        app.audit_commands = serde_json::from_str(
            r#"[
            {"command":"page","value":"settings"},
            {"command":"pause_preview"},
            {"command":"assert","state":{"page":"settings","paused":true}},
            {"command":"assert","state":{"unknown":false}}
        ]"#,
        )
        .unwrap();
        app.busy = true;
        let _ = app.audit_poll();
        assert_eq!(app.audit_commands.len(), 4);
        app.busy = false;
        for _ in 0..4 {
            let _ = app.audit_poll();
        }
        assert!(app.preview_paused);
        assert!(
            app.exit_failure
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("unknown")
        );
    }
    #[test]
    fn unavailable_screenshots_and_unfinished_smoke_runs_fail_with_bounded_timeouts() {
        let (_dir, mut app) = app();
        let _ = app.capture("timeout.png".into(), CaptureCompletion::Exit);
        app.active_capture.as_mut().unwrap().started = Instant::now() - Duration::from_secs(11);
        let _ = app.audit_poll();
        assert!(
            app.exit_failure
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("screenshot timed out")
        );
        app.active_capture = None;
        *app.exit_failure.lock().unwrap() = None;
        app.frame_limit = Some(1);
        app.started = Instant::now() - Duration::from_secs(46);
        let _ = app.audit_poll();
        assert!(
            app.exit_failure
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("smoke test timed out")
        );
    }
    #[test]
    fn finishing_an_audit_does_not_bypass_requested_frame_smoke_checks() {
        let (_dir, mut app) = app();
        app.frame_limit = Some(2);
        app.audit_commands.push_back(Command::Finish {});
        let _ = app.audit_poll();
        assert_eq!(app.audit_commands.len(), 1);
        assert_eq!(app.frame_limit, Some(2));
        assert!(app.exit_failure.lock().unwrap().is_none());
    }
    #[test]
    fn screenshot_completion_rechecks_gpu_failures_and_keeps_existing_files() {
        let (dir, mut app) = app();
        let wallpaper = Arc::new(
            Wallpaper::from_package(&crate::graph::Document::default().compile().unwrap().package)
                .unwrap(),
        );
        app.preview = Some(Preview {
            generation: 1,
            wallpaper,
            parameters: Default::default(),
            globals: Globals::new(1, 1),
            status: Default::default(),
        });
        let path = dir.path().join("capture.png");
        let _ = app.capture(path.clone(), CaptureCompletion::Exit);
        app.preview.as_ref().unwrap().status.lock().unwrap().error =
            Some("late GPU failure".into());
        let screenshot =
            iced::window::Screenshot::new(vec![10, 20, 30, 255], iced::Size::new(1, 1), 1.);
        let _ = app.update(Message::Screenshot(screenshot.clone()));
        assert!(path.is_file());
        assert!(
            app.exit_failure
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("late GPU failure")
        );
        let original = std::fs::read(&path).unwrap();
        let _ = app.capture(path.clone(), CaptureCompletion::Checkpoint);
        let _ = app.update(Message::Screenshot(screenshot));
        assert!(
            app.exit_failure
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("Could not save screenshot")
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    #[test]
    fn scripted_screenshots_preserve_the_separate_final_cli_destination() {
        let (dir, mut app) = app();
        let checkpoint = dir.path().join("checkpoint.png");
        let final_path = dir.path().join("final.png");
        app.final_screenshot = Some(final_path.clone());
        app.audit_commands = VecDeque::from([
            Command::Screenshot {
                path: checkpoint.clone(),
            },
            Command::Finish {},
        ]);
        let screenshot =
            iced::window::Screenshot::new(vec![10, 20, 30, 255], iced::Size::new(1, 1), 1.);
        let _ = app.audit_poll();
        assert_eq!(app.active_capture.as_ref().unwrap().path, checkpoint);
        assert_eq!(app.final_screenshot, Some(final_path.clone()));
        let _ = app.update(Message::Screenshot(screenshot.clone()));
        assert!(checkpoint.is_file());
        assert!(!final_path.exists());
        let _ = app.audit_poll();
        let capture = app.active_capture.as_ref().unwrap();
        assert_eq!(capture.path, final_path);
        assert!(capture.completion == CaptureCompletion::AuditFinish);
        assert!(app.final_screenshot.is_none());
        assert_eq!(app.audit_commands.len(), 1);
        let _ = app.update(Message::Screenshot(screenshot));
        assert!(final_path.is_file());
        let _ = app.audit_poll();
        assert!(app.audit_commands.is_empty());
        assert!(app.active_capture.is_none());
        assert!(app.exit_failure.lock().unwrap().is_none());
    }
    #[test]
    fn checkpoint_errors_can_recover_before_the_terminal_capture() {
        let (dir, mut app) = app();
        let checkpoint = dir.path().join("invalid.png");
        let final_path = dir.path().join("recovered.png");
        app.final_screenshot = Some(final_path.clone());
        app.audit_commands = VecDeque::from([
            Command::Screenshot {
                path: checkpoint.clone(),
            },
            Command::Finish {},
        ]);
        app.preview = Some(Preview {
            generation: 1,
            wallpaper: Arc::new(
                Wallpaper::from_package(
                    &crate::graph::Document::default().compile().unwrap().package,
                )
                .unwrap(),
            ),
            parameters: Default::default(),
            globals: Globals::new(1, 1),
            status: Default::default(),
        });
        let status = app.preview.as_ref().unwrap().status.clone();
        status.lock().unwrap().error = Some("intentional invalid edit".into());
        let screenshot =
            iced::window::Screenshot::new(vec![10, 20, 30, 255], iced::Size::new(1, 1), 1.);
        let _ = app.audit_poll();
        let _ = app.update(Message::Screenshot(screenshot.clone()));
        assert!(checkpoint.is_file());
        assert!(app.exit_failure.lock().unwrap().is_none());
        status.lock().unwrap().error = None;
        let _ = app.audit_poll();
        let _ = app.update(Message::Screenshot(screenshot));
        let _ = app.audit_poll();
        assert!(final_path.is_file());
        assert!(app.exit_failure.lock().unwrap().is_none());
        assert!(app.audit_commands.is_empty());
    }
    #[test]
    fn final_cli_capture_after_finish_retains_startup_and_late_gpu_failures() {
        for late_gpu_error in [false, true] {
            let (dir, mut app) = app();
            let path = dir.path().join("error.png");
            app.final_screenshot = Some(path.clone());
            app.audit_commands.push_back(Command::Finish {});
            if !late_gpu_error {
                let _ = app.update(Message::Designer(DesignerMessage::Opened(Err(
                    "missing source".into(),
                ))));
            }
            let _ = app.audit_poll();
            assert!(
                app.active_capture.as_ref().unwrap().completion == CaptureCompletion::AuditFinish
            );
            if late_gpu_error {
                app.preview = Some(Preview {
                    generation: 1,
                    wallpaper: Arc::new(
                        Wallpaper::from_package(
                            &crate::graph::Document::default().compile().unwrap().package,
                        )
                        .unwrap(),
                    ),
                    parameters: Default::default(),
                    globals: Globals::new(1, 1),
                    status: Default::default(),
                });
                app.preview.as_ref().unwrap().status.lock().unwrap().error =
                    Some("late GPU failure".into());
            }
            let screenshot =
                iced::window::Screenshot::new(vec![10, 20, 30, 255], iced::Size::new(1, 1), 1.);
            let _ = app.update(Message::Screenshot(screenshot));
            assert!(path.is_file());
            let expected_error = if late_gpu_error {
                "late GPU failure"
            } else {
                "missing source"
            };
            assert!(
                app.exit_failure
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .contains(expected_error)
            );
        }
    }
    #[test]
    fn accepted_cpu_failures_allow_checkpoint_capture_and_matching_recovery() {
        for failure in 0..5 {
            let (dir, mut app) = app();
            let wallpaper = Arc::new(
                Wallpaper::from_package(
                    &crate::graph::Document::default().compile().unwrap().package,
                )
                .unwrap(),
            );
            let error = "intentional CPU failure".to_owned();
            let message = match failure {
                0 => Message::Loaded(Err(error.clone())),
                1 => Message::Designer(DesignerMessage::Opened(Err(error.clone()))),
                2 => Message::Designer(DesignerMessage::Compiled(Err(error.clone()))),
                3 => Message::Graph(GraphMessage::Opened(Err(error.clone()))),
                _ => Message::Graph(GraphMessage::Compiled(0, Err(error.clone()))),
            };
            let _ = app.update(message);
            app.assert_state(&if failure == 4 {
                serde_json::json!({ "graph_diagnostic": error })
            } else {
                serde_json::json!({ "status_contains": error })
            })
            .unwrap();
            let checkpoint = dir.path().join("invalid.png");
            let _ = app.capture(checkpoint.clone(), CaptureCompletion::Checkpoint);
            let screenshot =
                iced::window::Screenshot::new(vec![10, 20, 30, 255], iced::Size::new(1, 1), 1.);
            let _ = app.update(Message::Screenshot(screenshot.clone()));
            assert!(checkpoint.is_file());
            assert!(app.exit_failure.lock().unwrap().is_none());
            let _ = app.update(Message::Scanned(Ok((vec![], vec![]))));
            let _ = app.update(Message::Designer(DesignerMessage::Opened(Ok(None))));
            let _ = app.update(Message::Graph(GraphMessage::Opened(Ok(None))));
            assert_eq!(app.capture_error(), Some(error));
            let recovered = match failure {
                0 => Message::Loaded(Ok(("wallpaper".into(), wallpaper, BTreeMap::new()))),
                1 | 2 => {
                    Message::Designer(DesignerMessage::Compiled(Ok((wallpaper, "ready".into()))))
                }
                _ => Message::Graph(GraphMessage::Compiled(0, Ok((wallpaper, "ready".into())))),
            };
            let _ = app.update(recovered);
            assert!(app.capture_error().is_none());
            let final_path = dir.path().join("recovered.png");
            app.final_screenshot = Some(final_path.clone());
            app.audit_commands.push_back(Command::Finish {});
            let _ = app.audit_poll();
            let _ = app.update(Message::Screenshot(screenshot));
            let _ = app.audit_poll();
            assert!(final_path.is_file());
            assert!(app.exit_failure.lock().unwrap().is_none());
            assert!(app.audit_commands.is_empty());
        }
    }
    #[test]
    fn graph_recovery_preserves_scan_failures_until_a_matching_library_recovery() {
        for partial_scan in [false, true] {
            for recover_scan in [false, true] {
                let (dir, mut app) = app();
                let scan_error = "library scan still failed";
                let scan_result = if partial_scan {
                    Ok((vec![], vec![scan_error.into()]))
                } else {
                    Err(scan_error.into())
                };
                let _ = app.update(Message::Scanned(scan_result));
                let _ = app.update(Message::Graph(GraphMessage::Opened(Err(
                    "graph open failed".into(),
                ))));

                let document = Arc::new(crate::graph::Document::default());
                let wallpaper = Arc::new(
                    Wallpaper::from_package(&document.compile().unwrap().package).unwrap(),
                );
                let _ = app.update(Message::Graph(GraphMessage::Opened(Ok(Some(document)))));
                let _ = app.update(Message::Graph(GraphMessage::Compiled(
                    app.graph.revision,
                    Ok((wallpaper, "ready".into())),
                )));
                assert_eq!(app.capture_error().as_deref(), Some(scan_error));

                if recover_scan {
                    let _ = app.update(Message::Scanned(Ok((vec![], vec![]))));
                    assert!(app.capture_error().is_none());
                }
                let final_path = dir.path().join("terminal.png");
                app.final_screenshot = Some(final_path.clone());
                app.audit_commands.push_back(Command::Finish {});
                let _ = app.audit_poll();
                assert!(
                    app.active_capture.as_ref().unwrap().completion
                        == CaptureCompletion::AuditFinish
                );
                let screenshot =
                    iced::window::Screenshot::new(vec![10, 20, 30, 255], iced::Size::new(1, 1), 1.);
                let _ = app.update(Message::Screenshot(screenshot));
                assert!(final_path.is_file());
                assert_eq!(
                    app.exit_failure.lock().unwrap().as_deref(),
                    (!recover_scan).then_some(scan_error)
                );
                if recover_scan {
                    let _ = app.audit_poll();
                    assert!(app.audit_commands.is_empty());
                }
            }
        }
    }
    #[test]
    fn stale_graph_compiles_cannot_poison_or_clear_capture_and_frame_verdicts() {
        let (dir, mut app) = app();
        let wallpaper = Arc::new(
            Wallpaper::from_package(&crate::graph::Document::default().compile().unwrap().package)
                .unwrap(),
        );
        app.page = Page::Graph;
        app.graph.revision = 2;
        let _ = app.update(Message::Graph(GraphMessage::Compiled(
            2,
            Err("current error".into()),
        )));
        let _ = app.update(Message::Graph(GraphMessage::Compiled(
            1,
            Ok((wallpaper.clone(), "stale".into())),
        )));
        assert_eq!(app.capture_error().as_deref(), Some("current error"));
        let _ = app.update(Message::Graph(GraphMessage::Compiled(
            2,
            Ok((wallpaper, "ready".into())),
        )));
        let _ = app.update(Message::Graph(GraphMessage::Compiled(
            1,
            Err("stale error".into()),
        )));
        assert!(app.capture_error().is_none());
        let preview = app.graph.preview.as_ref().unwrap();
        {
            let mut status = preview.status.lock().unwrap();
            status.rendered_generation = Some(preview.generation);
            status.frames = 2;
        }
        app.frame_limit = Some(2);
        app.final_screenshot = Some(dir.path().join("frames.png"));
        app.audit_commands.push_back(Command::Finish {});
        let _ = app.audit_poll();
        assert!(app.frame_limit.is_none());
        assert!(app.active_capture.is_some());
        let screenshot =
            iced::window::Screenshot::new(vec![10, 20, 30, 255], iced::Size::new(1, 1), 1.);
        let _ = app.update(Message::Screenshot(screenshot));
        let _ = app.audit_poll();
        assert!(app.exit_failure.lock().unwrap().is_none());
        assert!(app.audit_commands.is_empty());
        assert!(dir.path().join("frames.png").is_file());
    }
    #[test]
    fn successful_cpu_results_preserve_unresolved_configuration_failures() {
        let (_dir, mut app) = app();
        app.startup_error = Some("invalid configuration".into());
        let _ = app.update(Message::Scanned(Ok((vec![], vec![]))));
        assert_eq!(
            app.capture_error().as_deref(),
            Some("invalid configuration")
        );
    }
    #[test]
    fn malformed_commands_fail_and_screenshots_block_the_queue_until_written() {
        assert!(
            serde_json::from_str::<Command>(r#"{"command":"pause_preview","typo":1}"#).is_err()
        );
        assert!(
            Command::LayerValue {
                id: "typo".into(),
                value: 1.
            }
            .message()
            .is_err()
        );
        assert!(
            Command::DesignerEffect {
                effect: "typo".into(),
                enabled: true
            }
            .message()
            .is_err()
        );
        let (_dir, mut app) = app();
        app.audit_commands = serde_json::from_str(
            r#"[{"command":"screenshot","path":"frame.png"},{"command":"pause_preview"}]"#,
        )
        .unwrap();
        let _ = app.audit_poll();
        assert!(app.active_capture.is_some());
        assert!(matches!(
            app.active_capture.as_ref().unwrap().completion,
            CaptureCompletion::Checkpoint
        ));
        let _ = app.audit_poll();
        assert!(!app.preview_paused);
        assert_eq!(app.audit_commands.len(), 1);
    }
}
