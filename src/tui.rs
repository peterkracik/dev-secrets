//! The interactive, k9s-style terminal UI.
//!
//! Three master/detail panes — Projects → Environments → Secrets — are shown
//! side by side. Focus moves rightward as you drill in. Every mutating action
//! is available through a single keypress and persists immediately.

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::model::{Environment, Project};
use crate::store::StoreHandle;
use crate::{envfile, resolve};

/// Which pane currently has keyboard focus.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Projects,
    Envs,
    Secrets,
}

/// What a submitted text prompt should do.
enum InputAction {
    NewProject,
    NewEnv,
    NewSecret,
    EditSecret { key: String },
    DuplicateEnv { from: String },
    Import,
    Export,
    SetFolder,
}

struct InputState {
    title: String,
    prompt: String,
    buffer: String,
    action: InputAction,
}

/// What a confirmation prompt should do when accepted.
#[allow(clippy::enum_variant_names)]
enum ConfirmAction {
    DeleteProject(String),
    DeleteEnv {
        project: String,
        env: String,
    },
    DeleteSecret {
        project: String,
        env: String,
        key: String,
    },
}

struct ConfirmState {
    message: String,
    action: ConfirmAction,
}

/// A minimal in-app multi-line editor for a whole environment, shown as a
/// `.env` document. Lines are edited directly; on save the text is parsed
/// back and replaces the environment's contents.
struct EnvEditor {
    project: String,
    env: String,
    lines: Vec<String>,
    /// Cursor column, as a character index within the current line.
    cx: usize,
    /// Cursor row.
    cy: usize,
}

impl EnvEditor {
    fn cur_len(&self) -> usize {
        self.lines[self.cy].chars().count()
    }

    fn insert_char(&mut self, c: char) {
        let line = &mut self.lines[self.cy];
        let byte = char_byte_idx(line, self.cx);
        line.insert(byte, c);
        self.cx += 1;
    }

    fn insert_newline(&mut self) {
        let line = &mut self.lines[self.cy];
        let byte = char_byte_idx(line, self.cx);
        let rest = line.split_off(byte);
        self.lines.insert(self.cy + 1, rest);
        self.cy += 1;
        self.cx = 0;
    }

    fn backspace(&mut self) {
        if self.cx > 0 {
            let line = &mut self.lines[self.cy];
            let start = char_byte_idx(line, self.cx - 1);
            let end = char_byte_idx(line, self.cx);
            line.replace_range(start..end, "");
            self.cx -= 1;
        } else if self.cy > 0 {
            let cur = self.lines.remove(self.cy);
            self.cy -= 1;
            self.cx = self.cur_len();
            self.lines[self.cy].push_str(&cur);
        }
    }

    fn delete_forward(&mut self) {
        if self.cx < self.cur_len() {
            let line = &mut self.lines[self.cy];
            let start = char_byte_idx(line, self.cx);
            let end = char_byte_idx(line, self.cx + 1);
            line.replace_range(start..end, "");
        } else if self.cy + 1 < self.lines.len() {
            let next = self.lines.remove(self.cy + 1);
            self.lines[self.cy].push_str(&next);
        }
    }

    fn move_left(&mut self) {
        if self.cx > 0 {
            self.cx -= 1;
        } else if self.cy > 0 {
            self.cy -= 1;
            self.cx = self.cur_len();
        }
    }

    fn move_right(&mut self) {
        if self.cx < self.cur_len() {
            self.cx += 1;
        } else if self.cy + 1 < self.lines.len() {
            self.cy += 1;
            self.cx = 0;
        }
    }

    fn move_up(&mut self) {
        if self.cy > 0 {
            self.cy -= 1;
            self.cx = self.cx.min(self.cur_len());
        }
    }

    fn move_down(&mut self) {
        if self.cy + 1 < self.lines.len() {
            self.cy += 1;
            self.cx = self.cx.min(self.cur_len());
        }
    }
}

pub struct App {
    handle: StoreHandle,
    focus: Focus,
    proj_idx: usize,
    env_idx: usize,
    secret_idx: usize,
    reveal: bool,
    show_help: bool,
    status: String,
    input: Option<InputState>,
    confirm: Option<ConfirmState>,
    /// Active in-app full-environment editor, if any.
    editor: Option<EnvEditor>,
    /// Set when the user asks to edit the current environment in `$EDITOR`.
    /// Handled in the main loop (where the terminal can be suspended).
    pending_editor: bool,
    should_quit: bool,
}

/// Entry point: set up the terminal, run the loop, restore on exit.
pub fn run() -> Result<()> {
    let handle = StoreHandle::open()?;
    let mut app = App::new(handle);
    let mut terminal = ratatui::init();
    let result = app.run_loop(&mut terminal);
    ratatui::restore();
    result
}

impl App {
    fn new(handle: StoreHandle) -> Self {
        App {
            handle,
            focus: Focus::Projects,
            proj_idx: 0,
            env_idx: 0,
            secret_idx: 0,
            reveal: false,
            show_help: false,
            status: "Welcome to dev-secrets. Press ? for help.".to_string(),
            input: None,
            confirm: None,
            editor: None,
            pending_editor: false,
            should_quit: false,
        }
    }

    fn run_loop<B: ratatui::backend::Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        while !self.should_quit {
            terminal.draw(|f| self.draw(f))?;
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    self.on_key(key)?;
                }
            }
            if self.pending_editor {
                self.pending_editor = false;
                self.edit_env_in_editor(terminal)?;
            }
        }
        Ok(())
    }

    // --- store navigation helpers -----------------------------------------

    fn current_project_name(&self) -> Option<String> {
        self.handle
            .store
            .projects
            .get_index(self.proj_idx)
            .map(|(k, _)| k.clone())
    }

    fn current_project(&self) -> Option<&Project> {
        self.handle
            .store
            .projects
            .get_index(self.proj_idx)
            .map(|(_, p)| p)
    }

    fn current_env_name(&self) -> Option<String> {
        self.current_project()?
            .environments
            .get_index(self.env_idx)
            .map(|(k, _)| k.clone())
    }

    fn current_env(&self) -> Option<&Environment> {
        self.current_project()?
            .environments
            .get_index(self.env_idx)
            .map(|(_, e)| e)
    }

    fn current_secret_key(&self) -> Option<String> {
        self.current_env()?
            .values
            .get_index(self.secret_idx)
            .map(|(k, _)| k.clone())
    }

    fn clamp_indices(&mut self) {
        let proj_count = self.handle.store.projects.len();
        if self.proj_idx >= proj_count {
            self.proj_idx = proj_count.saturating_sub(1);
        }
        let env_count = self
            .current_project()
            .map(|p| p.environments.len())
            .unwrap_or(0);
        if self.env_idx >= env_count {
            self.env_idx = env_count.saturating_sub(1);
        }
        let secret_count = self.current_env().map(|e| e.values.len()).unwrap_or(0);
        if self.secret_idx >= secret_count {
            self.secret_idx = secret_count.saturating_sub(1);
        }
    }

    fn save(&mut self) {
        if let Err(e) = self.handle.save() {
            self.status = format!("Save failed: {e}");
        }
    }

    // --- key handling ------------------------------------------------------

    fn on_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.editor.is_some() {
            return self.on_editor_key(key);
        }
        if self.input.is_some() {
            return self.on_input_key(key);
        }
        if self.confirm.is_some() {
            return self.on_confirm_key(key);
        }
        if self.show_help {
            self.show_help = false;
            return Ok(());
        }
        self.on_nav_key(key)
    }

    fn on_nav_key(&mut self, key: KeyEvent) -> Result<()> {
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('q'), _) => {
                self.should_quit = true;
            }
            (KeyCode::Char('?'), _) => self.show_help = true,
            (KeyCode::Char('s'), _) => self.reveal = !self.reveal,
            (KeyCode::Tab, _) => self.cycle_focus(),
            (KeyCode::Down, _) | (KeyCode::Char('j'), _) => self.move_selection(1),
            (KeyCode::Up, _) | (KeyCode::Char('k'), _) => self.move_selection(-1),
            (KeyCode::Right, _) | (KeyCode::Char('l'), _) => self.focus_deeper(),
            (KeyCode::Left, _) | (KeyCode::Char('h'), _) => self.focus_shallower(),
            (KeyCode::Enter, _) => self.on_enter(),
            (KeyCode::Char('n'), _) => self.start_new(),
            (KeyCode::Char('e'), _) => self.start_edit(),
            (KeyCode::Char('a'), _) => self.open_inline_editor(),
            (KeyCode::Char('E'), _) => self.request_edit_env(),
            (KeyCode::Char('d'), _) => self.start_delete(),
            (KeyCode::Char('y'), _) => self.start_duplicate(),
            (KeyCode::Char('i'), _) => self.start_import(),
            (KeyCode::Char('x'), _) => self.start_export(),
            (KeyCode::Char('D'), _) => self.set_default_env(),
            (KeyCode::Char('f'), _) => self.start_set_folder(),
            _ => {}
        }
        Ok(())
    }

    fn cycle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Projects => Focus::Envs,
            Focus::Envs => Focus::Secrets,
            Focus::Secrets => Focus::Projects,
        };
    }

    fn focus_deeper(&mut self) {
        match self.focus {
            Focus::Projects if self.current_project().is_some() => self.focus = Focus::Envs,
            Focus::Envs if self.current_env().is_some() => self.focus = Focus::Secrets,
            _ => {}
        }
    }

    fn focus_shallower(&mut self) {
        self.focus = match self.focus {
            Focus::Secrets => Focus::Envs,
            Focus::Envs => Focus::Projects,
            Focus::Projects => Focus::Projects,
        };
    }

    fn on_enter(&mut self) {
        match self.focus {
            Focus::Projects | Focus::Envs => self.focus_deeper(),
            Focus::Secrets => self.start_edit_secret(),
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let count = match self.focus {
            Focus::Projects => self.handle.store.projects.len(),
            Focus::Envs => self
                .current_project()
                .map(|p| p.environments.len())
                .unwrap_or(0),
            Focus::Secrets => self.current_env().map(|e| e.values.len()).unwrap_or(0),
        };
        if count == 0 {
            return;
        }
        let idx = match self.focus {
            Focus::Projects => &mut self.proj_idx,
            Focus::Envs => &mut self.env_idx,
            Focus::Secrets => &mut self.secret_idx,
        };
        let new = (*idx as i32 + delta).rem_euclid(count as i32) as usize;
        *idx = new;
        // Reset deeper selections when moving in a parent pane.
        match self.focus {
            Focus::Projects => {
                self.env_idx = 0;
                self.secret_idx = 0;
            }
            Focus::Envs => self.secret_idx = 0,
            Focus::Secrets => {}
        }
    }

    // --- action starters ---------------------------------------------------

    fn start_new(&mut self) {
        match self.focus {
            Focus::Projects => {
                self.input = Some(InputState {
                    title: "New project".into(),
                    prompt: "Project name:".into(),
                    buffer: String::new(),
                    action: InputAction::NewProject,
                })
            }
            Focus::Envs => {
                if self.current_project().is_none() {
                    self.status = "Create a project first.".into();
                    return;
                }
                self.input = Some(InputState {
                    title: "New environment".into(),
                    prompt: "Environment name:".into(),
                    buffer: String::new(),
                    action: InputAction::NewEnv,
                });
            }
            Focus::Secrets => {
                if self.current_env().is_none() {
                    self.status = "Create an environment first.".into();
                    return;
                }
                self.input = Some(InputState {
                    title: "New secret".into(),
                    prompt: "KEY=VALUE:".into(),
                    buffer: String::new(),
                    action: InputAction::NewSecret,
                });
            }
        }
    }

    /// Context-aware edit: a single secret when focused on the Secrets pane,
    /// otherwise the whole environment in the inline editor.
    fn start_edit(&mut self) {
        if self.focus == Focus::Secrets && self.current_secret_key().is_some() {
            self.start_edit_secret();
        } else {
            self.open_inline_editor();
        }
    }

    /// Open the in-app full-environment editor for the current environment.
    fn open_inline_editor(&mut self) {
        let (Some(project), Some(env)) = (self.current_project_name(), self.current_env_name())
        else {
            self.status = "Select an environment to edit.".into();
            return;
        };
        let values = self
            .current_env()
            .map(|e| e.values.clone())
            .unwrap_or_default();
        let mut lines: Vec<String> = envfile::serialize(&values)
            .lines()
            .map(|l| l.to_string())
            .collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        self.editor = Some(EnvEditor {
            project,
            env,
            lines,
            cx: 0,
            cy: 0,
        });
    }

    fn start_edit_secret(&mut self) {
        if self.focus != Focus::Secrets {
            return;
        }
        let Some(key) = self.current_secret_key() else {
            return;
        };
        let current = self
            .current_env()
            .and_then(|e| e.values.get(&key))
            .cloned()
            .unwrap_or_default();
        self.input = Some(InputState {
            title: format!("Edit {key}"),
            prompt: "Value:".into(),
            buffer: current,
            action: InputAction::EditSecret { key },
        });
    }

    /// Flag that the current environment should be opened in `$EDITOR`. The
    /// actual launch happens back in the main loop, which owns the terminal.
    fn request_edit_env(&mut self) {
        if self.current_env().is_none() {
            self.status = "Select an environment to edit.".into();
            return;
        }
        self.pending_editor = true;
    }

    fn start_delete(&mut self) {
        match self.focus {
            Focus::Projects => {
                if let Some(name) = self.current_project_name() {
                    self.confirm = Some(ConfirmState {
                        message: format!("Delete project `{name}` and all its environments?"),
                        action: ConfirmAction::DeleteProject(name),
                    });
                }
            }
            Focus::Envs => {
                if let (Some(project), Some(env)) =
                    (self.current_project_name(), self.current_env_name())
                {
                    self.confirm = Some(ConfirmState {
                        message: format!("Delete environment `{project}/{env}`?"),
                        action: ConfirmAction::DeleteEnv { project, env },
                    });
                }
            }
            Focus::Secrets => {
                if let (Some(project), Some(env), Some(key)) = (
                    self.current_project_name(),
                    self.current_env_name(),
                    self.current_secret_key(),
                ) {
                    self.confirm = Some(ConfirmState {
                        message: format!("Delete secret `{key}` from `{project}/{env}`?"),
                        action: ConfirmAction::DeleteSecret { project, env, key },
                    });
                }
            }
        }
    }

    fn start_duplicate(&mut self) {
        if self.focus != Focus::Envs {
            self.status = "Duplicate works on environments (focus the Envs pane).".into();
            return;
        }
        let Some(from) = self.current_env_name() else {
            return;
        };
        self.input = Some(InputState {
            title: format!("Duplicate {from}"),
            prompt: "New environment name:".into(),
            buffer: String::new(),
            action: InputAction::DuplicateEnv { from },
        });
    }

    fn start_import(&mut self) {
        if self.current_env().is_none() {
            self.status = "Select an environment to import into.".into();
            return;
        }
        self.input = Some(InputState {
            title: "Import .env".into(),
            prompt: "Path to .env file:".into(),
            buffer: String::new(),
            action: InputAction::Import,
        });
    }

    fn start_export(&mut self) {
        if self.current_env().is_none() {
            self.status = "Select an environment to export.".into();
            return;
        }
        self.input = Some(InputState {
            title: "Export .env".into(),
            prompt: "Output path:".into(),
            buffer: ".env".into(),
            action: InputAction::Export,
        });
    }

    fn start_set_folder(&mut self) {
        if self.focus != Focus::Projects || self.current_project().is_none() {
            self.status = "Focus a project to assign its folder.".into();
            return;
        }
        let current = self
            .current_project()
            .and_then(|p| p.folder.clone())
            .unwrap_or_default();
        self.input = Some(InputState {
            title: "Assign folder".into(),
            prompt: "Folder path (blank = current dir):".into(),
            buffer: current,
            action: InputAction::SetFolder,
        });
    }

    fn set_default_env(&mut self) {
        if self.focus != Focus::Envs {
            self.status = "Set default works on environments.".into();
            return;
        }
        if let (Some(project), Some(env)) = (self.current_project_name(), self.current_env_name()) {
            if let Some(p) = self.handle.store.project_mut(&project) {
                p.default_env = Some(env.clone());
            }
            self.save();
            self.status = format!("Default env for `{project}` is now `{env}`.");
        }
    }

    // --- input modal -------------------------------------------------------

    fn on_input_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(input) = self.input.as_mut() else {
            return Ok(());
        };
        match key.code {
            KeyCode::Esc => {
                self.input = None;
            }
            KeyCode::Enter => {
                let state = self.input.take().unwrap();
                self.submit_input(state)?;
            }
            KeyCode::Backspace => {
                input.buffer.pop();
            }
            KeyCode::Char(c) => {
                input.buffer.push(c);
            }
            _ => {}
        }
        Ok(())
    }

    fn submit_input(&mut self, state: InputState) -> Result<()> {
        let value = state.buffer.trim().to_string();
        match state.action {
            InputAction::NewProject => {
                if value.is_empty() {
                    self.status = "Project name cannot be empty.".into();
                } else if self.handle.store.projects.contains_key(&value) {
                    self.status = format!("Project `{value}` already exists.");
                } else {
                    self.handle
                        .store
                        .projects
                        .insert(value.clone(), Project::default());
                    self.save();
                    self.status = format!("Created project `{value}`.");
                }
            }
            InputAction::NewEnv => {
                if let Some(project) = self.current_project_name() {
                    if value.is_empty() {
                        self.status = "Environment name cannot be empty.".into();
                    } else if let Some(p) = self.handle.store.project_mut(&project) {
                        if p.environments.contains_key(&value) {
                            self.status = format!("Environment `{value}` already exists.");
                        } else {
                            p.environments.insert(value.clone(), Environment::default());
                            if p.default_env.is_none() {
                                p.default_env = Some(value.clone());
                            }
                            self.save();
                            self.status = format!("Created environment `{value}`.");
                        }
                    }
                }
            }
            InputAction::NewSecret => {
                let Some((key, val)) = value.split_once('=') else {
                    self.status = "Use KEY=VALUE format.".into();
                    return Ok(());
                };
                let key = key.trim().to_string();
                let val = val.trim().to_string();
                if key.is_empty() {
                    self.status = "Key cannot be empty.".into();
                    return Ok(());
                }
                if let Some(e) = self.current_env_mut() {
                    e.values.insert(key.clone(), val);
                    self.save();
                    self.status = format!("Set `{key}`.");
                }
            }
            InputAction::EditSecret { key } => {
                if let Some(e) = self.current_env_mut() {
                    e.values.insert(key.clone(), state.buffer);
                    self.save();
                    self.status = format!("Updated `{key}`.");
                }
            }
            InputAction::DuplicateEnv { from } => {
                if let Some(project) = self.current_project_name() {
                    if value.is_empty() {
                        self.status = "Name cannot be empty.".into();
                    } else if let Some(p) = self.handle.store.project_mut(&project) {
                        if p.environments.contains_key(&value) {
                            self.status = format!("Environment `{value}` already exists.");
                        } else if let Some(src) = p.environments.get(&from).cloned() {
                            p.environments.insert(value.clone(), src);
                            self.save();
                            self.status = format!("Duplicated `{from}` to `{value}`.");
                        }
                    }
                }
            }
            InputAction::Import => {
                self.do_import(&value);
            }
            InputAction::Export => {
                self.do_export(&value);
            }
            InputAction::SetFolder => {
                self.do_set_folder(&value);
            }
        }
        self.clamp_indices();
        Ok(())
    }

    fn current_env_mut(&mut self) -> Option<&mut Environment> {
        let project = self.current_project_name()?;
        let env = self.current_env_name()?;
        self.handle
            .store
            .project_mut(&project)?
            .environments
            .get_mut(&env)
    }

    fn do_import(&mut self, path: &str) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("Import failed: {e}");
                return;
            }
        };
        let parsed = envfile::parse(&text);
        let count = parsed.len();
        if let Some(e) = self.current_env_mut() {
            for (k, v) in parsed {
                e.values.insert(k, v);
            }
            self.save();
            self.status = format!("Imported {count} secrets from {path}.");
        }
    }

    fn do_export(&mut self, path: &str) {
        let (Some(project), Some(env)) = (self.current_project_name(), self.current_env_name())
        else {
            return;
        };
        let mut resolved = indexmap::IndexMap::new();
        let keys: Vec<String> = self
            .current_env()
            .map(|e| e.values.keys().cloned().collect())
            .unwrap_or_default();
        for key in keys {
            match resolve::resolve_at(&self.handle.store, &project, &env, &key) {
                Ok(v) => {
                    resolved.insert(key, v);
                }
                Err(e) => {
                    self.status = format!("Export failed: {e}");
                    return;
                }
            }
        }
        let output = envfile::serialize(&resolved);
        match std::fs::write(path, output) {
            Ok(()) => {
                self.status = format!("Exported {} secrets to {path}.", resolved.len());
            }
            Err(e) => self.status = format!("Export failed: {e}"),
        }
    }

    fn do_set_folder(&mut self, raw: &str) {
        let Some(project) = self.current_project_name() else {
            return;
        };
        let folder = if raw.is_empty() {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        } else {
            let p = std::path::Path::new(raw);
            let abs = if p.is_absolute() {
                p.to_path_buf()
            } else {
                std::env::current_dir().unwrap_or_default().join(p)
            };
            Some(
                std::fs::canonicalize(&abs)
                    .unwrap_or(abs)
                    .to_string_lossy()
                    .into_owned(),
            )
        };
        if let Some(p) = self.handle.store.project_mut(&project) {
            p.folder = folder.clone();
        }
        self.save();
        self.status = match folder {
            Some(f) => format!("Linked `{project}` to {f}."),
            None => format!("Cleared folder for `{project}`."),
        };
    }

    /// Open the whole current environment in the user's `$EDITOR` as a `.env`
    /// document. On save it is parsed back and replaces the environment's
    /// contents. The terminal is suspended for the duration of the editor.
    fn edit_env_in_editor<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        let (Some(project), Some(env)) = (self.current_project_name(), self.current_env_name())
        else {
            return Ok(());
        };
        let values = self
            .current_env()
            .map(|e| e.values.clone())
            .unwrap_or_default();

        // Write the current values to a temp file as a commented .env.
        let mut path = std::env::temp_dir();
        path.push(format!(
            "devsecrets-{}-{}-{}.env",
            sanitize(&project),
            sanitize(&env),
            std::process::id()
        ));
        let template = editor_template(&project, &env, &values);
        if let Err(e) = std::fs::write(&path, template) {
            self.status = format!("Could not create temp file: {e}");
            return Ok(());
        }

        // Suspend the TUI, run the editor, then restore.
        suspend_terminal()?;
        let launched = launch_editor(&path);
        resume_terminal()?;
        terminal.clear()?;

        match launched {
            Ok(true) => {}
            Ok(false) => {
                self.status = "Editor exited without saving; no changes made.".into();
                let _ = std::fs::remove_file(&path);
                return Ok(());
            }
            Err(e) => {
                self.status = format!("Could not launch editor: {e}");
                let _ = std::fs::remove_file(&path);
                return Ok(());
            }
        }

        let edited = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("Could not read edited file: {e}");
                let _ = std::fs::remove_file(&path);
                return Ok(());
            }
        };
        let _ = std::fs::remove_file(&path);

        let parsed = envfile::parse(&edited);
        let count = parsed.len();
        if let Some(e) = self.current_env_mut() {
            e.values = parsed;
        }
        self.save();
        self.clamp_indices();
        self.status = format!("Updated `{project}/{env}` ({count} secrets) from editor.");
        Ok(())
    }

    // --- inline editor -----------------------------------------------------

    fn on_editor_key(&mut self, key: KeyEvent) -> Result<()> {
        // Ctrl-S saves, Esc / Ctrl-C cancels.
        match (key.code, key.modifiers) {
            (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                if let Some(ed) = self.editor.take() {
                    self.apply_editor(ed);
                }
                return Ok(());
            }
            (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.editor = None;
                self.status = "Edit cancelled — no changes made.".into();
                return Ok(());
            }
            _ => {}
        }

        let Some(ed) = self.editor.as_mut() else {
            return Ok(());
        };
        match key.code {
            KeyCode::Enter => ed.insert_newline(),
            KeyCode::Backspace => ed.backspace(),
            KeyCode::Delete => ed.delete_forward(),
            KeyCode::Left => ed.move_left(),
            KeyCode::Right => ed.move_right(),
            KeyCode::Up => ed.move_up(),
            KeyCode::Down => ed.move_down(),
            KeyCode::Home => ed.cx = 0,
            KeyCode::End => ed.cx = ed.cur_len(),
            KeyCode::Tab => {
                ed.insert_char(' ');
                ed.insert_char(' ');
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                ed.insert_char(c)
            }
            _ => {}
        }
        Ok(())
    }

    fn apply_editor(&mut self, ed: EnvEditor) {
        let text = ed.lines.join("\n");
        let parsed = envfile::parse(&text);
        let count = parsed.len();
        if let Some(p) = self.handle.store.project_mut(&ed.project) {
            if let Some(e) = p.environments.get_mut(&ed.env) {
                e.values = parsed;
            }
        }
        self.save();
        self.clamp_indices();
        self.status = format!("Updated `{}/{}` ({count} secrets).", ed.project, ed.env);
    }

    // --- confirm modal -----------------------------------------------------

    fn on_confirm_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                if let Some(state) = self.confirm.take() {
                    self.apply_confirm(state.action);
                }
            }
            _ => {
                self.confirm = None;
            }
        }
        Ok(())
    }

    fn apply_confirm(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::DeleteProject(name) => {
                self.handle.store.projects.shift_remove(&name);
                self.status = format!("Deleted project `{name}`.");
            }
            ConfirmAction::DeleteEnv { project, env } => {
                if let Some(p) = self.handle.store.project_mut(&project) {
                    p.environments.shift_remove(&env);
                    if p.default_env.as_deref() == Some(&env) {
                        p.default_env = p.environments.keys().next().cloned();
                    }
                }
                self.status = format!("Deleted environment `{project}/{env}`.");
            }
            ConfirmAction::DeleteSecret { project, env, key } => {
                if let Some(p) = self.handle.store.project_mut(&project) {
                    if let Some(e) = p.environments.get_mut(&env) {
                        e.values.shift_remove(&key);
                    }
                }
                self.status = format!("Deleted secret `{key}`.");
            }
        }
        self.save();
        self.clamp_indices();
    }

    // --- rendering ---------------------------------------------------------

    fn draw(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(1),
            ])
            .split(f.area());

        self.draw_title(f, chunks[0]);
        self.draw_panes(f, chunks[1]);
        self.draw_status(f, chunks[2]);

        if self.show_help {
            self.draw_help(f);
        }
        if let Some(input) = &self.input {
            self.draw_input(f, input);
        }
        if let Some(confirm) = &self.confirm {
            self.draw_confirm(f, confirm);
        }
        if let Some(editor) = &self.editor {
            self.draw_editor(f, editor);
        }
    }

    fn draw_editor(&self, f: &mut Frame, ed: &EnvEditor) {
        let area = f.area().inner(Margin {
            horizontal: 4,
            vertical: 2,
        });
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(format!(" Edit {}/{} (.env) ", ed.project, ed.env))
            .title_bottom(" Ctrl-S: save & apply   Esc: cancel ");
        let inner = block.inner(area);
        f.render_widget(block, area);

        let height = inner.height as usize;
        let width = inner.width.max(1) as usize;
        // Scroll so the cursor stays visible.
        let vscroll = ed.cy.saturating_sub(height.saturating_sub(1));
        let hscroll = ed.cx.saturating_sub(width.saturating_sub(1));

        let mut rendered: Vec<Line> = Vec::new();
        let last = (vscroll + height).min(ed.lines.len());
        for line in &ed.lines[vscroll..last] {
            let slice: String = line.chars().skip(hscroll).take(width).collect();
            // Comment lines are dimmed for readability.
            let style = if slice.trim_start().starts_with('#') {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            rendered.push(Line::from(Span::styled(slice, style)));
        }
        f.render_widget(Paragraph::new(rendered), inner);

        let cursor_x = inner.x + (ed.cx - hscroll) as u16;
        let cursor_y = inner.y + (ed.cy - vscroll) as u16;
        f.set_cursor_position((cursor_x, cursor_y));
    }

    fn draw_title(&self, f: &mut Frame, area: Rect) {
        let title = Line::from(vec![
            Span::styled(
                " dev-secrets ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("store: {}", self.handle.path.display()),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        f.render_widget(Paragraph::new(title), area);
    }

    fn draw_panes(&self, f: &mut Frame, area: Rect) {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(28),
                Constraint::Percentage(28),
                Constraint::Percentage(44),
            ])
            .split(area);

        // Projects pane
        let proj_items: Vec<ListItem> = self
            .handle
            .store
            .projects
            .iter()
            .map(|(name, p)| ListItem::new(format!("{name} ({})", p.environments.len())))
            .collect();
        self.render_list(
            f,
            cols[0],
            "Projects",
            proj_items,
            self.proj_idx,
            self.focus == Focus::Projects,
        );

        // Environments pane
        let env_items: Vec<ListItem> = self
            .current_project()
            .map(|p| {
                p.environments
                    .iter()
                    .map(|(name, e)| {
                        let default = if p.default_env.as_deref() == Some(name) {
                            " *"
                        } else {
                            ""
                        };
                        ListItem::new(format!("{name}{default} ({})", e.values.len()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.render_list(
            f,
            cols[1],
            "Environments",
            env_items,
            self.env_idx,
            self.focus == Focus::Envs,
        );

        // Secrets pane
        let secret_items: Vec<ListItem> = self
            .current_env()
            .map(|e| {
                e.values
                    .iter()
                    .map(|(k, v)| {
                        let shown = if self.reveal { v.clone() } else { mask(v) };
                        ListItem::new(Line::from(vec![
                            Span::styled(k.clone(), Style::default().fg(Color::Yellow)),
                            Span::raw(" = "),
                            Span::raw(shown),
                        ]))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let secret_title = if self.reveal {
            "Secrets [shown]"
        } else {
            "Secrets [hidden]"
        };
        self.render_list(
            f,
            cols[2],
            secret_title,
            secret_items,
            self.secret_idx,
            self.focus == Focus::Secrets,
        );
    }

    fn render_list(
        &self,
        f: &mut Frame,
        area: Rect,
        title: &str,
        items: Vec<ListItem>,
        selected: usize,
        focused: bool,
    ) {
        let border_style = if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let highlight = if focused {
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::DIM)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(format!(" {title} "));
        let list = List::new(items)
            .block(block)
            .highlight_style(highlight)
            .highlight_symbol("› ");
        let mut state = ListState::default();
        state.select(Some(selected));
        f.render_stateful_widget(list, area, &mut state);
    }

    fn draw_status(&self, f: &mut Frame, area: Rect) {
        let keys =
            "n:new e:edit a:edit-env E:$EDITOR d:del y:dup i:import x:export D:default f:folder s:show ?:help q:quit";
        let line = Line::from(vec![
            Span::styled(
                format!(" {} ", self.status),
                Style::default().fg(Color::Green),
            ),
            Span::raw("  "),
            Span::styled(keys, Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(Paragraph::new(line).wrap(Wrap { trim: true }), area);
    }

    fn draw_input(&self, f: &mut Frame, input: &InputState) {
        let area = centered_rect(60, 7, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(format!(" {} ", input.title));
        let inner = Paragraph::new(vec![
            Line::from(Span::styled(
                &input.prompt,
                Style::default().fg(Color::Gray),
            )),
            Line::from(Span::styled(
                format!("{}_", input.buffer),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "Enter: confirm   Esc: cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(block)
        .wrap(Wrap { trim: false });
        f.render_widget(inner, area);
    }

    fn draw_confirm(&self, f: &mut Frame, confirm: &ConfirmState) {
        let area = centered_rect(60, 6, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .title(" Confirm ");
        let inner = Paragraph::new(vec![
            Line::from(Span::raw(&confirm.message)),
            Line::from(""),
            Line::from(Span::styled(
                "y: yes    any other key: cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(block)
        .wrap(Wrap { trim: true });
        f.render_widget(inner, area);
    }

    fn draw_help(&self, f: &mut Frame) {
        let area = centered_rect(70, 20, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" Help — press any key to close ");
        let lines = vec![
            Line::from("Navigation"),
            Line::from("  ↑/k ↓/j     move selection"),
            Line::from("  →/l Enter   drill into Projects → Envs → Secrets"),
            Line::from("  ←/h         go back   Tab: cycle panes"),
            Line::from(""),
            Line::from("Actions (context = focused pane)"),
            Line::from("  n  new project / env / secret"),
            Line::from("  e  edit secret (on Secrets) / whole env inline (otherwise)"),
            Line::from("  a  edit the whole environment inline (multi-line .env editor)"),
            Line::from("  E  edit the whole environment in $EDITOR (as a .env)"),
            Line::from("  d  delete focused item (asks to confirm)"),
            Line::from("  y  duplicate environment"),
            Line::from("  i  import a .env file into the selected env"),
            Line::from("  x  export the selected env to a .env file"),
            Line::from("  D  set selected env as the project default"),
            Line::from("  f  assign a working folder to the project"),
            Line::from("  s  toggle showing/hiding secret values"),
            Line::from(""),
            Line::from("References: a value like ${proj.env.KEY} is resolved on export."),
            Line::from("Quit: q or Ctrl-C"),
        ];
        f.render_widget(Paragraph::new(lines).block(block), area);
    }
}

fn mask(value: &str) -> String {
    let len = value.chars().count();
    if len == 0 {
        "(empty)".into()
    } else if len <= 4 {
        "•".repeat(len)
    } else {
        let head: String = value.chars().take(2).collect();
        format!("{head}{}", "•".repeat(6))
    }
}

/// Build the `.env` document shown to the user in their editor.
fn editor_template(project: &str, env: &str, values: &indexmap::IndexMap<String, String>) -> String {
    let mut out = String::new();
    out.push_str(&format!("# dev-secrets — editing {project}/{env}\n"));
    out.push_str("# Edit as a normal .env file: KEY=VALUE per line.\n");
    out.push_str("# Lines you remove are deleted; blank lines and # comments are ignored.\n");
    out.push_str("# References like ${project.env.KEY} are kept as-is and resolved on export.\n");
    out.push_str("# Save and quit to apply, or quit without saving to cancel.\n\n");
    out.push_str(&envfile::serialize(values));
    out
}

/// Byte offset of the `n`-th character in `s` (or `s.len()` if past the end).
fn char_byte_idx(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map(|(b, _)| b).unwrap_or(s.len())
}

/// Replace characters that are awkward in a temp file name.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

/// Pick the editor command: `$VISUAL`, then `$EDITOR`, then a sane default.
fn editor_command() -> String {
    std::env::var("VISUAL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("EDITOR").ok().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| if cfg!(windows) { "notepad".into() } else { "vi".into() })
}

/// Spawn the editor on `path` and wait. Returns `Ok(true)` if the editor
/// exited successfully (the user saved/closed normally).
fn launch_editor(path: &std::path::Path) -> Result<bool> {
    let command = editor_command();
    // The editor variable may include arguments, e.g. `code --wait`.
    let mut parts = command.split_whitespace();
    let program = parts.next().unwrap_or("vi");
    let args: Vec<&str> = parts.collect();

    let status = std::process::Command::new(program)
        .args(&args)
        .arg(path)
        .status()?;
    Ok(status.success())
}

/// Leave raw mode + the alternate screen so an external program can use the
/// terminal normally.
fn suspend_terminal() -> Result<()> {
    use ratatui::crossterm::execute;
    use ratatui::crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};
    disable_raw_mode()?;
    execute!(std::io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

/// Re-enter raw mode + the alternate screen after an external program exits.
fn resume_terminal() -> Result<()> {
    use ratatui::crossterm::execute;
    use ratatui::crossterm::terminal::{enable_raw_mode, EnterAlternateScreen};
    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    Ok(())
}

/// Centred rectangle `width` percent wide and `height` rows tall.
fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    #[test]
    fn editor_template_roundtrips() {
        let mut values = IndexMap::new();
        values.insert("DB_HOST".to_string(), "localhost".to_string());
        values.insert("TOKEN".to_string(), "with space".to_string());
        values.insert("API_URL".to_string(), "http://${api.dev.DB_HOST}:5432".to_string());

        let doc = editor_template("api", "dev", &values);
        // Header lines are comments and must be ignored on parse.
        let parsed = envfile::parse(&doc);
        assert_eq!(parsed, values);
    }

    #[test]
    fn sanitize_replaces_specials() {
        assert_eq!(sanitize("my project/1"), "my_project_1");
    }
}
