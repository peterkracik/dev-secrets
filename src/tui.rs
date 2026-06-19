//! The interactive, Telescope-style terminal UI.
//!
//! A centered, bounded floating window presents one "picker" at a time as you
//! drill through Projects → Environments → Secrets. Each picker has a fuzzy
//! filter prompt, a results list with match highlighting, and a live preview
//! pane. Every mutating action is a single keypress and persists immediately.

use std::collections::HashSet;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};
use ratatui::{Frame, Terminal};

use crate::cli::Format;
use crate::fuzzy;
use crate::meta::{self, Meta};
use crate::model::{Environment, Kind, Project};
use crate::store::StoreHandle;
use crate::{envfile, resolve};

/// Accent colour used throughout the UI.
const ACCENT: Color = Color::Cyan;
/// Maximum size of the floating window; it never fills the whole terminal.
const MAX_WIDTH: u16 = 143;
const MAX_HEIGHT: u16 = 34;

/// Which screen currently has keyboard focus. Projects and environments share
/// one combined tree screen; secrets are a second screen you drill into.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Tree,
    Secrets,
}

/// A visible row in the combined Projects/Environments tree.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Project(usize),
    Env(usize, usize),
}

/// What a submitted text prompt should do.
enum InputAction {
    NewProject,
    NewEnv,
    EditSecret { key: String },
    DuplicateEnv { from: String },
    Import,
    Export,
}

struct InputState {
    title: String,
    prompt: String,
    buffer: String,
    action: InputAction,
}

/// Which field of the new-secret form is active.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KvField {
    Key,
    Value,
    Type,
}

impl KvField {
    fn next(self) -> KvField {
        match self {
            KvField::Key => KvField::Value,
            KvField::Value => KvField::Type,
            KvField::Type => KvField::Key,
        }
    }
    fn prev(self) -> KvField {
        match self {
            KvField::Key => KvField::Type,
            KvField::Value => KvField::Key,
            KvField::Type => KvField::Value,
        }
    }
}

/// Form for adding a secret — separate Key, Value, and Type fields so a value
/// can be pasted on its own and its type chosen.
struct KvInputState {
    key: String,
    value: String,
    kind: Kind,
    active: KvField,
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

/// How an import applies the file to the environment.
#[derive(Clone, Copy)]
enum ImportMode {
    /// Add new keys; confirm each changed key individually.
    Merge,
    /// Add new keys and overwrite every changed key without asking.
    OverwriteAll,
    /// Clear the environment first, then load only the file's keys.
    Replace,
}

/// A running import that asks per changed key whether to overwrite it. New
/// keys are added up front; only conflicts (existing key, different value) are
/// queued here.
struct ImportSession {
    source: String,
    /// Conflicts still to decide: (key, old_value, new_value).
    conflicts: Vec<(String, String, String)>,
    pos: usize,
    added: usize,
    updated: usize,
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
    /// Folder → (project, env) assignments, for auto-select and the `f` action.
    meta: Meta,
    focus: Focus,
    proj_idx: usize,
    env_idx: usize,
    secret_idx: usize,
    /// Selected row in the combined tree screen.
    tree_sel: usize,
    /// Names of projects whose environments are collapsed in the tree.
    collapsed: HashSet<String>,
    reveal: bool,
    show_help: bool,
    /// Selected row in the reference-autocomplete popup.
    ac_sel: usize,
    /// True when the user dismissed the autocomplete popup with Esc.
    ac_dismissed: bool,
    /// Current fuzzy filter for the active picker.
    query: String,
    /// Whether keystrokes are being typed into the filter prompt.
    searching: bool,
    status: String,
    input: Option<InputState>,
    /// Active two-field new-secret form, if any.
    kv_input: Option<KvInputState>,
    confirm: Option<ConfirmState>,
    /// Path awaiting a merge/overwrite/replace choice for import, if any.
    import_choice: Option<String>,
    /// Active per-key import confirmation session, if any.
    import_session: Option<ImportSession>,
    /// Output path awaiting an export-format choice, if any.
    export_choice: Option<String>,
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
    let meta = meta::load().unwrap_or_default();
    let mut app = App::new(handle, meta);
    app.select_assigned();
    let mut terminal = ratatui::init();
    let result = app.run_loop(&mut terminal);
    ratatui::restore();
    result
}

impl App {
    fn new(handle: StoreHandle, meta: Meta) -> Self {
        App {
            handle,
            meta,
            focus: Focus::Tree,
            proj_idx: 0,
            env_idx: 0,
            secret_idx: 0,
            tree_sel: 0,
            collapsed: HashSet::new(),
            reveal: false,
            show_help: false,
            ac_sel: 0,
            ac_dismissed: false,
            query: String::new(),
            searching: false,
            status: "Welcome to dev-secrets. Press / to search, ? for help.".to_string(),
            input: None,
            kv_input: None,
            confirm: None,
            import_choice: None,
            import_session: None,
            export_choice: None,
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

    /// If the current folder is assigned to a project/env, jump straight to it.
    fn select_assigned(&mut self) {
        let Ok(dir) = meta::current_dir() else {
            return;
        };
        let Some(a) = self.meta.get(&dir) else {
            return;
        };
        let (project, env) = (a.project.clone(), a.env.clone());
        let Some(pi) = self.handle.store.projects.get_index_of(&project) else {
            return;
        };
        self.proj_idx = pi;
        let ei = self
            .handle
            .store
            .projects
            .get_index(pi)
            .and_then(|(_, p)| p.environments.get_index_of(&env));
        if let Some(ei) = ei {
            self.env_idx = ei;
            self.secret_idx = 0;
            self.focus = Focus::Secrets;
            self.status = format!("This folder is assigned to {project}/{env}.");
        } else {
            self.focus = Focus::Tree;
        }
        self.tree_sel = self.tree_row_for(self.proj_idx, ei);
    }

    /// Index of the tree row for a project (and optional env), best-effort.
    fn tree_row_for(&self, pi: usize, ei: Option<usize>) -> usize {
        let rows = self.tree_rows();
        rows.iter()
            .position(|r| match (r, ei) {
                (Row::Env(p, e), Some(ei)) => *p == pi && *e == ei,
                (Row::Project(p), None) => *p == pi,
                _ => false,
            })
            .or_else(|| {
                rows.iter()
                    .position(|r| matches!(r, Row::Project(p) if *p == pi))
            })
            .unwrap_or(0)
    }

    // --- key handling ------------------------------------------------------

    fn on_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.editor.is_some() {
            return self.on_editor_key(key);
        }
        if self.kv_input.is_some() {
            return self.on_kv_key(key);
        }
        if self.input.is_some() {
            return self.on_input_key(key);
        }
        if self.confirm.is_some() {
            return self.on_confirm_key(key);
        }
        if self.import_choice.is_some() {
            return self.on_import_choice_key(key);
        }
        if self.import_session.is_some() {
            return self.on_import_session_key(key);
        }
        if self.export_choice.is_some() {
            return self.on_export_choice_key(key);
        }
        if self.show_help {
            self.show_help = false;
            return Ok(());
        }
        if self.searching {
            return self.on_search_key(key);
        }
        self.on_nav_key(key)
    }

    /// Keys while the fuzzy filter prompt is focused.
    fn on_search_key(&mut self, key: KeyEvent) -> Result<()> {
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => self.should_quit = true,
            (KeyCode::Esc, _) => {
                self.searching = false;
                self.query.clear();
                self.sync_selection();
            }
            (KeyCode::Enter, _) => {
                // Accept the filter and open the highlighted item.
                self.searching = false;
                self.on_enter();
            }
            (KeyCode::Backspace, _) => {
                self.query.pop();
                self.sync_selection();
            }
            (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                self.move_selection(1)
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                self.move_selection(-1)
            }
            (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
                self.query.push(c);
                self.sync_selection();
            }
            _ => {}
        }
        Ok(())
    }

    fn on_nav_key(&mut self, key: KeyEvent) -> Result<()> {
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('q'), _) => {
                self.should_quit = true;
            }
            (KeyCode::Char('/'), _) => self.searching = true,
            (KeyCode::Char('?'), _) => self.show_help = true,
            (KeyCode::Char('s'), _) => self.reveal = !self.reveal,
            (KeyCode::Down, _) | (KeyCode::Char('j'), _) => self.move_selection(1),
            (KeyCode::Up, _) | (KeyCode::Char('k'), _) => self.move_selection(-1),
            (KeyCode::Right, _) | (KeyCode::Char('l'), _) => self.focus_deeper(),
            (KeyCode::Left, _) | (KeyCode::Char('h'), _) | (KeyCode::Esc, _) => {
                self.focus_shallower()
            }
            (KeyCode::Enter, _) => self.on_enter(),
            (KeyCode::Char('n'), _) => self.start_new(),
            (KeyCode::Char('p'), _) => self.start_new_project(),
            (KeyCode::Char('e'), _) => self.start_edit(),
            (KeyCode::Char('a'), _) => self.open_inline_editor(),
            (KeyCode::Char('E'), _) => self.request_edit_env(),
            (KeyCode::Char('c'), _) => self.copy_context(),
            (KeyCode::Char('C'), _) => self.copy_env(),
            (KeyCode::Char('d'), _) => self.start_delete(),
            (KeyCode::Char('y'), _) => self.start_duplicate(),
            (KeyCode::Char('i'), _) => self.start_import(),
            (KeyCode::Char('x'), _) => self.start_export(),
            (KeyCode::Char('D'), _) => self.set_default_env(),
            (KeyCode::Char('f'), _) => self.assign_folder(),
            (KeyCode::Char('g'), _) => self.goto_reference(),
            _ => {}
        }
        Ok(())
    }

    fn reset_query(&mut self) {
        self.query.clear();
        self.searching = false;
    }

    fn focus_deeper(&mut self) {
        match self.focus {
            Focus::Tree => match self.selected_row() {
                Some(Row::Env(pi, ei)) => {
                    self.proj_idx = pi;
                    self.env_idx = ei;
                    self.secret_idx = 0;
                    self.focus = Focus::Secrets;
                    self.reset_query();
                }
                Some(Row::Project(pi)) => self.set_collapsed(pi, false),
                None => {}
            },
            Focus::Secrets => {}
        }
    }

    fn focus_shallower(&mut self) {
        match self.focus {
            Focus::Secrets => {
                self.focus = Focus::Tree;
                self.reset_query();
                self.tree_sel = self.tree_row_for(self.proj_idx, Some(self.env_idx));
            }
            Focus::Tree => match self.selected_row() {
                Some(Row::Env(pi, _)) => {
                    // Jump to the parent project row.
                    self.tree_sel = self.tree_row_for(pi, None);
                    self.sync_from_tree();
                }
                Some(Row::Project(pi)) => self.set_collapsed(pi, true),
                None => {}
            },
        }
    }

    fn on_enter(&mut self) {
        match self.focus {
            Focus::Tree => match self.selected_row() {
                Some(Row::Env(..)) => self.focus_deeper(),
                Some(Row::Project(pi)) => {
                    let collapsed = self.is_collapsed(pi);
                    self.set_collapsed(pi, !collapsed);
                }
                None => {}
            },
            Focus::Secrets => self.start_edit_secret(),
        }
    }

    // --- tree (combined Projects + Environments) --------------------------

    fn project_name_at(&self, pi: usize) -> Option<String> {
        self.handle
            .store
            .projects
            .get_index(pi)
            .map(|(k, _)| k.clone())
    }

    fn is_collapsed(&self, pi: usize) -> bool {
        self.project_name_at(pi)
            .map(|n| self.collapsed.contains(&n))
            .unwrap_or(false)
    }

    fn set_collapsed(&mut self, pi: usize, collapsed: bool) {
        if let Some(name) = self.project_name_at(pi) {
            if collapsed {
                self.collapsed.insert(name);
            } else {
                self.collapsed.remove(&name);
            }
            // Selection may now point past the end; clamp + resync.
            self.clamp_tree_sel();
            self.sync_from_tree();
        }
    }

    /// The visible rows of the tree, honouring the fuzzy filter and (when not
    /// searching) the per-project collapse state.
    fn tree_rows(&self) -> Vec<Row> {
        let q = &self.query;
        let mut rows = Vec::new();
        for (pi, (pname, proj)) in self.handle.store.projects.iter().enumerate() {
            let p_match = q.is_empty() || fuzzy::score(q, pname).is_some();
            let env_hits: Vec<usize> = proj
                .environments
                .keys()
                .enumerate()
                .filter(|(_, ename)| q.is_empty() || fuzzy::score(q, ename).is_some())
                .map(|(ei, _)| ei)
                .collect();

            // Include the project if it matches, or any of its envs match.
            if !p_match && env_hits.is_empty() {
                continue;
            }
            rows.push(Row::Project(pi));

            // Show envs: respect collapse only when not searching.
            let show = if q.is_empty() {
                !self.collapsed.contains(pname)
            } else {
                true
            };
            if show {
                // If the project itself matched, keep all its envs; otherwise
                // only the matching ones.
                let envs: Vec<usize> = if p_match {
                    (0..proj.environments.len()).collect()
                } else {
                    env_hits
                };
                for ei in envs {
                    rows.push(Row::Env(pi, ei));
                }
            }
        }
        rows
    }

    fn selected_row(&self) -> Option<Row> {
        self.tree_rows().get(self.tree_sel).copied()
    }

    /// The (project, env) indices if an environment row is selected.
    fn selected_env(&self) -> Option<(usize, usize)> {
        match self.selected_row()? {
            Row::Env(pi, ei) => Some((pi, ei)),
            Row::Project(_) => None,
        }
    }

    fn selected_project_index(&self) -> Option<usize> {
        match self.selected_row()? {
            Row::Project(pi) | Row::Env(pi, _) => Some(pi),
        }
    }

    fn clamp_tree_sel(&mut self) {
        let n = self.tree_rows().len();
        if self.tree_sel >= n {
            self.tree_sel = n.saturating_sub(1);
        }
    }

    /// Sync proj_idx/env_idx from the selected tree row so the rest of the app
    /// (preview, actions) sees a consistent current project/env.
    fn sync_from_tree(&mut self) {
        match self.selected_row() {
            Some(Row::Project(pi)) => {
                self.proj_idx = pi;
                self.env_idx = 0;
            }
            Some(Row::Env(pi, ei)) => {
                self.proj_idx = pi;
                self.env_idx = ei;
            }
            None => {}
        }
    }

    // --- secrets-level selection helpers ----------------------------------

    /// Secret keys in the current environment (for fuzzy filtering).
    fn secret_labels(&self) -> Vec<String> {
        self.current_env()
            .map(|e| e.values.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Real indices of secrets matching the query, best match first.
    fn filtered_indices(&self) -> Vec<usize> {
        let labels = self.secret_labels();
        if self.query.is_empty() {
            return (0..labels.len()).collect();
        }
        let mut scored: Vec<(i32, usize)> = labels
            .iter()
            .enumerate()
            .filter_map(|(i, label)| fuzzy::score(&self.query, label).map(|(s, _)| (s, i)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, i)| i).collect()
    }

    /// Keep the selection on a visible item after the query changes.
    fn sync_selection(&mut self) {
        match self.focus {
            Focus::Tree => {
                self.clamp_tree_sel();
                self.sync_from_tree();
            }
            Focus::Secrets => {
                let f = self.filtered_indices();
                if f.is_empty() {
                    return;
                }
                if !f.contains(&self.secret_idx) {
                    self.secret_idx = f[0];
                }
            }
        }
    }

    fn move_selection(&mut self, delta: i32) {
        match self.focus {
            Focus::Tree => {
                let n = self.tree_rows().len();
                if n == 0 {
                    return;
                }
                self.tree_sel = (self.tree_sel as i32 + delta).rem_euclid(n as i32) as usize;
                self.sync_from_tree();
            }
            Focus::Secrets => {
                let f = self.filtered_indices();
                if f.is_empty() {
                    return;
                }
                let pos = f.iter().position(|&i| i == self.secret_idx).unwrap_or(0);
                let new = (pos as i32 + delta).rem_euclid(f.len() as i32) as usize;
                self.secret_idx = f[new];
            }
        }
    }

    // --- action starters ---------------------------------------------------

    /// `n`: on the tree, add an environment to the selected project; on the
    /// secrets screen, add a secret.
    fn start_new(&mut self) {
        match self.focus {
            Focus::Tree => self.start_new_env(),
            Focus::Secrets => {
                if self.current_env().is_none() {
                    self.status = "Create an environment first.".into();
                    return;
                }
                self.ac_reset();
                self.kv_input = Some(KvInputState {
                    key: String::new(),
                    value: String::new(),
                    kind: Kind::Text,
                    active: KvField::Key,
                });
            }
        }
    }

    /// `p`: create a new project.
    fn start_new_project(&mut self) {
        self.input = Some(InputState {
            title: "New project".into(),
            prompt: "Project name:".into(),
            buffer: String::new(),
            action: InputAction::NewProject,
        });
    }

    /// Create a new environment under the selected project.
    fn start_new_env(&mut self) {
        let Some(pi) = self.selected_project_index() else {
            self.status = "No project yet — press p to create one.".into();
            return;
        };
        self.proj_idx = pi;
        self.input = Some(InputState {
            title: "New environment".into(),
            prompt: "Environment name:".into(),
            buffer: String::new(),
            action: InputAction::NewEnv,
        });
    }

    /// Ensure an environment is selected, syncing proj_idx/env_idx to it.
    /// Returns false (and sets a status) when none is selected.
    fn require_selected_env(&mut self) -> bool {
        match self.focus {
            Focus::Secrets => self.current_env().is_some(),
            Focus::Tree => {
                if let Some((pi, ei)) = self.selected_env() {
                    self.proj_idx = pi;
                    self.env_idx = ei;
                    true
                } else {
                    self.status = "Select an environment (not a project) first.".into();
                    false
                }
            }
        }
    }

    /// Context-aware edit: a single secret on the secrets screen, otherwise the
    /// selected environment in the inline editor.
    fn start_edit(&mut self) {
        if self.focus == Focus::Secrets && self.current_secret_key().is_some() {
            self.start_edit_secret();
        } else if self.require_selected_env() {
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

    /// Context-aware copy: the selected secret's value on the secrets screen,
    /// otherwise the selected environment as a `.env` document.
    fn copy_context(&mut self) {
        if self.focus == Focus::Secrets && self.current_secret_key().is_some() {
            self.copy_secret();
        } else if self.require_selected_env() {
            self.copy_env();
        }
    }

    /// Copy the selected secret's resolved value to the clipboard.
    fn copy_secret(&mut self) {
        let (Some(project), Some(env), Some(key)) = (
            self.current_project_name(),
            self.current_env_name(),
            self.current_secret_key(),
        ) else {
            self.status = "No secret selected.".into();
            return;
        };
        match resolve::resolve_at(&self.handle.store, &project, &env, &key) {
            Ok(value) => self.copy_text(&value, &format!("value of `{key}`")),
            Err(e) => self.status = format!("Copy failed: {e}"),
        }
    }

    /// Copy the whole current environment as a resolved `.env` document.
    fn copy_env(&mut self) {
        let (Some(project), Some(env)) = (self.current_project_name(), self.current_env_name())
        else {
            self.status = "Select an environment to copy.".into();
            return;
        };
        let keys: Vec<String> = self
            .current_env()
            .map(|e| e.values.keys().cloned().collect())
            .unwrap_or_default();
        let mut resolved = indexmap::IndexMap::new();
        for key in keys {
            match resolve::resolve_at(&self.handle.store, &project, &env, &key) {
                Ok(v) => {
                    resolved.insert(key, v);
                }
                Err(e) => {
                    self.status = format!("Copy failed: {e}");
                    return;
                }
            }
        }
        let text = envfile::serialize(&resolved);
        self.copy_text(&text, &format!("{}/{} as .env", project, env));
    }

    fn copy_text(&mut self, text: &str, what: &str) {
        match crate::clip::copy(text) {
            Ok(method) => self.status = format!("Copied {what} to clipboard ({method})."),
            Err(e) => self.status = format!("Copy failed: {e}"),
        }
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
        self.ac_reset();
        self.input = Some(InputState {
            title: format!("Edit {key}"),
            prompt: "Value:".into(),
            buffer: current,
            action: InputAction::EditSecret { key },
        });
    }

    /// Jump to the secret that the current secret's first `${…}` reference
    /// points at.
    fn goto_reference(&mut self) {
        if self.focus != Focus::Secrets {
            return;
        }
        let (Some(project), Some(env), Some(key)) = (
            self.current_project_name(),
            self.current_env_name(),
            self.current_secret_key(),
        ) else {
            return;
        };
        let raw = self
            .handle
            .store
            .value(&project, &env, &key)
            .cloned()
            .unwrap_or_default();
        let Some(inner) = first_reference(&raw) else {
            self.status = "This secret isn't a reference.".into();
            return;
        };
        // Resolve the reference relative to the current project/env.
        let parts: Vec<&str> = inner.split('.').collect();
        let (tp, te, tk) = match parts.as_slice() {
            [k] => (project.clone(), env.clone(), (*k).to_string()),
            [e, k] => (project.clone(), (*e).to_string(), (*k).to_string()),
            [p, e, k] => ((*p).to_string(), (*e).to_string(), (*k).to_string()),
            _ => {
                self.status = format!("invalid reference ${{{inner}}}");
                return;
            }
        };
        let target = self
            .handle
            .store
            .projects
            .get_index_of(tp.as_str())
            .and_then(|pi| {
                let (_, p) = self.handle.store.projects.get_index(pi)?;
                let ei = p.environments.get_index_of(te.as_str())?;
                let (_, e) = p.environments.get_index(ei)?;
                let si = e.values.get_index_of(tk.as_str())?;
                Some((pi, ei, si))
            });
        match target {
            Some((pi, ei, si)) => {
                self.proj_idx = pi;
                self.env_idx = ei;
                self.secret_idx = si;
                self.focus = Focus::Secrets;
                self.reset_query();
                self.status = format!("→ {tp}/{te}/{tk}");
            }
            None => self.status = format!("reference target `{tp}.{te}.{tk}` not found"),
        }
    }

    /// Open the selected environment in `$EDITOR`. The actual launch happens
    /// back in the main loop, which owns the terminal.
    fn request_edit_env(&mut self) {
        if self.require_selected_env() {
            self.pending_editor = true;
        }
    }

    fn start_delete(&mut self) {
        match self.focus {
            Focus::Tree => match self.selected_row() {
                Some(Row::Project(pi)) => {
                    if let Some(name) = self.project_name_at(pi) {
                        self.confirm = Some(ConfirmState {
                            message: format!("Delete project `{name}` and all its environments?"),
                            action: ConfirmAction::DeleteProject(name),
                        });
                    }
                }
                Some(Row::Env(pi, ei)) => {
                    let project = self.project_name_at(pi).unwrap_or_default();
                    let env = self
                        .handle
                        .store
                        .projects
                        .get_index(pi)
                        .and_then(|(_, p)| p.environments.get_index(ei))
                        .map(|(n, _)| n.clone())
                        .unwrap_or_default();
                    self.confirm = Some(ConfirmState {
                        message: format!("Delete environment `{project}/{env}`?"),
                        action: ConfirmAction::DeleteEnv { project, env },
                    });
                }
                None => {}
            },
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
        if !self.require_selected_env() {
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
        if !self.require_selected_env() {
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
        if !self.require_selected_env() {
            return;
        }
        self.input = Some(InputState {
            title: "Export .env".into(),
            prompt: "Output path:".into(),
            buffer: ".env".into(),
            action: InputAction::Export,
        });
    }

    /// Assign the current working directory to the selected project + env.
    fn assign_folder(&mut self) {
        if !self.require_selected_env() {
            return;
        }
        let (Some(project), Some(env)) = (self.current_project_name(), self.current_env_name())
        else {
            return;
        };
        let dir = match meta::current_dir() {
            Ok(d) => d,
            Err(e) => {
                self.status = format!("Could not resolve current folder: {e}");
                return;
            }
        };
        self.meta.set(dir.clone(), project.clone(), env.clone());
        if let Err(e) = meta::save(&self.meta) {
            self.status = format!("Failed to save assignment: {e}");
            return;
        }
        self.status = format!("Assigned {dir} → {project}/{env}.");
    }

    fn set_default_env(&mut self) {
        if !self.require_selected_env() {
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
        // Reference autocomplete takes priority when the popup is open.
        let cands = self.autocomplete_candidates();
        if !cands.is_empty() {
            match key.code {
                KeyCode::Down | KeyCode::Tab => {
                    self.ac_sel = (self.ac_sel + 1) % cands.len();
                    return Ok(());
                }
                KeyCode::Up => {
                    self.ac_sel = (self.ac_sel + cands.len() - 1) % cands.len();
                    return Ok(());
                }
                KeyCode::Enter => {
                    self.accept_autocomplete(&cands[self.ac_sel.min(cands.len() - 1)]);
                    return Ok(());
                }
                KeyCode::Esc => {
                    self.ac_dismissed = true;
                    return Ok(());
                }
                _ => {}
            }
        }

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
                self.ac_reset();
            }
            KeyCode::Char(c) => {
                input.buffer.push(c);
                self.ac_reset();
            }
            _ => {}
        }
        Ok(())
    }

    fn on_kv_key(&mut self, key: KeyEvent) -> Result<()> {
        // Autocomplete only applies while editing the Value field.
        let cands = self.autocomplete_candidates();
        if !cands.is_empty() {
            match key.code {
                KeyCode::Down | KeyCode::Tab => {
                    self.ac_sel = (self.ac_sel + 1) % cands.len();
                    return Ok(());
                }
                KeyCode::Up => {
                    self.ac_sel = (self.ac_sel + cands.len() - 1) % cands.len();
                    return Ok(());
                }
                KeyCode::Enter => {
                    self.accept_autocomplete(&cands[self.ac_sel.min(cands.len() - 1)]);
                    return Ok(());
                }
                KeyCode::Esc => {
                    self.ac_dismissed = true;
                    return Ok(());
                }
                _ => {}
            }
        }

        let Some(form) = self.kv_input.as_mut() else {
            return Ok(());
        };
        match key.code {
            KeyCode::Esc => {
                self.kv_input = None;
            }
            KeyCode::Tab | KeyCode::Down => form.active = form.active.next(),
            KeyCode::Up => form.active = form.active.prev(),
            // On the Type field, ←/→ cycle the kind.
            KeyCode::Left | KeyCode::Right if form.active == KvField::Type => {
                form.kind = form.kind.next();
            }
            KeyCode::Enter => {
                // Enter advances Key → Value → Type, then submits.
                match form.active {
                    KvField::Key => form.active = KvField::Value,
                    KvField::Value => form.active = KvField::Type,
                    KvField::Type => {
                        let form = self.kv_input.take().unwrap();
                        self.submit_kv(form);
                    }
                }
            }
            KeyCode::Backspace => {
                match form.active {
                    KvField::Key => form.key.pop(),
                    KvField::Value => form.value.pop(),
                    KvField::Type => None,
                };
                self.ac_reset();
            }
            KeyCode::Char(c) => {
                match form.active {
                    KvField::Key => form.key.push(c),
                    KvField::Value => form.value.push(c),
                    KvField::Type => {}
                };
                self.ac_reset();
            }
            _ => {}
        }
        Ok(())
    }

    // --- reference autocomplete -------------------------------------------

    /// Reset the autocomplete selection/dismissal after the buffer changes.
    fn ac_reset(&mut self) {
        self.ac_sel = 0;
        self.ac_dismissed = false;
    }

    /// The value buffer currently being edited and the key to exclude from
    /// suggestions (to avoid self-references), if a value field is active.
    fn ac_target(&self) -> Option<(String, Option<String>)> {
        if let Some(input) = &self.input {
            if let InputAction::EditSecret { key } = &input.action {
                return Some((input.buffer.clone(), Some(key.clone())));
            }
        }
        if let Some(form) = &self.kv_input {
            if form.active == KvField::Value {
                return Some((form.value.clone(), None));
            }
        }
        None
    }

    /// Candidate reference strings (inner, i.e. without `${}`) for the open
    /// `${…` token at the end of the active value buffer. Empty when there is
    /// no open token, it was dismissed, or nothing matches.
    fn autocomplete_candidates(&self) -> Vec<String> {
        if self.ac_dismissed {
            return Vec::new();
        }
        let Some((buffer, exclude)) = self.ac_target() else {
            return Vec::new();
        };
        let Some(query) = open_ref_query(&buffer) else {
            return Vec::new();
        };
        self.reference_candidates(&query, exclude.as_deref())
    }

    /// Build reference suggestions relative to the current project/env,
    /// fuzzy-filtered by `query`, capped to a handful.
    fn reference_candidates(&self, query: &str, exclude_key: Option<&str>) -> Vec<String> {
        let cur_p = self.current_project_name();
        let cur_e = self.current_env_name();
        let mut all: Vec<String> = Vec::new();
        for (pname, proj) in &self.handle.store.projects {
            for (ename, env) in &proj.environments {
                for key in env.values.keys() {
                    let same_p = cur_p.as_deref() == Some(pname.as_str());
                    let same_e = same_p && cur_e.as_deref() == Some(ename.as_str());
                    if same_e && exclude_key == Some(key.as_str()) {
                        continue; // don't suggest the secret being edited
                    }
                    let inner = if same_e {
                        key.clone()
                    } else if same_p {
                        format!("{ename}.{key}")
                    } else {
                        format!("{pname}.{ename}.{key}")
                    };
                    all.push(inner);
                }
            }
        }
        if query.is_empty() {
            all.truncate(8);
            return all;
        }
        let mut scored: Vec<(i32, String)> = all
            .into_iter()
            .filter_map(|s| fuzzy::score(query, &s).map(|(sc, _)| (sc, s)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, s)| s).take(8).collect()
    }

    /// Replace the open `${…` token at the end of the active buffer with the
    /// completed `${inner}`.
    fn accept_autocomplete(&mut self, inner: &str) {
        let replace = |buf: &mut String| {
            if let Some(pos) = buf.rfind("${") {
                buf.truncate(pos);
                buf.push_str("${");
                buf.push_str(inner);
                buf.push('}');
            }
        };
        if let Some(input) = self.input.as_mut() {
            replace(&mut input.buffer);
        } else if let Some(form) = self.kv_input.as_mut() {
            replace(&mut form.value);
        }
        self.ac_reset();
    }

    fn submit_kv(&mut self, form: KvInputState) {
        let key = form.key.trim().to_string();
        if key.is_empty() {
            self.status = "Key cannot be empty.".into();
            self.kv_input = Some(form);
            return;
        }
        if let Err(msg) = form.kind.validate(&form.value) {
            self.status = msg;
            self.kv_input = Some(form);
            return;
        }
        let label = form.kind.label();
        if let Some(e) = self.current_env_mut() {
            e.values.insert(key.clone(), form.value);
            e.set_kind(&key, form.kind);
            self.save();
            self.status = format!("Set `{key}` ({label}).");
        }
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
            InputAction::EditSecret { key } => {
                let kind = self.current_env().map(|e| e.kind(&key)).unwrap_or_default();
                if let Err(msg) = kind.validate(&state.buffer) {
                    // Re-open the editor so the user can fix the value.
                    self.status = msg;
                    self.input = Some(InputState {
                        title: format!("Edit {key}"),
                        prompt: "Value:".into(),
                        buffer: state.buffer,
                        action: InputAction::EditSecret { key },
                    });
                } else if let Some(e) = self.current_env_mut() {
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
                // Defer until the user picks merge vs replace.
                self.import_choice = Some(value);
            }
            InputAction::Export => {
                // Defer until the user picks a format.
                self.export_choice = Some(value);
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

    /// Keys for the import-mode chooser: merge / overwrite-all / replace.
    fn on_import_choice_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('m') | KeyCode::Char('M') | KeyCode::Enter => {
                if let Some(path) = self.import_choice.take() {
                    self.begin_import(&path, ImportMode::Merge);
                }
            }
            KeyCode::Char('o') | KeyCode::Char('O') => {
                if let Some(path) = self.import_choice.take() {
                    self.begin_import(&path, ImportMode::OverwriteAll);
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if let Some(path) = self.import_choice.take() {
                    self.begin_import(&path, ImportMode::Replace);
                }
            }
            _ => {
                self.import_choice = None;
                self.status = "Import cancelled.".into();
            }
        }
        Ok(())
    }

    /// Read + parse the file and apply it according to `mode`. For `Merge`,
    /// new keys are added immediately and changed keys are queued for per-key
    /// confirmation.
    fn begin_import(&mut self, path: &str, mode: ImportMode) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("Import failed: {e}");
                return;
            }
        };
        let parsed = envfile::parse(&text);
        let Some(env) = self.current_env_mut() else {
            self.status = "No environment selected.".into();
            return;
        };

        match mode {
            ImportMode::Replace => {
                let count = parsed.len();
                env.values.clear();
                env.values.extend(parsed);
                prune_types(env);
                self.save();
                self.status = format!("Imported {count} secrets from {path} (replaced env).");
            }
            ImportMode::OverwriteAll => {
                let count = parsed.len();
                for (k, v) in parsed {
                    env.values.insert(k, v);
                }
                self.save();
                self.status = format!("Imported {count} secrets from {path} (overwrote all).");
            }
            ImportMode::Merge => {
                let mut added = 0;
                let mut conflicts = Vec::new();
                for (k, v) in parsed {
                    match env.values.get(&k) {
                        None => {
                            env.values.insert(k, v);
                            added += 1;
                        }
                        Some(old) if *old != v => {
                            conflicts.push((k, old.clone(), v));
                        }
                        Some(_) => {} // unchanged
                    }
                }
                self.save();
                if conflicts.is_empty() {
                    self.status = format!(
                        "Imported {added} new secrets from {path} (no changes to existing)."
                    );
                } else {
                    self.import_session = Some(ImportSession {
                        source: path.to_string(),
                        conflicts,
                        pos: 0,
                        added,
                        updated: 0,
                    });
                }
            }
        }
    }

    /// Keys while confirming each changed key during a merge import.
    fn on_import_session_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(session) = self.import_session.as_mut() else {
            return Ok(());
        };
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                let (k, _, new) = session.conflicts[session.pos].clone();
                if let Some(e) = self.current_env_mut() {
                    e.values.insert(k, new);
                }
                if let Some(s) = self.import_session.as_mut() {
                    s.updated += 1;
                    s.pos += 1;
                }
                self.advance_import();
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                if let Some(s) = self.import_session.as_mut() {
                    s.pos += 1;
                }
                self.advance_import();
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                // Overwrite this and every remaining conflict.
                let rest: Vec<(String, String)> = session.conflicts[session.pos..]
                    .iter()
                    .map(|(k, _, new)| (k.clone(), new.clone()))
                    .collect();
                let n = rest.len();
                if let Some(e) = self.current_env_mut() {
                    for (k, v) in rest {
                        e.values.insert(k, v);
                    }
                }
                if let Some(s) = self.import_session.as_mut() {
                    s.updated += n;
                    s.pos = s.conflicts.len();
                }
                self.advance_import();
            }
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                if let Some(s) = self.import_session.as_mut() {
                    s.pos = s.conflicts.len();
                }
                self.advance_import();
            }
            _ => {}
        }
        Ok(())
    }

    /// Finish the import session if all conflicts have been decided.
    fn advance_import(&mut self) {
        let done = self
            .import_session
            .as_ref()
            .map(|s| s.pos >= s.conflicts.len())
            .unwrap_or(true);
        if done {
            if let Some(s) = self.import_session.take() {
                self.save();
                self.status = format!(
                    "Imported from {}: {} added, {} updated.",
                    s.source, s.added, s.updated
                );
            }
        }
    }

    /// Keys for the export-format chooser.
    fn on_export_choice_key(&mut self, key: KeyEvent) -> Result<()> {
        let fmt = match key.code {
            KeyCode::Char('e') | KeyCode::Enter => Some(Format::Env),
            KeyCode::Char('s') => Some(Format::Shell),
            KeyCode::Char('j') => Some(Format::Json),
            KeyCode::Char('t') => Some(Format::Toml),
            _ => None,
        };
        match fmt {
            Some(fmt) => {
                if let Some(path) = self.export_choice.take() {
                    self.do_export(&path, fmt);
                }
            }
            None => {
                self.export_choice = None;
                self.status = "Export cancelled.".into();
            }
        }
        Ok(())
    }

    fn do_export(&mut self, path: &str, fmt: Format) {
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
        let output = match crate::commands::render(&resolved, fmt) {
            Ok(o) => o,
            Err(e) => {
                self.status = format!("Export failed: {e}");
                return;
            }
        };
        match std::fs::write(path, output) {
            Ok(()) => {
                self.status = format!("Exported {} secrets to {path}.", resolved.len());
            }
            Err(e) => self.status = format!("Export failed: {e}"),
        }
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
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => ed.insert_char(c),
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
                prune_types(e);
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
                        e.types.shift_remove(&key);
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
        // Blank the whole screen, then float a bounded window in the centre.
        f.render_widget(Clear, f.area());
        let win = window_area(f.area());
        self.draw_window(f, win);

        if self.show_help {
            self.draw_help(f);
        }
        if let Some(input) = &self.input {
            self.draw_input(f, input);
        }
        if let Some(form) = &self.kv_input {
            self.draw_kv(f, form);
        }
        if let Some(confirm) = &self.confirm {
            self.draw_confirm(f, confirm);
        }
        if let Some(path) = &self.import_choice {
            self.draw_import_choice(f, path);
        }
        if let Some(session) = &self.import_session {
            self.draw_import_session(f, session);
        }
        if let Some(path) = &self.export_choice {
            self.draw_export_choice(f, path);
        }
        if let Some(editor) = &self.editor {
            self.draw_editor(f, editor);
        }
    }

    fn draw_export_choice(&self, f: &mut Frame, path: &str) {
        let area = centered_rect(64, 9, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT))
            .title(" Export — choose format ");
        let inner = Paragraph::new(vec![
            Line::from(Span::styled(
                format!("To {path}"),
                Style::default().fg(Color::Gray),
            )),
            Line::from(""),
            Line::from("  e  .env       KEY=VALUE"),
            Line::from("  s  shell      export KEY=VALUE"),
            Line::from("  j  json       { \"KEY\": \"VALUE\" }"),
            Line::from("  t  toml       KEY = \"VALUE\""),
            Line::from(Span::styled(
                "  Enter = env   ·   Esc cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(block)
        .wrap(Wrap { trim: false });
        f.render_widget(inner, area);
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

    /// Draw the whole floating window: outer frame, results, preview, prompt.
    fn draw_window(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .title(Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    "dev-secrets",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  ▸  {} ", self.breadcrumb()),
                    Style::default().fg(Color::Gray),
                ),
            ]))
            .title_bottom(Line::from(format!(" {} ", self.status)).left_aligned())
            .title_bottom(
                Line::from(Span::styled(
                    " / search · n new env · p new project · e edit · d del · ? help · q quit ",
                    Style::default().fg(Color::DarkGray),
                ))
                .right_aligned(),
            );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
            .split(inner);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(rows[0]);

        self.draw_results(f, cols[0]);
        self.draw_preview(f, cols[1]);
        self.draw_prompt(f, rows[1]);
    }

    /// Breadcrumb shown in the window title.
    fn breadcrumb(&self) -> String {
        match self.focus {
            Focus::Tree => "Projects / Environments".to_string(),
            Focus::Secrets => format!(
                "{} ▸ {} ▸ Secrets",
                self.current_project_name().unwrap_or_default(),
                self.current_env_name().unwrap_or_default()
            ),
        }
    }

    fn draw_results(&self, f: &mut Frame, area: Rect) {
        match self.focus {
            Focus::Tree => self.draw_tree(f, area),
            Focus::Secrets => self.draw_secrets(f, area),
        }
    }

    /// The combined Projects + Environments tree with fuzzy highlighting.
    fn draw_tree(&self, f: &mut Frame, area: Rect) {
        let rows = self.tree_rows();
        let q = &self.query;
        let mut items: Vec<ListItem> = Vec::with_capacity(rows.len());
        for r in &rows {
            match *r {
                Row::Project(pi) => {
                    let (name, proj) = self.handle.store.projects.get_index(pi).unwrap();
                    let has_envs = !proj.environments.is_empty();
                    let collapsed = q.is_empty() && self.collapsed.contains(name);
                    let indicator = if !has_envs {
                        "  "
                    } else if collapsed {
                        "▸ "
                    } else {
                        "▾ "
                    };
                    let matches = fuzzy::score(q, name).map(|(_, m)| m).unwrap_or_default();
                    let mut spans = vec![Span::styled(
                        indicator,
                        Style::default().fg(Color::DarkGray),
                    )];
                    spans.extend(name_spans(
                        name,
                        &matches,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ));
                    spans.push(Span::styled(
                        format!("  ({} env)", proj.environments.len()),
                        Style::default().fg(Color::DarkGray),
                    ));
                    items.push(ListItem::new(Line::from(spans)));
                }
                Row::Env(pi, ei) => {
                    let (_, proj) = self.handle.store.projects.get_index(pi).unwrap();
                    let (ename, env) = proj.environments.get_index(ei).unwrap();
                    let is_default = proj.default_env.as_deref() == Some(ename.as_str());
                    let matches = fuzzy::score(q, ename).map(|(_, m)| m).unwrap_or_default();
                    let mut spans =
                        vec![Span::styled("    – ", Style::default().fg(Color::DarkGray))];
                    spans.extend(name_spans(ename, &matches, Style::default().fg(ACCENT)));
                    if is_default {
                        spans.push(Span::styled(" ★", Style::default().fg(Color::Yellow)));
                    }
                    spans.push(Span::styled(
                        format!("  {} keys", env.values.len()),
                        Style::default().fg(Color::DarkGray),
                    ));
                    items.push(ListItem::new(Line::from(spans)));
                }
            }
        }

        let nproj = self.handle.store.projects.len();
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(format!(" Projects · Environments  ({nproj}) "));

        if items.is_empty() {
            let msg = if nproj == 0 {
                "No projects yet — press p to create one".to_string()
            } else {
                "No matches".to_string()
            };
            f.render_widget(
                Paragraph::new(Span::styled(msg, Style::default().fg(Color::DarkGray)))
                    .block(block),
                area,
            );
            return;
        }

        let list = List::new(items)
            .block(block)
            .highlight_style(
                Style::default()
                    .bg(ACCENT)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▌");
        let mut state = ListState::default();
        state.select(Some(self.tree_sel.min(rows.len().saturating_sub(1))));
        f.render_stateful_widget(list, area, &mut state);
    }

    /// Spans for a secret value as shown in lists/previews. When hidden,
    /// references display as `$ref:…` and other values are masked. When
    /// revealed (`s`), references resolve to their actual value.
    fn shown_value_spans(
        &self,
        project: &str,
        env: &str,
        key: &str,
        raw: &str,
    ) -> Vec<Span<'static>> {
        if self.reveal && is_reference(raw) {
            return match resolve::resolve_at(&self.handle.store, project, env, key) {
                Ok(v) => vec![Span::styled(v, Style::default().fg(Color::Green))],
                Err(e) => vec![Span::styled(
                    format!("<{e}>"),
                    Style::default().fg(Color::Red),
                )],
            };
        }
        value_spans(raw, self.reveal)
    }

    /// The secrets list for the current environment, fuzzy-filtered.
    fn draw_secrets(&self, f: &mut Frame, area: Rect) {
        let labels = self.secret_labels();
        let filtered = self.filtered_indices();
        let project = self.current_project_name().unwrap_or_default();
        let env = self.current_env_name().unwrap_or_default();

        let mut items: Vec<ListItem> = Vec::with_capacity(filtered.len());
        let mut selected_row = 0;
        for (row, &idx) in filtered.iter().enumerate() {
            if idx == self.secret_idx {
                selected_row = row;
            }
            let name = &labels[idx];
            let matches = fuzzy::score(&self.query, name)
                .map(|(_, m)| m)
                .unwrap_or_default();
            let v = self
                .current_env()
                .and_then(|e| e.values.get_index(idx))
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            let mut spans = name_spans(name, &matches, Style::default());
            spans.push(Span::styled("  = ", Style::default().fg(Color::DarkGray)));
            spans.extend(self.shown_value_spans(&project, &env, name, &v));
            items.push(ListItem::new(Line::from(spans)));
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(format!(" Secrets  {}/{} ", filtered.len(), labels.len()));

        if items.is_empty() {
            let msg = if labels.is_empty() {
                "No secrets — press n to add one".to_string()
            } else {
                "No matches".to_string()
            };
            f.render_widget(
                Paragraph::new(Span::styled(msg, Style::default().fg(Color::DarkGray)))
                    .block(block),
                area,
            );
            return;
        }

        let list = List::new(items)
            .block(block)
            .highlight_style(
                Style::default()
                    .bg(ACCENT)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▌");
        let mut state = ListState::default();
        state.select(Some(selected_row));
        f.render_stateful_widget(list, area, &mut state);
    }

    /// The preview pane: details of the highlighted item.
    fn draw_preview(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" Preview ");
        let lines = match self.focus {
            Focus::Tree => match self.selected_row() {
                Some(Row::Env(..)) => self.preview_env(),
                _ => self.preview_project(),
            },
            Focus::Secrets => self.preview_secret(),
        };
        f.render_widget(
            Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    fn preview_project(&self) -> Vec<Line<'_>> {
        let Some(p) = self.current_project() else {
            return vec![Line::from(Span::styled(
                "No project selected.",
                Style::default().fg(Color::DarkGray),
            ))];
        };
        let name = self.current_project_name().unwrap_or_default();
        let folders = self.meta.folders_for_project(&name);
        let folder_line = if folders.is_empty() {
            "folders: (none assigned — press f here)".to_string()
        } else {
            format!("folders: {}", folders.join(", "))
        };
        let mut lines = vec![
            heading(&name),
            Line::from(""),
            Line::from(Span::styled(
                folder_line,
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "environments:",
                Style::default().fg(Color::Gray),
            )),
        ];
        for (env, e) in &p.environments {
            let def = if p.default_env.as_deref() == Some(env) {
                " ★"
            } else {
                ""
            };
            lines.push(Line::from(vec![
                Span::raw("  • "),
                Span::styled(env.clone(), Style::default().fg(ACCENT)),
                Span::styled(def, Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!("  ({} keys)", e.values.len()),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
        if p.environments.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (none yet — press n)",
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines
    }

    fn preview_env(&self) -> Vec<Line<'_>> {
        let Some(e) = self.current_env() else {
            return vec![Line::from(Span::styled(
                "No environment selected.",
                Style::default().fg(Color::DarkGray),
            ))];
        };
        let env = self.current_env_name().unwrap_or_default();
        let project = self.current_project_name().unwrap_or_default();
        let entries: Vec<(String, String)> = e
            .values
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let mut lines = vec![heading(&env), Line::from("")];
        for (k, v) in &entries {
            let mut spans = vec![
                Span::styled(k.clone(), Style::default().fg(Color::Yellow)),
                Span::raw(" = "),
            ];
            spans.extend(self.shown_value_spans(&project, &env, k, v));
            lines.push(Line::from(spans));
        }
        if entries.is_empty() {
            lines.push(Line::from(Span::styled(
                "(empty — press n to add a secret)",
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines
    }

    fn preview_secret(&self) -> Vec<Line<'_>> {
        let (Some(project), Some(env), Some(key)) = (
            self.current_project_name(),
            self.current_env_name(),
            self.current_secret_key(),
        ) else {
            return vec![Line::from(Span::styled(
                "No secret selected.",
                Style::default().fg(Color::DarkGray),
            ))];
        };
        let raw = self
            .handle
            .store
            .value(&project, &env, &key)
            .cloned()
            .unwrap_or_default();
        let kind = self.current_env().map(|e| e.kind(&key)).unwrap_or_default();
        let mut lines = vec![
            heading(&key),
            Line::from(Span::styled(
                format!("type: {}", kind.label()),
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
        ];
        let mut value_line = vec![Span::styled("value: ", Style::default().fg(Color::Gray))];
        value_line.extend(value_spans(&raw, self.reveal));
        lines.push(Line::from(value_line));
        // If the value contains references, show the resolved result too.
        if is_reference(&raw) {
            match resolve::resolve_at(&self.handle.store, &project, &env, &key) {
                Ok(resolved) => {
                    let shown = if self.reveal {
                        resolved
                    } else {
                        mask(&resolved)
                    };
                    lines.push(Line::from(vec![
                        Span::styled("resolved: ", Style::default().fg(Color::Gray)),
                        Span::styled(shown, Style::default().fg(Color::Green)),
                    ]));
                }
                Err(e) => lines.push(Line::from(Span::styled(
                    format!("resolved: <{e}>"),
                    Style::default().fg(Color::Red),
                ))),
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Enter/e edit · g goto ref · c copy · d delete · s show",
            Style::default().fg(Color::DarkGray),
        )));
        lines
    }

    /// The bottom prompt box with the fuzzy query.
    fn draw_prompt(&self, f: &mut Frame, area: Rect) {
        let border = if self.searching {
            ACCENT
        } else {
            Color::DarkGray
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border))
            .title(" Filter ");
        let inner = block.inner(area);
        f.render_widget(block, area);

        let cursor = if self.searching { "▏" } else { "" };
        let line = Line::from(vec![
            Span::styled(
                "❯ ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw(self.query.clone()),
            Span::styled(cursor, Style::default().fg(ACCENT)),
            if self.query.is_empty() && !self.searching {
                Span::styled(
                    "type / to fuzzy-filter",
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::raw("")
            },
        ]);
        f.render_widget(Paragraph::new(line), inner);
    }

    /// Whether an unclosed `${…` token is open in the active value field.
    fn ac_open(&self) -> bool {
        if self.ac_dismissed {
            return false;
        }
        self.ac_target()
            .and_then(|(b, _)| open_ref_query(&b))
            .is_some()
    }

    /// Lines for the autocomplete area: the candidate list, or a hint when a
    /// `${…` token is open but nothing matches.
    fn ac_render_lines(&self) -> Vec<Line<'static>> {
        let cands = self.autocomplete_candidates();
        if !cands.is_empty() {
            return self.ac_lines(&cands);
        }
        if self.ac_open() {
            return vec![Line::from(Span::styled(
                "references — (no other secrets to reference yet)",
                Style::default().fg(Color::DarkGray),
            ))];
        }
        Vec::new()
    }

    /// Suggestion lines for the autocomplete popup (empty when not active).
    fn ac_lines(&self, cands: &[String]) -> Vec<Line<'static>> {
        if cands.is_empty() {
            return Vec::new();
        }
        let sel = self.ac_sel.min(cands.len() - 1);
        let mut lines = vec![Line::from(Span::styled(
            "references — ↑/↓/Tab, Enter to insert, Esc to dismiss:",
            Style::default().fg(Color::DarkGray),
        ))];
        for (i, c) in cands.iter().enumerate() {
            let style = if i == sel {
                Style::default()
                    .fg(Color::Black)
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ACCENT)
            };
            lines.push(Line::from(Span::styled(format!("  ${{{c}}}"), style)));
        }
        lines
    }

    fn draw_input(&self, f: &mut Frame, input: &InputState) {
        let ac = self.ac_render_lines();
        let height = 3 + ac.len() as u16 + 2;
        let area = centered_rect(60, height, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(format!(" {} ", input.title));
        let mut lines = vec![
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
        ];
        lines.extend(ac);
        lines.push(Line::from(Span::styled(
            "Enter: confirm   Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )));
        f.render_widget(
            Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
    }

    fn draw_kv(&self, f: &mut Frame, form: &KvInputState) {
        let ac = self.ac_render_lines();
        let height = 4 + ac.len() as u16 + 2;
        let area = centered_rect(60, height, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" New secret ")
            .title_bottom(" Tab: field   Enter: next/confirm   Esc: cancel ");

        let field = |label: &str, value: &str, active: bool| -> Line {
            let label_style = if active {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            let cursor = if active { "_" } else { "" };
            Line::from(vec![
                Span::styled(format!("{label:<7}"), label_style),
                Span::styled(
                    format!("{value}{cursor}"),
                    Style::default().fg(Color::White),
                ),
            ])
        };

        // The Type field shows the kinds with the active one highlighted.
        let type_active = form.active == KvField::Type;
        let type_label_style = if type_active {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        let mut type_spans = vec![Span::styled(format!("{:<7}", "Type:"), type_label_style)];
        for k in [Kind::Text, Kind::Number, Kind::Json] {
            let style = if k == form.kind {
                Style::default()
                    .fg(Color::Black)
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            type_spans.push(Span::styled(format!(" {} ", k.label()), style));
            type_spans.push(Span::raw(" "));
        }
        if type_active {
            type_spans.push(Span::styled("(←/→)", Style::default().fg(Color::DarkGray)));
        }

        let mut lines = vec![
            field("Key:", &form.key, form.active == KvField::Key),
            field("Value:", &form.value, form.active == KvField::Value),
            Line::from(type_spans),
        ];
        lines.extend(ac);
        f.render_widget(
            Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
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

    fn draw_import_choice(&self, f: &mut Frame, path: &str) {
        let area = centered_rect(64, 9, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT))
            .title(" Import .env ");
        let inner = Paragraph::new(vec![
            Line::from(Span::styled(
                format!("From {path}"),
                Style::default().fg(Color::Gray),
            )),
            Line::from(""),
            Line::from("  m  merge — add new, confirm each changed key"),
            Line::from("  o  overwrite — add new, replace all changed keys"),
            Line::from("  r  replace — clear the env, then load the file"),
            Line::from(Span::styled(
                "  Esc  cancel",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(block)
        .wrap(Wrap { trim: false });
        f.render_widget(inner, area);
    }

    fn draw_import_session(&self, f: &mut Frame, s: &ImportSession) {
        let area = centered_rect(70, 10, f.area());
        f.render_widget(Clear, area);
        let (key, old, new) = &s.conflicts[s.pos];
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT))
            .title(format!(
                " Overwrite changed key ({}/{}) ",
                s.pos + 1,
                s.conflicts.len()
            ));
        let shown_old = if self.reveal { old.clone() } else { mask(old) };
        let shown_new = if self.reveal { new.clone() } else { mask(new) };
        let inner = Paragraph::new(vec![
            Line::from(Span::styled(
                key.clone(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::styled("old: ", Style::default().fg(Color::Gray)),
                Span::raw(shown_old),
            ]),
            Line::from(vec![
                Span::styled("new: ", Style::default().fg(Color::Gray)),
                Span::styled(shown_new, Style::default().fg(Color::Green)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "y overwrite · n keep · a overwrite all · q stop",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(block)
        .wrap(Wrap { trim: false });
        f.render_widget(inner, area);
    }

    fn draw_help(&self, f: &mut Frame) {
        let area = centered_rect(64, 26, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" Help — press any key to close ");

        let head = |t: &str| {
            Line::from(Span::styled(
                t.to_string(),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ))
        };
        let item = |k: &str, desc: &str| {
            Line::from(vec![
                Span::styled(format!("  {k:<7}"), Style::default().fg(Color::Yellow)),
                Span::raw(desc.to_string()),
            ])
        };

        let lines = vec![
            head("Navigation (Projects + Environments share one tree)"),
            item("/", "fuzzy-filter (matches projects and environments)"),
            item("↑/k ↓/j", "move selection (Ctrl-n/Ctrl-p while searching)"),
            item("→/l", "expand project / open environment → Secrets"),
            item("←/h", "collapse project / jump to parent project"),
            item("Enter", "toggle a project, or open an environment"),
            item("g", "go to the secret a reference points at"),
            Line::from(""),
            head("Actions"),
            item("p", "new project"),
            item("n", "new environment (or new secret on Secrets)"),
            item("e", "edit secret, or the env inline"),
            item("a / E", "edit env inline / in $EDITOR"),
            item("c / C", "copy secret value / whole env"),
            item("d", "delete project / env / secret"),
            item("y", "duplicate environment"),
            item("i / x", "import / export .env"),
            item("D", "set default env"),
            item("f", "assign current folder to this project/env"),
            item("s", "toggle showing/hiding secret values"),
            Line::from(""),
            head("References"),
            item("", "${secret} · ${env.secret} · ${project.env.secret}"),
            item("q", "quit (also Ctrl-C)"),
        ];
        f.render_widget(
            Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false }),
            area,
        );
    }
}

/// The centred, size-capped window rectangle (never the full terminal).
fn window_area(area: Rect) -> Rect {
    let w = area.width.min(MAX_WIDTH);
    let h = area.height.min(MAX_HEIGHT);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// Build a result line: `name` with matched characters highlighted, followed
/// by a styled `suffix`.
/// Drop type entries whose key no longer exists in `env.values`.
fn prune_types(env: &mut Environment) {
    let present: HashSet<String> = env.values.keys().cloned().collect();
    env.types.retain(|k, _| present.contains(k));
}

/// A bold heading line for the preview pane.
fn heading(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    ))
}

/// Spans for `name` with fuzzy-matched characters highlighted over `base`.
fn name_spans(name: &str, matches: &[usize], base: Style) -> Vec<Span<'static>> {
    name.chars()
        .enumerate()
        .map(|(i, ch)| {
            let style = if matches.contains(&i) {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                base
            };
            Span::styled(ch.to_string(), style)
        })
        .collect()
}

fn mask(value: &str) -> String {
    let len = value.chars().count();
    if len == 0 {
        return "(empty)".into();
    }
    if len <= 4 {
        return "•".repeat(len);
    }
    // Only hint with leading alphanumerics so structured values (JSON `{"`,
    // arrays `[`, quoted strings) don't leak their shape.
    let head: String = value
        .chars()
        .take(2)
        .take_while(|c| c.is_alphanumeric())
        .collect();
    format!("{head}{}", "•".repeat(6))
}

/// Whether a value contains at least one `${…}` reference.
fn is_reference(value: &str) -> bool {
    value.contains("${")
}

/// The inner text of the first `${…}` reference in `raw`, if any.
fn first_reference(raw: &str) -> Option<String> {
    let start = raw.find("${")?;
    let after = &raw[start + 2..];
    let end = after.find('}')?;
    Some(after[..end].to_string())
}

/// Spans for a secret value. References are shown in clear as
/// `$ref:project.env.key` (they aren't secret) with a coloured `$ref:` prefix;
/// other values are masked unless `reveal` is set.
fn value_spans(raw: &str, reveal: bool) -> Vec<Span<'static>> {
    if !is_reference(raw) {
        let shown = if reveal { raw.to_string() } else { mask(raw) };
        return vec![Span::styled(shown, Style::default().fg(Color::Gray))];
    }
    let lit = Style::default().fg(Color::Gray);
    let prefix = Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::BOLD);
    let target = Style::default().fg(Color::Blue);
    let mut spans = Vec::new();
    let mut rest = raw;
    while let Some(start) = rest.find("${") {
        if start > 0 {
            spans.push(Span::styled(rest[..start].to_string(), lit));
        }
        let after = &rest[start + 2..];
        if let Some(end) = after.find('}') {
            spans.push(Span::styled("$ref:", prefix));
            spans.push(Span::styled(after[..end].to_string(), target));
            rest = &after[end + 1..];
        } else {
            spans.push(Span::styled(rest[start..].to_string(), lit));
            return spans;
        }
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_string(), lit));
    }
    spans
}

/// Build the `.env` document shown to the user in their editor.
fn editor_template(
    project: &str,
    env: &str,
    values: &indexmap::IndexMap<String, String>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# dev-secrets — editing {project}/{env}\n"));
    out.push_str("# Edit as a normal .env file: KEY=VALUE per line.\n");
    out.push_str("# Lines you remove are deleted; blank lines and # comments are ignored.\n");
    out.push_str("# References like ${project.env.KEY} are kept as-is and resolved on export.\n");
    out.push_str("# Save and quit to apply, or quit without saving to cancel.\n\n");
    out.push_str(&envfile::serialize(values));
    out
}

/// If `buffer` ends with an unclosed `${…` reference, return the partial text
/// typed after `${` (the autocomplete query).
fn open_ref_query(buffer: &str) -> Option<String> {
    let pos = buffer.rfind("${")?;
    let after = &buffer[pos + 2..];
    if after.contains('}') {
        None
    } else {
        Some(after.to_string())
    }
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
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "notepad".into()
            } else {
                "vi".into()
            }
        })
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
        values.insert(
            "API_URL".to_string(),
            "http://${api.dev.DB_HOST}:5432".to_string(),
        );

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
