//! HDL support in the editor: the VHDL and Verilog sources of the project,
//! a source editor, compilation and synthesis with GHDL and Yosys
//! ([`lb_hdl`]), and the circuits synthesized from the sources.
//!
//! Synthesized circuits are ordinary subcircuits marked as generated
//! ([`lb_core::circuit::HdlInfo`]). They are read-only, flagged out of date
//! (in orange) when the sources change, and replaced as one undoable step by
//! the next successful synthesis; a failed synthesis keeps the last good
//! ones. Everything here is optional: the feature is compiled with the
//! `hdl` Cargo feature and shown when enabled in the settings.

mod highlight;
mod undo;
pub mod view;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use iced::widget::text_editor;
use iced::{Subscription, Task};

use lb_core::circuit::{HdlConfig, Project, VhdlStd};
use lb_core::history::{Command, Op};
use lb_hdl::lang::{Lang, Unit, units};
use lb_hdl::log::Diagnostic;
use lb_hdl::toolchain::{self, Action, Cancel, Job, Outcome};

use crate::app::{App, Message};

/// Where to get GHDL and Yosys for every platform.
pub const TOOLS_URL: &str = "https://github.com/YosysHQ/oss-cad-suite-build/releases";

/// A source file of the project, as last seen on disk.
#[derive(Clone, Debug)]
pub struct SourceFile {
    /// As written in the project (relative to the project file).
    pub entry: String,
    pub path: PathBuf,
    pub lang: Option<Lang>,
    pub units: Vec<Unit>,
    stamp: Option<Stamp>,
    pub missing: bool,
}

/// A file open in the source editor.
#[derive(Debug)]
pub struct Tab {
    pub path: PathBuf,
    pub lang: Option<Lang>,
    pub content: text_editor::Content,
    pub dirty: bool,
    stamp: Option<Stamp>,
    /// The file changed on disk while edited here.
    pub changed_on_disk: bool,
    undo: undo::Undo,
    /// The spaces inserted by the last Tabs in a row, as `(line, from,
    /// to)` columns: Backspace takes each run back at once.
    tab_runs: Vec<(usize, usize, usize)>,
}

#[derive(Debug)]
pub struct Hdl {
    /// The HDL view is shown instead of the circuit.
    pub panel: bool,
    /// Widths of the sources and messages columns of the HDL view.
    pub sources_w: f32,
    pub log_w: f32,
    pub tabs: Vec<Tab>,
    pub active: usize,
    /// A tab with unsaved changes the user asked to close once.
    close_asked: Option<usize>,
    pub log: Vec<Diagnostic>,
    job: Option<(Action, Cancel)>,
    /// Synthesize again when the running job ends.
    queued: bool,
    /// Counts the projects opened, so that a job started for another
    /// project is not applied to this one.
    epoch: u64,
    pub files: Vec<SourceFile>,
    /// Digest of the sources on disk, compared with the one of each
    /// synthesized circuit.
    pub digest: String,
    /// Project settings the files were read for.
    seen: Option<(Vec<String>, VhdlStd, Option<PathBuf>)>,
    /// The tools found on the `PATH` or at the configured paths.
    pub ghdl: Option<PathBuf>,
    pub yosys: Option<PathBuf>,
}

impl Default for Hdl {
    fn default() -> Self {
        Hdl {
            panel: false,
            sources_w: 280.0,
            log_w: 420.0,
            tabs: Vec::new(),
            active: 0,
            close_asked: None,
            log: Vec::new(),
            job: None,
            queued: false,
            epoch: 0,
            files: Vec::new(),
            digest: String::new(),
            seen: None,
            ghdl: None,
            yosys: None,
        }
    }
}

impl Hdl {
    /// The job being run, if any.
    pub fn running(&self) -> Option<Action> {
        self.job.as_ref().map(|j| j.0)
    }

    pub fn has_unsaved(&self) -> bool {
        self.tabs.iter().any(|t| t.dirty)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    TogglePanel,
    AddFiles,
    FilesPicked(Vec<PathBuf>),
    NewFile(Lang),
    NewFilePicked(Lang, Option<PathBuf>),
    RemoveSource(usize),
    SetStd(VhdlStd),
    /// Synthesize an entity into a circuit of the project.
    AddTop(String),
    /// Show a synthesized circuit.
    View(String),
    Open(PathBuf),
    /// Open a file with the cursor at a 1-based line and column.
    OpenAt(PathBuf, usize, usize),
    SelectTab(usize),
    CloseTab(usize),
    Edit(text_editor::Action),
    Undo,
    Redo,
    /// Delete the word after the cursor (`true`) or before it, or the selection.
    DeleteWord(bool),
    /// Scroll the editor so that this line (with a fraction) is at the top.
    ScrollTo(f32),
    Save,
    Reload,
    KeepMine,
    OpenExternal,
    Compile,
    Synthesize,
    Cancel,
    /// A job ended; the epoch it was started in.
    Done(u64, Arc<Outcome>),
    /// Open the source a log line points to.
    GoTo(usize),
    ClearLog,
    /// Show the dialog with the paths of the tools.
    ShowTools,
    SetGhdl(String),
    SetYosys(String),
    GetTools,
    /// Look for changed files.
    Poll,
}

fn msg(m: Msg) -> Message {
    Message::Hdl(m)
}

pub fn update(app: &mut App, m: Msg) -> Task<Message> {
    match m {
        Msg::TogglePanel => {
            app.hdl.panel = !app.hdl.panel;
            if app.hdl.panel {
                find_tools(app);
            }
        }
        Msg::AddFiles => {
            return Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .add_filter("VHDL and Verilog sources", &["vhd", "vhdl", "v", "sv"])
                        .pick_files()
                        .await
                        .map(|f| f.iter().map(|h| h.path().to_path_buf()).collect())
                        .unwrap_or_default()
                },
                |files| msg(Msg::FilesPicked(files)),
            );
        }
        Msg::FilesPicked(files) => {
            if let Some(first) = files.first().cloned() {
                add_sources(app, &files);
                open(app, &first);
            }
        }
        Msg::NewFile(lang) => {
            let (name, ext) = match lang {
                Lang::Vhdl => ("new_entity.vhd", "vhd"),
                Lang::Verilog => ("new_module.v", "v"),
            };
            return Task::perform(
                async move {
                    rfd::AsyncFileDialog::new()
                        .set_file_name(name)
                        .add_filter(lang.name(), &[ext])
                        .save_file()
                        .await
                        .map(|h| h.path().to_path_buf())
                },
                move |p| msg(Msg::NewFilePicked(lang, p)),
            );
        }
        Msg::NewFilePicked(lang, Some(path)) => {
            // An existing file is added as it is, never overwritten.
            if !path.exists() {
                let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if let Err(e) = std::fs::write(&path, lang.template(&identifier(&stem))) {
                    app.warn(format!("Cannot create {}: {e}", path.display()));
                    return Task::none();
                }
            }
            add_sources(app, std::slice::from_ref(&path));
            open(app, &path);
        }
        Msg::NewFilePicked(_, None) => {}
        Msg::RemoveSource(i) => {
            let mut cfg = app.project.hdl.clone();
            if i < cfg.sources.len() {
                let gone = cfg.sources.remove(i);
                set_config(app, cfg, &format!("Remove {gone}"));
            }
        }
        Msg::SetStd(std) => {
            let cfg = HdlConfig { vhdl_std: std, ..app.project.hdl.clone() };
            set_config(app, cfg, "VHDL standard");
        }
        Msg::AddTop(name) => {
            if !app.project.hdl.tops.contains(&name) {
                let mut cfg = app.project.hdl.clone();
                cfg.tops.push(name.clone());
                set_config(app, cfg, &format!("Add {name}"));
            }
            return start(app, Action::Synthesize);
        }
        Msg::View(name) => {
            app.hdl.panel = false;
            return app.update(Message::OpenCircuit(name));
        }
        Msg::Open(path) => open(app, &path),
        Msg::OpenAt(path, line, col) => {
            open(app, &path);
            if let Some(tab) = app.hdl.tabs.get_mut(app.hdl.active) {
                let position = text_editor::Position { line: line.saturating_sub(1), column: col.saturating_sub(1) };
                tab.content.move_to(text_editor::Cursor { position, selection: None });
            }
        }
        Msg::SelectTab(i) => {
            if i < app.hdl.tabs.len() {
                app.hdl.active = i;
            }
        }
        Msg::CloseTab(i) => {
            let Some(tab) = app.hdl.tabs.get(i) else { return Task::none() };
            if tab.dirty && app.hdl.close_asked != Some(i) {
                app.hdl.close_asked = Some(i);
                let name = file_name(&tab.path);
                app.warn(format!("{name} has unsaved changes: save it, or close it again to discard them."));
                return Task::none();
            }
            app.hdl.close_asked = None;
            app.hdl.tabs.remove(i);
            if app.hdl.active >= i && app.hdl.active > 0 {
                app.hdl.active -= 1;
            }
        }
        Msg::Edit(action) => {
            if let Some(tab) = app.hdl.tabs.get_mut(app.hdl.active) {
                use text_editor::{Action, Edit};
                // Tab without a selection: spaces to the next tab stop (the
                // editor's own counts only the blanks before the cursor).
                let tab_key = matches!(action, Action::Edit(Edit::Indent)) && tab.content.cursor().selection.is_none();
                let action = if tab_key {
                    Action::Edit(Edit::Paste(Arc::new(" ".repeat(to_tab_stop(&tab.content)))))
                } else {
                    action
                };
                tab.undo.record(&tab.content, &action);
                if action.is_edit() {
                    tab.dirty = true;
                }
                let before = tab.content.cursor();
                let times = match action {
                    Action::Edit(Edit::Backspace) => match tab_run(tab) {
                        Some(n) => {
                            tab.tab_runs.pop();
                            n
                        }
                        None => {
                            tab.tab_runs.clear();
                            backspaces(&tab.content)
                        }
                    },
                    _ if tab_key => 1,
                    Action::Scroll { .. } => 1,
                    _ => {
                        tab.tab_runs.clear();
                        1
                    }
                };
                for _ in 0..times {
                    tab.content.perform(action.clone());
                }
                if tab_key {
                    let (from, to) = (before.position, tab.content.cursor().position);
                    tab.tab_runs.push((from.line, from.column, to.column));
                }
            }
        }
        m @ (Msg::Undo | Msg::Redo) => {
            if let Some(tab) = app.hdl.tabs.get_mut(app.hdl.active) {
                tab.tab_runs.clear();
                let done = match m {
                    Msg::Undo => tab.undo.undo(&mut tab.content),
                    _ => tab.undo.redo(&mut tab.content),
                };
                tab.dirty |= done;
            }
        }
        Msg::DeleteWord(forward) => {
            if let Some(tab) = app.hdl.tabs.get_mut(app.hdl.active) {
                use text_editor::{Action, Edit, Motion};
                tab.tab_runs.clear();
                tab.undo.checkpoint(&tab.content);
                if tab.content.selection().is_none() {
                    tab.content.perform(Action::Select(if forward { Motion::WordRight } else { Motion::WordLeft }));
                }
                tab.content.perform(Action::Edit(Edit::Delete));
                tab.dirty = true;
            }
        }
        Msg::ScrollTo(line) => {
            if let Some(tab) = app.hdl.tabs.get_mut(app.hdl.active) {
                let lines = (line - tab.content.scroll().0).round() as i32;
                if lines != 0 {
                    tab.content.perform(text_editor::Action::Scroll { lines });
                }
            }
        }
        Msg::Save => {
            let i = app.hdl.active;
            if save_tab(app, i) {
                return files_changed(app);
            }
        }
        Msg::Reload => {
            if let Some(tab) = app.hdl.tabs.get_mut(app.hdl.active) {
                reload(tab);
            }
        }
        Msg::KeepMine => {
            if let Some(tab) = app.hdl.tabs.get_mut(app.hdl.active) {
                tab.changed_on_disk = false;
            }
        }
        Msg::OpenExternal => {
            if let Some(tab) = app.hdl.active_tab() {
                open_external(&tab.path.display().to_string());
            }
        }
        Msg::GetTools => open_external(TOOLS_URL),
        Msg::ShowTools => {
            find_tools(app);
            app.modal = Some(crate::app::Modal::HdlTools);
        }
        Msg::Compile => return start(app, Action::Compile),
        Msg::Synthesize => {
            // Nothing chosen yet: every entity and module of the sources.
            if app.project.hdl.tops.is_empty() {
                sync(app);
                let mut cfg = app.project.hdl.clone();
                for u in app.hdl.files.iter().flat_map(|f| &f.units) {
                    if !cfg.tops.contains(&u.name) && app.project.circuit(&u.name).is_none_or(|c| c.hdl.is_some()) {
                        cfg.tops.push(u.name.clone());
                    }
                }
                set_config(app, cfg, "Add all entities");
            }
            return start(app, Action::Synthesize);
        }
        Msg::Cancel => {
            if let Some((_, c)) = &app.hdl.job {
                c.cancel();
            }
            app.hdl.queued = false;
        }
        Msg::Done(epoch, outcome) => {
            if epoch != app.hdl.epoch {
                app.hdl.job = None;
                return Task::none();
            }
            return done(app, &outcome);
        }
        Msg::GoTo(i) => {
            let Some(d) = app.hdl.log.get(i) else { return Task::none() };
            if let (Some(file), Some(line)) = (d.file.clone(), d.line) {
                let col = d.col.unwrap_or(1);
                if let Some(path) = source_for(app, &file) {
                    return update(app, Msg::OpenAt(path, line, col));
                }
            }
        }
        Msg::ClearLog => app.hdl.log.clear(),
        Msg::SetGhdl(p) => {
            let settings = crate::settings::Settings { ghdl: p, ..app.settings.clone() };
            app.change_settings(settings);
            find_tools(app);
        }
        Msg::SetYosys(p) => {
            let settings = crate::settings::Settings { yosys: p, ..app.settings.clone() };
            app.change_settings(settings);
            find_tools(app);
        }
        Msg::Poll => return poll(app),
    }
    Task::none()
}

/// Checks the files every second while the feature is in use.
pub fn subscription(app: &App) -> Subscription<Message> {
    let used = !app.project.hdl.sources.is_empty() || !app.hdl.tabs.is_empty();
    if app.settings.hdl && used {
        iced::time::every(Duration::from_secs(1)).map(|_| msg(Msg::Poll))
    } else {
        Subscription::none()
    }
}

// ---- sources ---------------------------------------------------------------------

/// The folder sources are relative to: the project's, or the current one
/// for a project never saved.
fn base_dir(path: Option<&Path>) -> PathBuf {
    match path.and_then(Path::parent) {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => std::env::current_dir().unwrap_or_default(),
    }
}

fn resolve(base: &Path, entry: &str) -> PathBuf {
    let p = Path::new(entry);
    if p.is_absolute() { p.to_path_buf() } else { base.join(p) }
}

/// How a file is written in the project: relative to its folder when inside
/// it (with `/` separators), absolute otherwise.
fn entry_for(base: &Path, saved: bool, path: &Path) -> String {
    match path.strip_prefix(base) {
        Ok(rel) if saved => rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"),
        _ => path.display().to_string(),
    }
}

/// Rewrites the source entries for a project moving from `old` to `new`
/// (Save as), keeping them pointing at the same files.
pub fn rebase(project: &mut Project, old: Option<&Path>, new: &Path) {
    let (from, to) = (base_dir(old), base_dir(Some(new)));
    for s in &mut project.hdl.sources {
        *s = entry_for(&to, true, &resolve(&from, s));
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// `name` made a legal VHDL and Verilog identifier.
fn identifier(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if !s.starts_with(|c: char| c.is_ascii_alphabetic()) {
        s.insert_str(0, "u_");
    }
    s
}

/// When a file was last modified, and its size: a change of either means
/// the file changed (some file systems keep times to the second only).
type Stamp = (SystemTime, u64);

fn stamp(p: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// Changes the HDL settings of the project as one undoable step.
fn set_config(app: &mut App, cfg: HdlConfig, label: &str) {
    if cfg == app.project.hdl {
        return;
    }
    let mut cmd = Command::new(label, app.circuit().name.clone());
    cmd.push(Op::SetHdl { before: Box::new(app.project.hdl.clone()), after: Box::new(cfg) });
    app.exec(cmd);
}

fn add_sources(app: &mut App, files: &[PathBuf]) {
    let base = base_dir(app.path.as_deref());
    let mut cfg = app.project.hdl.clone();
    for f in files {
        let entry = entry_for(&base, app.path.is_some(), f);
        let known = cfg.sources.iter().any(|s| resolve(&base, s) == *f);
        if !known {
            cfg.sources.push(entry);
        }
    }
    let n = cfg.sources.len() - app.project.hdl.sources.len();
    set_config(app, cfg, if n == 1 { "Add source" } else { "Add sources" });
}

/// Brings the list of sources, their units, the digest and the out-of-date
/// flags up to date with the project and the disk. Cheap when nothing
/// changed; called after every edit.
pub fn sync(app: &mut App) {
    if !app.settings.hdl {
        return;
    }
    let cfg = &app.project.hdl;
    let seen = (cfg.sources.clone(), cfg.vhdl_std, app.path.clone());
    let base = base_dir(app.path.as_deref());
    let mut changed = app.hdl.seen.as_ref() != Some(&seen);
    if changed {
        app.hdl.files = seen
            .0
            .iter()
            .map(|e| {
                let path = resolve(&base, e);
                SourceFile {
                    entry: e.clone(),
                    lang: Lang::of(&path),
                    path,
                    units: Vec::new(),
                    stamp: None,
                    missing: true,
                }
            })
            .collect();
        app.hdl.seen = Some(seen);
        find_tools(app);
    }
    for f in &app.hdl.files {
        let m = stamp(&f.path);
        if m != f.stamp || (m.is_none() && !f.missing) {
            changed = true;
        }
    }
    if changed {
        let mut contents = Vec::new();
        for f in &mut app.hdl.files {
            f.stamp = stamp(&f.path);
            let data = std::fs::read(&f.path).ok();
            f.missing = data.is_none();
            f.units = match (&data, f.lang) {
                (Some(d), Some(lang)) => units(&String::from_utf8_lossy(d), lang),
                _ => Vec::new(),
            };
            contents.push((f.entry.clone(), data));
        }
        app.hdl.digest = lb_hdl::digest(app.project.hdl.vhdl_std, &contents);
    }
    mark_stale(app);
}

/// Sets the out-of-date flag of every synthesized circuit.
fn mark_stale(app: &mut App) {
    let digest = app.hdl.digest.clone();
    let mut any = false;
    for c in &mut app.project.circuits {
        if let Some(h) = &mut c.hdl {
            let stale = h.digest != digest;
            any |= h.stale != stale;
            h.stale = stale;
        }
    }
    if any {
        app.redraw();
    }
}

/// Whether some synthesized circuit is out of date.
pub fn any_stale(project: &Project) -> bool {
    project.circuits.iter().any(|c| c.hdl.as_ref().is_some_and(|h| h.stale))
}

/// The project was replaced (new or opened): forget its files.
pub fn project_loaded(app: &mut App) {
    if let Some((_, c)) = &app.hdl.job {
        c.cancel();
    }
    app.hdl.queued = false;
    app.hdl.epoch += 1;
    app.hdl.tabs.clear();
    app.hdl.active = 0;
    app.hdl.log.clear();
    app.hdl.seen = None;
    sync(app);
}

/// The source a tool message refers to: the same path, or a source with the
/// same file name (tools may print shortened paths).
fn source_for(app: &App, file: &Path) -> Option<PathBuf> {
    let files = &app.hdl.files;
    files
        .iter()
        .find(|f| f.path == file)
        .or_else(|| files.iter().find(|f| f.path.ends_with(file) || f.path.file_name() == file.file_name()))
        .map(|f| f.path.clone())
}

// ---- tabs ------------------------------------------------------------------------

fn open(app: &mut App, path: &Path) {
    app.hdl.panel = true;
    if let Some(i) = app.hdl.tabs.iter().position(|t| t.path == path) {
        app.hdl.active = i;
        return;
    }
    match std::fs::read_to_string(path) {
        Ok(text) => {
            app.hdl.tabs.push(Tab {
                path: path.to_path_buf(),
                lang: Lang::of(path),
                content: text_editor::Content::with_text(&text),
                dirty: false,
                stamp: stamp(path),
                changed_on_disk: false,
                undo: undo::Undo::default(),
                tab_runs: Vec::new(),
            });
            app.hdl.active = app.hdl.tabs.len() - 1;
        }
        Err(e) => app.warn(format!("Cannot open {}: {e}", path.display())),
    }
}

/// Width of an indentation step in the source editor (the editor's tab
/// width, see vendor/README.md).
const TAB: usize = 4;

/// Spaces from the cursor to the next tab stop.
fn to_tab_stop(content: &text_editor::Content) -> usize {
    let cursor = content.cursor().position;
    let column = content.line(cursor.line).and_then(|l| Some(l.text.get(..cursor.column)?.chars().count()));
    TAB - column.unwrap_or(0) % TAB
}

/// The length of the spaces of the last Tab, when the cursor is right after
/// them and they are still there.
fn tab_run(tab: &Tab) -> Option<usize> {
    let &(line, from, to) = tab.tab_runs.last()?;
    let cursor = tab.content.cursor();
    if cursor.selection.is_some() || (cursor.position.line, cursor.position.column) != (line, to) {
        return None;
    }
    let text = tab.content.line(line)?.text;
    text.get(from..to)?.bytes().all(|b| b == b' ').then_some(to - from)
}

/// Characters a Backspace deletes: back to the previous tab stop when only
/// spaces are in between, as if they were a tab; otherwise one.
fn backspaces(content: &text_editor::Content) -> usize {
    let cursor = content.cursor();
    if cursor.selection.is_some() {
        return 1;
    }
    let Some(line) = content.line(cursor.position.line) else { return 1 };
    let Some(before) = line.text.get(..cursor.position.column) else { return 1 };
    let spaces = before.len() - before.trim_end_matches(' ').len();
    let n = match before.chars().count() % TAB {
        0 => TAB,
        n => n,
    };
    if spaces >= n { n } else { 1 }
}

fn reload(tab: &mut Tab) {
    if let Ok(text) = std::fs::read_to_string(&tab.path) {
        let cursor = tab.content.cursor();
        tab.undo = undo::Undo::default();
        tab.content = text_editor::Content::with_text(&text);
        tab.content.move_to(text_editor::Cursor { selection: None, ..cursor });
        tab.dirty = false;
    }
    tab.tab_runs.clear();
    tab.stamp = stamp(&tab.path);
    tab.changed_on_disk = false;
}

/// Writes a tab to its file; false (with a message) when it cannot.
fn save_tab(app: &mut App, i: usize) -> bool {
    let Some(tab) = app.hdl.tabs.get_mut(i) else { return false };
    match std::fs::write(&tab.path, tab.content.text()) {
        Ok(()) => {
            tab.dirty = false;
            tab.changed_on_disk = false;
            tab.stamp = stamp(&tab.path);
            true
        }
        Err(e) => {
            let p = tab.path.display().to_string();
            app.warn(format!("Cannot save {p}: {e}"));
            false
        }
    }
}

/// Saves every edited source (when the project is saved, or before a job).
pub fn save_all(app: &mut App) {
    for i in 0..app.hdl.tabs.len() {
        if app.hdl.tabs[i].dirty {
            save_tab(app, i);
        }
    }
}

/// Reacts to sources changed on disk: refreshes, then synthesizes again
/// when asked to in the settings.
fn files_changed(app: &mut App) -> Task<Message> {
    let before = app.hdl.digest.clone();
    sync(app);
    let auto = app.settings.hdl_auto && !app.project.hdl.tops.is_empty() && any_stale(&app.project);
    if app.hdl.digest != before && auto { start(app, Action::Synthesize) } else { Task::none() }
}

fn poll(app: &mut App) -> Task<Message> {
    for tab in &mut app.hdl.tabs {
        let m = stamp(&tab.path);
        if m != tab.stamp {
            if tab.dirty {
                tab.changed_on_disk = true;
                tab.stamp = m;
            } else {
                reload(tab);
            }
        }
    }
    files_changed(app)
}

// ---- jobs ------------------------------------------------------------------------

pub fn find_tools(app: &mut App) {
    let (ghdl_set, yosys_set) = (app.settings.ghdl.clone(), app.settings.yosys.clone());
    let mut ghdl = toolchain::find("ghdl", &ghdl_set);
    let mut yosys = toolchain::find("yosys", &yosys_set);
    // The two come together (OSS CAD Suite): one found shows where the other is.
    let dir = |p: &Option<PathBuf>| p.as_ref().and_then(|p| p.parent()).map(|d| d.display().to_string());
    if ghdl.is_none() && ghdl_set.trim().is_empty() {
        ghdl = dir(&yosys).and_then(|d| toolchain::find("ghdl", &d));
    }
    if yosys.is_none() && yosys_set.trim().is_empty() {
        yosys = dir(&ghdl).and_then(|d| toolchain::find("yosys", &d));
    }
    app.hdl.ghdl = ghdl;
    app.hdl.yosys = yosys;
}

/// Folder for the GHDL library and the Yosys files of this project, in the
/// user cache.
fn workdir(app: &App) -> PathBuf {
    let cache = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    };
    let key = app.path.as_ref().map_or_else(|| "untitled".to_string(), |p| p.display().to_string());
    let id = lb_hdl::digest(VhdlStd::V08, &[(key, None)]);
    cache.unwrap_or_else(std::env::temp_dir).join("logibuild").join("hdl").join(id)
}

fn start(app: &mut App, action: Action) -> Task<Message> {
    if app.hdl.job.is_some() {
        app.hdl.queued |= action == Action::Synthesize;
        return Task::none();
    }
    save_all(app);
    sync(app);
    find_tools(app);
    let job = Job {
        sources: app.hdl.files.iter().map(|f| f.path.clone()).collect(),
        tops: app.project.hdl.tops.clone(),
        std: app.project.hdl.vhdl_std,
        workdir: workdir(app),
        ghdl: app.hdl.ghdl.clone(),
        yosys: app.hdl.yosys.clone(),
        digest: app.hdl.digest.clone(),
    };
    let cancel = Cancel::default();
    app.hdl.job = Some((action, cancel.clone()));
    let epoch = app.hdl.epoch;
    let what = if action == Action::Compile { "Compiling…" } else { "Synthesizing…" };
    app.hdl.log = vec![Diagnostic::info(what)];
    Task::perform(
        async move {
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let _ = tx.send(toolchain::run(&job, action, &cancel));
            });
            rx.await.unwrap_or_default()
        },
        move |o| msg(Msg::Done(epoch, Arc::new(o))),
    )
}

fn done(app: &mut App, outcome: &Outcome) -> Task<Message> {
    app.hdl.job = None;
    app.hdl.log = outcome.log.clone();
    if !outcome.ok && !outcome.cancelled {
        app.hdl.panel = true;
    } else if let Some(imported) = &outcome.imported {
        apply(app, imported.circuits.clone());
    }
    if std::mem::take(&mut app.hdl.queued) {
        return start(app, Action::Synthesize);
    }
    Task::none()
}

/// Puts synthesized circuits into the project as one undoable step:
/// replacing the previous version of each, adding the new ones.
fn apply(app: &mut App, circuits: Vec<lb_core::circuit::Circuit>) {
    let mut cmd = Command::new("Synthesize", app.circuit().name.clone());
    // The project as the command leaves it, for the indices of the operations.
    let mut after = app.project.clone();
    // Synthesized circuits no longer produced (an entity renamed or removed
    // from the sources) go too, unless a circuit still uses them.
    let fresh: HashSet<&str> = circuits.iter().map(|c| c.name.as_str()).collect();
    let mut gone: HashSet<String> = after
        .circuits
        .iter()
        .filter(|c| c.hdl.is_some() && !fresh.contains(c.name.as_str()))
        .map(|c| c.name.clone())
        .collect();
    loop {
        let used: HashSet<&str> = after
            .circuits
            .iter()
            .filter(|c| !gone.contains(&c.name) && !fresh.contains(c.name.as_str()))
            .chain(circuits.iter())
            .flat_map(|c| c.subcircuit_refs())
            .collect();
        let before = gone.len();
        gone.retain(|g| !used.contains(g.as_str()));
        if gone.len() == before {
            break;
        }
    }
    for i in (0..after.circuits.len()).rev() {
        if gone.contains(&after.circuits[i].name) {
            cmd.push(Op::RemoveCircuit(i, Box::new(after.circuits.remove(i))));
        }
    }
    let mut added = Vec::new();
    for c in circuits {
        match after.circuit_index(&c.name) {
            Some(i) if after.circuits[i].hdl.is_none() => {
                app.hdl.log.push(Diagnostic::error(format!(
                    "The project already has a circuit named “{}”: rename it to import the entity.",
                    c.name
                )));
                app.hdl.panel = true;
                return;
            }
            Some(i) => {
                let old = &after.circuits[i];
                let same = old.components == c.components && old.wires == c.wires;
                if !same || old.hdl.as_ref().map(|h| &h.digest) != c.hdl.as_ref().map(|h| &h.digest) {
                    cmd.push(Op::RemoveCircuit(i, Box::new(old.clone())));
                    cmd.push(Op::AddCircuit(i, Box::new(c)));
                }
            }
            None => {
                added.push(c.name.clone());
                let at = after.circuits.len() + added.len() - 1;
                cmd.push(Op::AddCircuit(at, Box::new(c)));
            }
        }
    }
    if !cmd.is_empty() {
        app.exec(cmd);
    }
}

/// Opens a file or URL with the desktop's default application.
fn open_external(target: &str) {
    let r = if cfg!(windows) {
        std::process::Command::new("cmd").args(["/C", "start", "", target]).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(target).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(target).spawn()
    };
    let _ = r;
}

#[cfg(test)]
mod tests {
    use lb_hdl::log::Level;

    use super::*;

    #[test]
    fn entries_relative_to_the_project() {
        let base = Path::new("/home/u/proj");
        assert_eq!(entry_for(base, true, Path::new("/home/u/proj/hdl/alu.vhd")), "hdl/alu.vhd");
        assert_eq!(entry_for(base, true, Path::new("/opt/lib/uart.v")), "/opt/lib/uart.v");
        assert_eq!(entry_for(base, false, Path::new("/home/u/proj/a.vhd")), "/home/u/proj/a.vhd", "unsaved project");
        assert_eq!(resolve(base, "hdl/alu.vhd"), Path::new("/home/u/proj/hdl/alu.vhd"));

        let mut p = Project::default();
        p.hdl.sources = vec!["hdl/alu.vhd".into(), "/opt/lib/uart.v".into(), "/home/u/new/x.vhd".into()];
        rebase(&mut p, Some(Path::new("/home/u/proj/a.lbc")), Path::new("/home/u/new/a.lbc"));
        assert_eq!(p.hdl.sources, ["/home/u/proj/hdl/alu.vhd", "/opt/lib/uart.v", "x.vhd"]);
    }

    fn counter_json() -> String {
        r#"{"modules": {"counter": {
            "ports": {"clk": {"direction": "input", "bits": [2]}, "q": {"direction": "output", "bits": [3, 4]}},
            "cells": {
                "r": {"hide_name": 1, "type": "$dff", "parameters": {"WIDTH": 2, "CLK_POLARITY": 1},
                      "connections": {"CLK": [2], "D": [5, 6], "Q": [3, 4]}},
                "a": {"hide_name": 1, "type": "$add", "parameters": {"A_WIDTH": 2, "B_WIDTH": 1, "Y_WIDTH": 2},
                      "connections": {"A": [3, 4], "B": ["1"], "Y": [5, 6]}}
            },
            "netnames": {}
        }}}"#
            .into()
    }

    /// An application with HDL enabled and one source, `counter.vhd`.
    fn app_with_source(dir: &Path) -> (App, PathBuf) {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let src = dir.join("counter.vhd");
        std::fs::write(&src, "entity counter is end;\n").unwrap();
        let mut app = App::new(None, false, None).0;
        let _ = app.update(Message::SetHdlEnabled(true));
        let _ = update(&mut app, Msg::FilesPicked(vec![src.clone()]));
        assert_eq!(app.hdl.files.len(), 1);
        assert_eq!(app.hdl.files[0].units[0].name, "counter");
        assert_eq!(app.hdl.tabs.len(), 1, "the new source is opened");
        (app, src)
    }

    fn synthesized(app: &mut App) {
        let design = lb_hdl::json::Design::parse(&counter_json()).unwrap();
        let imported = lb_hdl::import::import(&design, &[], &app.hdl.digest).unwrap();
        let outcome = Outcome { ok: true, imported: Some(imported), ..Outcome::default() };
        let epoch = app.hdl.epoch;
        let _ = update(app, Msg::Done(epoch, Arc::new(outcome)));
    }

    #[test]
    fn synthesis_is_undoable_read_only_and_tracks_its_sources() {
        let dir = std::env::temp_dir().join(format!("lb_app_hdl_{}", std::process::id()));
        let (mut app, src) = app_with_source(&dir);
        let mut cfg = app.project.hdl.clone();
        cfg.tops.push("counter".into());
        set_config(&mut app, cfg, "Add counter");
        synthesized(&mut app);
        let c = app.project.circuit("counter").expect("synthesized");
        assert!(!c.hdl.as_ref().unwrap().stale);
        let n = c.components.len();

        // Read-only: editing commands are refused.
        let _ = app.update(Message::OpenCircuit("counter".into()));
        let ids: Vec<u32> = app.circuit().components.iter().map(|c| c.id).collect();
        app.selection.comps.extend(ids);
        let _ = app.update(Message::Delete);
        assert_eq!(app.project.circuit("counter").unwrap().components.len(), n);

        // Changed sources: out of date; a failed synthesis keeps it.
        std::fs::write(&src, "entity counter is end; -- changed\n").unwrap();
        let _ = update(&mut app, Msg::Poll);
        assert!(app.project.circuit("counter").unwrap().hdl.as_ref().unwrap().stale);
        assert!(any_stale(&app.project));
        let epoch = app.hdl.epoch;
        let _ = update(&mut app, Msg::Done(epoch, Arc::new(Outcome::default())));
        assert!(app.project.circuit("counter").is_some_and(|c| c.hdl.as_ref().unwrap().stale));
        assert!(app.hdl.panel, "the log is shown after a failure");
        synthesized(&mut app);
        assert!(!any_stale(&app.project));

        // One undo step removes the synthesized circuits.
        let _ = app.update(Message::Undo);
        let _ = app.update(Message::Undo);
        assert!(app.project.circuit("counter").is_none());
        let _ = app.update(Message::Redo);
        assert!(app.project.circuit("counter").is_some());

        // Converted, it can be edited and is no longer synthesized.
        let _ = app.update(Message::ConvertCircuit("counter".into()));
        assert!(app.project.circuit("counter").unwrap().hdl.is_none());
        assert!(app.project.hdl.tops.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn synthesizing_without_tops_takes_every_entity() {
        let dir = std::env::temp_dir().join(format!("lb_app_hdl_all_{}", std::process::id()));
        let (mut app, _) = app_with_source(&dir);
        assert!(app.project.hdl.tops.is_empty());
        let _ = update(&mut app, Msg::Synthesize);
        assert_eq!(app.project.hdl.tops, ["counter"]);
        assert!(app.hdl.running().is_some());
        let _ = update(&mut app, Msg::Cancel);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn circuits_no_longer_synthesized_are_removed_unless_used() {
        let dir = std::env::temp_dir().join(format!("lb_app_hdl_gone_{}", std::process::id()));
        let (mut app, _) = app_with_source(&dir);
        let old = |name: &str| {
            let mut c = lb_core::circuit::Circuit::new(name);
            c.hdl = Some(lb_core::circuit::HdlInfo { digest: "old".into(), stale: true });
            c
        };
        // `old_used` is placed in the main circuit, `old_free` nowhere.
        app.project.circuits.push(old("old_used"));
        app.project.circuits.push(old("old_free"));
        let main = app.project.main().name.clone();
        let id = app.project.circuits[0].add(lb_core::component::Kind::Subcircuit, (0, 0));
        app.project.circuit_mut(&main).unwrap().component_mut(id).unwrap().props.circuit = "old_used".into();
        synthesized(&mut app);
        assert!(app.project.circuit("old_free").is_none());
        assert!(app.project.circuit("old_used").is_some());
        assert!(app.project.circuit("counter").is_some());
        // One undo step brings everything back.
        let _ = app.update(Message::Undo);
        assert!(app.project.circuit("old_free").is_some());
        assert!(app.project.circuit("counter").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn drawn_circuits_are_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("lb_app_hdl_name_{}", std::process::id()));
        let (mut app, _) = app_with_source(&dir);
        let _ = app.update(Message::NewCircuit);
        let _ = app.update(Message::RenameCircuit("counter".into()));
        assert!(app.project.circuit("counter").is_some());
        synthesized(&mut app);
        assert!(app.project.circuit("counter").unwrap().hdl.is_none(), "kept as drawn");
        assert!(app.hdl.log.iter().any(|d| d.level == Level::Error && d.text.contains("already has a circuit")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edited_sources_are_saved_and_reload_when_unchanged() {
        let dir = std::env::temp_dir().join(format!("lb_app_hdl_tabs_{}", std::process::id()));
        let (mut app, src) = app_with_source(&dir);
        let _ = update(&mut app, Msg::Edit(text_editor::Action::Edit(text_editor::Edit::Insert('x'))));
        assert!(app.hdl.has_unsaved() && app.is_dirty());
        // Changed on disk while edited: the user decides.
        std::fs::write(&src, "entity counter is end; -- other editor\n").unwrap();
        let _ = update(&mut app, Msg::Poll);
        assert!(app.hdl.tabs[0].changed_on_disk);
        let _ = update(&mut app, Msg::Reload);
        assert!(!app.hdl.has_unsaved());
        assert!(app.hdl.tabs[0].content.text().contains("other editor"));
        let _ = update(&mut app, Msg::Edit(text_editor::Action::Edit(text_editor::Edit::Insert('y'))));
        let _ = update(&mut app, Msg::Save);
        let saved = std::fs::read_to_string(&src).unwrap();
        assert!(saved.contains('y') && saved.contains("other editor"));
        assert_eq!(saved, app.hdl.tabs[0].content.text());
        // Closing an edited tab asks once.
        let _ = update(&mut app, Msg::Edit(text_editor::Action::Edit(text_editor::Edit::Insert('z'))));
        let _ = update(&mut app, Msg::CloseTab(0));
        assert_eq!(app.hdl.tabs.len(), 1);
        let _ = update(&mut app, Msg::CloseTab(0));
        assert!(app.hdl.tabs.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn editor_keys_indent_delete_words_and_undo() {
        use text_editor::{Action, Cursor, Edit, Position};
        let dir = std::env::temp_dir().join(format!("lb_app_hdl_keys_{}", std::process::id()));
        let (mut app, _) = app_with_source(&dir);
        let text = |app: &App| app.hdl.tabs[0].content.text();
        let at = |app: &mut App, column| {
            let position = Position { line: 0, column };
            app.hdl.tabs[0].content.move_to(Cursor { position, selection: None });
        };
        // "entity counter is end;": Ctrl+Backspace after "counter".
        at(&mut app, 14);
        let _ = update(&mut app, Msg::DeleteWord(false));
        assert_eq!(text(&app), "entity  is end;\n");
        assert!(app.hdl.has_unsaved());
        // Tab: four spaces at the start of the line.
        at(&mut app, 0);
        let _ = update(&mut app, Msg::Edit(Action::Edit(Edit::Indent)));
        assert_eq!(text(&app), "    entity  is end;\n");
        // Backspace after spaces goes back to the previous tab stop.
        at(&mut app, 4);
        let _ = update(&mut app, Msg::Edit(Action::Edit(Edit::Indent)));
        let _ = update(&mut app, Msg::Edit(Action::Edit(Edit::Insert(' '))));
        assert_eq!(text(&app), format!("{}entity  is end;\n", " ".repeat(9)));
        let _ = update(&mut app, Msg::Edit(Action::Edit(Edit::Backspace)));
        assert_eq!(text(&app), format!("{}entity  is end;\n", " ".repeat(8)));
        let _ = update(&mut app, Msg::Edit(Action::Edit(Edit::Backspace)));
        assert_eq!(text(&app), "    entity  is end;\n");
        let _ = update(&mut app, Msg::Edit(Action::Edit(Edit::Backspace)));
        assert_eq!(text(&app), "entity  is end;\n");
        // One undo step for the run of Backspaces.
        let _ = update(&mut app, Msg::Undo);
        assert_eq!(text(&app), format!("{}entity  is end;\n", " ".repeat(9)));
        let _ = update(&mut app, Msg::Undo);
        let _ = update(&mut app, Msg::Undo);
        let _ = update(&mut app, Msg::Undo);
        let _ = update(&mut app, Msg::Undo);
        assert_eq!(text(&app), "entity counter is end;\n");
        let _ = update(&mut app, Msg::Redo);
        assert_eq!(text(&app), "entity  is end;\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backspace_takes_back_the_spaces_of_a_tab() {
        use text_editor::{Action, Cursor, Edit, Position};
        let dir = std::env::temp_dir().join(format!("lb_app_hdl_tab_runs_{}", std::process::id()));
        let (mut app, _) = app_with_source(&dir);
        let text = |app: &App| app.hdl.tabs[0].content.text();
        let at = |app: &mut App, column| {
            let position = Position { line: 0, column };
            app.hdl.tabs[0].content.move_to(Cursor { position, selection: None });
        };
        let key = |app: &mut App, edit| _ = update(app, Msg::Edit(Action::Edit(edit)));
        // "e|ntity": Tab adds 3 spaces, Backspace takes them back.
        at(&mut app, 1);
        key(&mut app, Edit::Indent);
        assert_eq!(text(&app), "e   ntity counter is end;\n");
        key(&mut app, Edit::Backspace);
        assert_eq!(text(&app), "entity counter is end;\n");
        // Two Tabs, two Backspaces.
        key(&mut app, Edit::Indent);
        key(&mut app, Edit::Indent);
        assert_eq!(text(&app), "e       ntity counter is end;\n");
        key(&mut app, Edit::Backspace);
        assert_eq!(text(&app), "e   ntity counter is end;\n");
        key(&mut app, Edit::Backspace);
        assert_eq!(text(&app), "entity counter is end;\n");
        // Typed spaces go one at a time.
        at(&mut app, 2);
        key(&mut app, Edit::Insert(' '));
        key(&mut app, Edit::Insert(' '));
        key(&mut app, Edit::Backspace);
        assert_eq!(text(&app), "en tity counter is end;\n");
        // After typing, the spaces of a Tab are ordinary spaces.
        at(&mut app, 1);
        key(&mut app, Edit::Indent);
        key(&mut app, Edit::Insert('x'));
        key(&mut app, Edit::Backspace);
        key(&mut app, Edit::Backspace);
        assert_eq!(text(&app), "e  n tity counter is end;\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_hdl_view_replaces_the_circuit_and_ignores_canvas_keys() {
        let dir = std::env::temp_dir().join(format!("lb_app_hdl_view_{}", std::process::id()));
        let (mut app, _) = app_with_source(&dir);
        assert!(app.hdl_shown(), "opening a source shows the HDL view");
        // Delete would act on the hidden circuit.
        let _ = app.update(Message::PickTool(lb_core::component::Kind::And));
        let _ = app.update(Message::Editor(crate::canvas::EditorEvent::Place { pos: (0, 0) }));
        let n = app.circuit().components.len();
        let del = iced::keyboard::Key::Named(iced::keyboard::key::Named::Delete);
        let _ = app.update(Message::Key(iced::keyboard::Event::KeyPressed {
            key: del.clone(),
            modified_key: del,
            physical_key: iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
            location: iced::keyboard::Location::Standard,
            modifiers: iced::keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        }));
        assert_eq!(app.circuit().components.len(), n);
        let _ = update(&mut app, Msg::TogglePanel);
        assert!(!app.hdl_shown());
        let _ = update(&mut app, Msg::ShowTools);
        assert_eq!(app.modal, Some(crate::app::Modal::HdlTools));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn background_messages_keep_menus_open() {
        let dir = std::env::temp_dir().join(format!("lb_app_hdl_menu_{}", std::process::id()));
        let (mut app, _) = app_with_source(&dir);
        let _ = app.update(Message::Menu(Some(crate::app::Menu::File)));
        let _ = app.update(Message::Hdl(Msg::Poll));
        let epoch = app.hdl.epoch;
        let _ = app.update(Message::Hdl(Msg::Done(epoch, Arc::new(Outcome::default()))));
        assert_eq!(app.menu, Some(crate::app::Menu::File));
        let _ = app.update(Message::Hdl(Msg::ClearLog));
        assert_eq!(app.menu, None, "a click elsewhere closes it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn view_shows_the_synthesized_circuit() {
        let dir = std::env::temp_dir().join(format!("lb_app_hdl_view_circuit_{}", std::process::id()));
        let (mut app, _) = app_with_source(&dir);
        synthesized(&mut app);
        let _ = update(&mut app, Msg::View("counter".into()));
        assert!(!app.hdl_shown());
        assert_eq!(app.circuit().name, "counter");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn identifiers() {
        assert_eq!(identifier("my-alu 2"), "my_alu_2");
        assert_eq!(identifier("7seg"), "u_7seg");
    }
}
