//! The HDL view, shown instead of the circuit: sources and their entities
//! on the left, the source editor in the middle, the messages of the tools
//! on the right, with draggable boundaries; and the dialog with the paths of
//! the tools.

use iced::keyboard::Key;
use iced::keyboard::key::Named;
use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry};
use iced::widget::text_editor::{Binding, Edit, KeyPress, Status};
use iced::widget::{
    Space, button, canvas as canvas_widget, column, container, pick_list, row, stack, text, text_editor, text_input,
};
use iced::{Alignment, Element, Fill, Font, Padding, Point, Rectangle, Renderer, Size, Theme};

use lb_core::circuit::VhdlStd;
use lb_hdl::lang::Lang;
use lb_hdl::log::Level;
use lb_hdl::toolchain::Action;

use super::{Msg, highlight};
use crate::app::{App, Message, Split, splitter};
use crate::elide::elide;
use crate::icons::{self, Emblem, Icon};
use crate::theme;

fn msg(m: Msg) -> Message {
    Message::Hdl(m)
}

/// Small text button.
fn small<'a>(label: &str, m: Msg) -> button::Button<'a, Message> {
    button(text(label.to_string()).size(theme::fs(theme::SMALL))).on_press(msg(m)).padding([3, 8]).style(theme::tool)
}

/// The Compile / Synthesize (or Cancel) buttons, for the toolbar and the
/// panel header.
pub fn actions(app: &App) -> Vec<Element<'_, Message>> {
    let tb = |i: Icon, label: &str, m: Option<Msg>, tip: &str| -> Element<Message> {
        theme::tip(
            button(theme::icon_label(i, label, m.is_some()))
                .on_press_maybe(m.map(msg))
                .padding([5, 8])
                .style(theme::tool),
            tip,
        )
    };
    let has_sources = !app.project.hdl.sources.is_empty();
    match app.hdl.running() {
        Some(a) => {
            let what = if a == Action::Compile { "Compiling…" } else { "Synthesizing…" };
            vec![
                text(what).size(theme::fs(theme::BODY)).color(theme::colors().muted).into(),
                tb(Icon::Stop, "Cancel", Some(Msg::Cancel), "Stop the tools"),
            ]
        }
        None => vec![
            tb(
                Icon::Compile,
                "Compile",
                has_sources.then_some(Msg::Compile),
                "Check the HDL sources for errors (GHDL, Yosys)",
            ),
            tb(
                Icon::Chip,
                "Synthesize",
                has_sources.then_some(Msg::Synthesize),
                "Turn the HDL entities of the project into circuits (all of them when none was added)",
            ),
        ],
    }
}

/// The file actions and the VHDL revision, in the toolbar of the HDL view.
pub fn toolbar(app: &App) -> Vec<Element<'_, Message>> {
    let tbl = |i: Icon, label: &str, m: Msg| -> Element<Message> {
        button(theme::icon_label(i, label, true)).on_press(msg(m)).padding([5, 8]).style(theme::tool).into()
    };
    vec![
        tbl(Icon::Open, "Open files", Msg::AddFiles),
        tbl(Icon::New, "New VHDL", Msg::NewFile(Lang::Vhdl)),
        tbl(Icon::New, "New Verilog", Msg::NewFile(Lang::Verilog)),
        theme::tool_separator(),
        pick_list(&VhdlStd::ALL[..], Some(app.project.hdl.vhdl_std), |s| msg(Msg::SetStd(s)))
            .text_size(theme::fs(theme::BODY))
            .padding([4, 10])
            .style(theme::dropdown)
            .menu_style(theme::dropdown_menu)
            .handle(theme::dropdown_handle())
            .into(),
    ]
}

pub fn workspace(app: &App) -> Element<'_, Message> {
    let header = row![
        button(theme::icon_label(Icon::ChevronLeft, "Circuit", true))
            .on_press(msg(Msg::TogglePanel))
            .padding([4, 8])
            .style(theme::tool),
        Space::new().width(Fill),
    ]
    .extend(actions(app))
    .push(theme::tool_separator())
    .push(small("Tools…", Msg::ShowTools))
    .spacing(4)
    .align_y(Alignment::Center);
    column![
        container(header)
            .padding(Padding { top: 3.0, right: 6.0, bottom: 3.0, left: 6.0 })
            .height(theme::fs(34.0))
            .width(Fill)
            .align_y(Alignment::Center)
            .style(theme::window),
        theme::hline(),
        row![
            container(sources(app)).width(app.hdl.sources_w).height(Fill).style(theme::window),
            splitter(Split::HdlSources, app.is_resizing(Split::HdlSources)),
            container(editor(app)).width(Fill).height(Fill).style(theme::view),
            splitter(Split::HdlLog, app.is_resizing(Split::HdlLog)),
            container(log(app)).width(app.hdl.log_w).height(Fill).style(theme::window),
        ]
        .height(Fill),
    ]
    .width(Fill)
    .height(Fill)
    .into()
}

/// The paths of GHDL and Yosys, shown in a dialog from the settings.
pub fn tools(app: &App) -> Element<'_, Message> {
    let tool = |label: &'static str, value: &str, found: &Option<std::path::PathBuf>, on: fn(String) -> Msg| {
        let status: Element<Message> = match found {
            Some(p) => row![
                icons::icon(Icon::Check, 14.0, theme::colors().positive),
                muted(format!("Found: {}", p.display()), 11.0),
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .into(),
            None => row![
                icons::icon(Icon::Close, 14.0, theme::colors().negative),
                text("Not found").size(theme::fs(theme::TINY)).color(theme::colors().negative),
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .into(),
        };
        column![
            text(label).size(theme::fs(theme::BODY)).font(theme::FONT_MEDIUM),
            text_input("Empty: search the PATH and ~/oss-cad-suite/bin", value)
                .on_input(move |s| msg(on(s)))
                .size(theme::fs(theme::BODY))
                .padding([5, 8])
                .style(theme::input),
            status,
        ]
        .spacing(4)
    };
    let mut col = column![
        text("HDL tools").size(theme::fs(theme::TITLE)).font(theme::FONT_MEDIUM),
        text("The path of each program, or of the folder holding it, such as ~/oss-cad-suite/bin.")
            .size(theme::fs(theme::BODY)),
        tool("GHDL", &app.settings.ghdl, &app.hdl.ghdl, Msg::SetGhdl),
        tool("Yosys", &app.settings.yosys, &app.hdl.yosys, Msg::SetYosys),
    ]
    .spacing(12);
    if app.hdl.ghdl.is_none() || app.hdl.yosys.is_none() {
        col = col.push(
            button(text("Get GHDL and Yosys (OSS CAD Suite)").size(theme::fs(theme::BODY)))
                .on_press(msg(Msg::GetTools))
                .padding([2, 0])
                .style(theme::link),
        );
    }
    col.into()
}

fn muted<'a>(s: impl ToString, size: f32) -> text::Text<'a> {
    text(s.to_string()).size(theme::fs(size)).color(theme::colors().muted)
}

/// The header of a side panel: its title, a muted summary and a text action
/// on the right.
fn panel_header<'a>(title: &str, summary: String, action: &str, m: Msg) -> Element<'a, Message> {
    let c = theme::colors();
    let header = row![
        text(title.to_string()).size(theme::fs(theme::SMALL)).font(theme::FONT_MEDIUM),
        muted(summary, 11.0),
        Space::new().width(Fill),
        button(text(action.to_string()).size(theme::fs(theme::SMALL)))
            .on_press(msg(m))
            .padding([2, 4])
            .style(theme::bare(c.muted, c.text)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    container(header).padding([4, 8]).into()
}

/// The hint of an empty side panel.
fn placeholder<'a>(s: &str) -> Element<'a, Message> {
    container(muted(s, 12.0)).width(Fill).padding([8, 10]).into()
}

fn sources(app: &App) -> Element<'_, Message> {
    let c = theme::colors();
    let link = |label: &str, m: Msg| {
        button(text(label.to_string()).size(theme::fs(theme::SMALL)))
            .on_press(msg(m))
            .padding([0, 4])
            .style(theme::bare(c.link, c.accent))
    };
    let active = app.hdl.active_tab().map(|t| t.path.clone());
    let mut list = column![];
    if app.hdl.files.is_empty() {
        list = list.push(placeholder("No sources yet."));
    }
    for f in &app.hdl.files {
        let name = f.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.entry.clone());
        let lang = if f.missing { "missing" } else { f.lang.map_or("?", Lang::name) };
        let file = row![
            container(elide(name).size(theme::fs(theme::BODY)).color(if f.missing { c.negative } else { c.text }))
                .width(Fill),
            text(lang).size(theme::fs(theme::TINY)).color(if f.missing { c.negative } else { c.muted }),
        ]
        .align_y(Alignment::Center);
        list = list.push(
            button(file)
                .on_press(msg(Msg::Open(f.path.clone())))
                .width(Fill)
                .padding([4, 10])
                .style(theme::row_item(active.as_ref() == Some(&f.path))),
        );
        let mut units = column![];
        for u in &f.units {
            // The state of the circuit synthesized from the entity, and what
            // can be done with it.
            let view = || -> Element<Message> {
                theme::tip(link("View", Msg::View(u.name.clone())), "Show the synthesized circuit")
            };
            let (state, color, action) = match app.project.circuit(&u.name).map(|c| &c.hdl) {
                Some(Some(h)) if h.stale => ("old", c.neutral, view()),
                Some(Some(_)) => ("built", c.muted, view()),
                Some(None) => ("name taken", c.muted, Space::new().into()),
                None => ("", c.muted, link("Add", Msg::AddTop(u.name.clone())).into()),
            };
            // The actions are inside the row, so that the whole row lights up.
            let mut entity =
                row![container(elide(u.name.clone()).size(theme::fs(theme::SMALL)).font(Font::MONOSPACE)).width(Fill)]
                    .spacing(6)
                    .align_y(Alignment::Center);
            if !state.is_empty() {
                entity = entity.push(text(state).color(color).size(theme::fs(theme::TINY)));
            }
            let entity = entity.push(action);
            units = units.push(
                button(entity)
                    .on_press(msg(Msg::OpenAt(f.path.clone(), u.line, 1)))
                    .width(Fill)
                    .padding(Padding { top: 3.0, right: 4.0, bottom: 3.0, left: 26.0 })
                    .style(theme::row_item(false)),
            );
        }
        if !f.units.is_empty() {
            // Tree guide: a vertical line joining the entities of the file,
            // over the rows so that their highlight still spans the width.
            let guide = container(theme::vline()).padding(Padding { left: 16.0, ..Padding::ZERO }).height(Fill);
            list = list.push(stack![units, guide].width(Fill));
        }
    }
    let count = match app.hdl.files.len() {
        0 => String::new(),
        n => n.to_string(),
    };
    column![
        panel_header("Sources", count, "Add", Msg::AddFiles),
        theme::hline(),
        theme::scroll(container(list).padding(Padding { top: 4.0, ..Padding::ZERO })).height(Fill),
    ]
    .into()
}

fn editor(app: &App) -> Element<'_, Message> {
    let c = theme::colors();
    let hdl = &app.hdl;
    let Some(tab) = hdl.active_tab() else {
        return empty_editor(app);
    };
    let action = |label: &str, m: Msg| {
        button(text(label.to_string()).size(theme::fs(theme::SMALL)))
            .on_press(msg(m))
            .padding([4, 8])
            .style(theme::bare(c.muted, c.text))
    };
    // Classic tabs: the active one merges with the editor below.
    let mut tabs = row![];
    for (i, t) in hdl.tabs.iter().enumerate() {
        let active = i == hdl.active;
        let name = t.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let label = if t.dirty { format!("• {name}") } else { name };
        let fg = if active { c.text } else { c.muted };
        tabs = tabs.push(
            container(
                row![
                    button(text(label).size(theme::fs(theme::BODY)))
                        .on_press(msg(Msg::SelectTab(i)))
                        .padding(Padding { top: 6.0, right: 4.0, bottom: 6.0, left: 12.0 })
                        .style(theme::bare(fg, c.text)),
                    theme::tip(
                        button(text("×").size(theme::fs(14.0)))
                            .on_press(msg(Msg::CloseTab(i)))
                            .padding(Padding { top: 2.0, right: 10.0, bottom: 2.0, left: 4.0 })
                            .style(theme::bare(c.muted, c.negative)),
                        "Close",
                    ),
                ]
                .align_y(Alignment::Center),
            )
            .height(Fill)
            .align_y(Alignment::Center)
            .style(theme::tab(active)),
        );
        tabs = tabs.push(theme::vline());
    }
    let source = hdl.files.iter().position(|f| f.path == tab.path);
    let mut header = row![tabs, Space::new().width(Fill), action("Save", Msg::Save)];
    header = header.push(action("Open externally", Msg::OpenExternal));
    if let Some(i) = source {
        header = header.push(action("Remove", Msg::RemoveSource(i)));
    }
    let mut col = column![
        // Fixed height: the separators between tabs would stretch it.
        container(header.spacing(2).align_y(Alignment::Center))
            .width(Fill)
            .height(theme::fs(32.0))
            .style(theme::window),
        theme::hline(),
    ];
    if tab.changed_on_disk {
        col = col.push(
            container(theme::message(
                Emblem::Warning,
                row![
                    text("The file changed on disk.").size(theme::fs(theme::SMALL)).width(Fill),
                    theme::push_button("Reload").on_press(msg(Msg::Reload)),
                    theme::push_button("Keep my version").on_press(msg(Msg::KeepMine)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            ))
            .padding(4),
        );
    }
    let edit = text_editor(&tab.content)
        .on_action(|a| msg(Msg::Edit(a)))
        .font(Font::MONOSPACE)
        .size(theme::fs(theme::BODY))
        .height(Fill)
        .padding(Padding { top: 8.0, right: 4.0, bottom: 8.0, left: 12.0 })
        .style(theme::code_area)
        .key_binding(key_binding)
        .highlight_with::<highlight::Hdl>(tab.lang, highlight::format);
    let (top, visible, lines) = tab.content.scroll();
    let bar = canvas_widget(Scrollbar { top, visible, lines }).width(BAR_W).height(Fill);
    col.push(container(row![edit, bar]).height(Fill).style(theme::view)).height(Fill).into()
}

/// The keys of the source editor, besides those of iced.
fn key_binding(kp: KeyPress) -> Option<Binding<Message>> {
    let (ctrl, shift) = (kp.modifiers.command(), kp.modifiers.shift());
    if ctrl && matches!(kp.key.as_ref(), Key::Character("s")) {
        return Some(Binding::Custom(msg(Msg::Save)));
    }
    if matches!(kp.status, Status::Focused { .. }) {
        let m = match kp.key.as_ref() {
            Key::Character("z") if ctrl && !shift => Some(Msg::Undo),
            Key::Character("z") if ctrl => Some(Msg::Redo),
            Key::Character("y") if ctrl => Some(Msg::Redo),
            Key::Named(Named::Tab) if !ctrl => {
                Some(Msg::Edit(text_editor::Action::Edit(if shift { Edit::Unindent } else { Edit::Indent })))
            }
            Key::Named(Named::Backspace) if ctrl => Some(Msg::DeleteWord(false)),
            Key::Named(Named::Delete) if ctrl => Some(Msg::DeleteWord(true)),
            _ => None,
        };
        if let Some(m) = m {
            return Some(Binding::Custom(msg(m)));
        }
    }
    Binding::from_key_press(kp)
}

/// Width of the scrollbar of the source editor.
const BAR_W: f32 = 10.0;

/// The scrollbar of the source editor (the editor of iced has none), in
/// lines of text: the first one shown, how many fit, how many there are.
struct Scrollbar {
    top: f32,
    visible: f32,
    lines: usize,
}

/// While the handle is dragged: where it was taken, from its top.
#[derive(Default)]
struct Grab(Option<f32>);

impl Scrollbar {
    fn shown(&self) -> bool {
        self.visible > 0.0 && (self.lines as f32) > self.visible
    }

    /// The highest first line: the last line at the bottom.
    fn max_top(&self) -> f32 {
        (self.lines as f32 - self.visible).max(1.0)
    }

    /// The top and height of the handle in a bar `h` high.
    fn handle(&self, h: f32) -> (f32, f32) {
        let len = (h * self.visible / self.lines as f32).clamp(24.0_f32.min(h), h);
        ((h - len) * (self.top / self.max_top()).clamp(0.0, 1.0), len)
    }

    /// The first line that puts the handle at `y`, taken `grab` from its top.
    fn top_at(&self, y: f32, grab: f32, h: f32) -> f32 {
        let (_, len) = self.handle(h);
        ((y - grab) / (h - len).max(1.0)).clamp(0.0, 1.0) * self.max_top()
    }
}

impl canvas::Program<Message> for Scrollbar {
    type State = Grab;

    fn update(
        &self,
        grab: &mut Grab,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if !self.shown() {
            return None;
        }
        let scroll = |line: f32| canvas::Action::publish(msg(Msg::ScrollTo(line))).and_capture();
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let y = cursor.position_in(bounds)?.y;
                let (top, len) = self.handle(bounds.height);
                if (top..top + len).contains(&y) {
                    grab.0 = Some(y - top);
                    Some(canvas::Action::capture())
                } else {
                    // On the rail: the handle jumps there, centred.
                    grab.0 = Some(len / 2.0);
                    Some(scroll(self.top_at(y, len / 2.0, bounds.height)))
                }
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let g = grab.0?;
                Some(scroll(self.top_at(position.y - bounds.y, g, bounds.height)))
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                grab.0.take().map(|_| canvas::Action::capture())
            }
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => -y * 3.0,
                    mouse::ScrollDelta::Pixels { y, .. } => -y / 18.0,
                };
                Some(scroll(self.top + lines))
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        grab: &Grab,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        if !self.shown() {
            return Vec::new();
        }
        let mut frame = Frame::new(renderer, bounds.size());
        let (top, len) = self.handle(bounds.height);
        // As the other scrollbars (theme::scrollbar): thin, accent while used.
        let active = grab.0.is_some() || cursor.is_over(bounds);
        let color = if active { theme::colors().accent } else { theme::alpha(theme::colors().text, 0.28) };
        let w = 6.0;
        let handle = canvas::Path::rounded_rectangle(
            Point::new((bounds.width - w) / 2.0, top + 2.0),
            Size::new(w, (len - 4.0).max(w)),
            (w / 2.0).into(),
        );
        frame.fill(&handle, color);
        vec![frame.into_geometry()]
    }
}

/// The editor with no file open: what to do next.
fn empty_editor(app: &App) -> Element<'_, Message> {
    let c = theme::colors();
    let hint = if app.hdl.files.is_empty() {
        "Create a VHDL or Verilog file, or add existing ones to the project."
    } else {
        "Pick a source from the list, or add another one."
    };
    let act = |i: Icon, label: &str, m: Msg| theme::push_content(theme::icon_label(i, label, true)).on_press(msg(m));
    container(
        column![
            icons::icon(Icon::Chip, 48.0, c.disabled),
            Space::new().height(4),
            text("No file open").size(theme::fs(theme::TITLE)).font(theme::FONT_MEDIUM),
            muted(hint, 13.0),
            Space::new().height(8),
            row![
                act(Icon::Open, "Open files", Msg::AddFiles),
                act(Icon::New, "New VHDL", Msg::NewFile(Lang::Vhdl)),
                act(Icon::New, "New Verilog", Msg::NewFile(Lang::Verilog)),
            ]
            .spacing(8),
        ]
        .spacing(6)
        .align_x(Alignment::Center),
    )
    .center(Fill)
    .padding(16)
    .into()
}

fn log(app: &App) -> Element<'_, Message> {
    let errors = app.hdl.log.iter().filter(|d| d.level == Level::Error).count();
    let warnings = app.hdl.log.iter().filter(|d| d.level == Level::Warning).count();
    let summary = match (errors, warnings) {
        (0, 0) => String::new(),
        (e, w) => format!("{e} errors, {w} warnings"),
    };
    let mut lines = column![];
    if app.hdl.log.is_empty() {
        lines = lines.push(placeholder("The output of Compile and Synthesize appears here."));
    }
    for (i, d) in app.hdl.log.iter().enumerate() {
        let color = match d.level {
            Level::Error => theme::colors().negative,
            Level::Warning => theme::colors().neutral,
            Level::Info => theme::colors().muted,
        };
        let t = text(d.text.clone()).size(theme::fs(theme::SMALL)).font(Font::MONOSPACE).color(color);
        let line: Element<Message> = if d.file.is_some() && d.line.is_some() {
            theme::tip(
                button(t).on_press(msg(Msg::GoTo(i))).padding([1, 6]).width(Fill).style(theme::row_item(false)),
                "Go to the source",
            )
        } else {
            container(t).padding([1, 6]).into()
        };
        lines = lines.push(line);
    }
    column![
        panel_header("Messages", summary, "Clear", Msg::ClearLog),
        theme::hline(),
        theme::scroll(container(lines).padding([4, 0])).height(Fill),
    ]
    .into()
}
