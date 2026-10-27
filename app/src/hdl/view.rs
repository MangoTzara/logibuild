//! The HDL view, shown instead of the circuit: sources and their entities
//! on the left, the source editor in the middle, the messages of the tools
//! on the right, with draggable boundaries; and the dialog with the paths of
//! the tools.

use iced::keyboard::Key;
use iced::widget::text_editor::{Binding, KeyPress};
use iced::widget::{Space, button, column, container, pick_list, row, stack, text, text_editor, text_input};
use iced::{Alignment, Element, Fill, Font, Padding};

use lb_core::circuit::VhdlStd;
use lb_hdl::lang::Lang;
use lb_hdl::log::Level;
use lb_hdl::toolchain::Action;

use super::{Msg, highlight};
use crate::app::{App, Message, Split, splitter};
use crate::icons::{self, Emblem, Icon};
use crate::theme;

fn msg(m: Msg) -> Message {
    Message::Hdl(m)
}

/// Small text button.
fn small<'a>(label: &str, m: Msg) -> button::Button<'a, Message> {
    button(text(label.to_string()).size(theme::fs(12.0))).on_press(msg(m)).padding([3, 8]).style(theme::tool)
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
                text(what).size(theme::fs(13.0)).color(theme::colors().muted).into(),
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
        theme::tool_separator(),
        tbl(Icon::Open, "Open files…", Msg::AddFiles),
        tbl(Icon::New, "New VHDL file…", Msg::NewFile(Lang::Vhdl)),
        tbl(Icon::New, "New Verilog file…", Msg::NewFile(Lang::Verilog)),
        theme::tool_separator(),
        pick_list(&VhdlStd::ALL[..], Some(app.project.hdl.vhdl_std), |s| msg(Msg::SetStd(s)))
            .text_size(theme::fs(13.0))
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
                text("Not found").size(theme::fs(11.0)).color(theme::colors().negative),
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .into(),
        };
        column![
            text(label).size(theme::fs(13.0)).font(theme::FONT_MEDIUM),
            text_input("Empty: search the PATH and ~/oss-cad-suite/bin", value)
                .on_input(move |s| msg(on(s)))
                .size(theme::fs(13.0))
                .padding([5, 8])
                .style(theme::input),
            status,
        ]
        .spacing(4)
    };
    let mut col = column![
        text("HDL tools").size(theme::fs(15.0)).font(theme::FONT_MEDIUM),
        text("The path of each program, or of the folder holding it, such as ~/oss-cad-suite/bin.")
            .size(theme::fs(13.0)),
        tool("GHDL", &app.settings.ghdl, &app.hdl.ghdl, Msg::SetGhdl),
        tool("Yosys", &app.settings.yosys, &app.hdl.yosys, Msg::SetYosys),
    ]
    .spacing(12);
    if app.hdl.ghdl.is_none() || app.hdl.yosys.is_none() {
        col = col.push(
            button(text("Get GHDL and Yosys (OSS CAD Suite)").size(theme::fs(13.0)))
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

fn sources(app: &App) -> Element<'_, Message> {
    let c = theme::colors();
    let link = |label: &str, m: Msg| {
        button(text(label.to_string()).size(theme::fs(12.0)))
            .on_press(msg(m))
            .padding([0, 4])
            .style(theme::bare(c.link, c.accent))
    };
    let active = app.hdl.active_tab().map(|t| t.path.clone());
    let mut list = column![];
    if app.hdl.files.is_empty() {
        list = list
            .push(container(muted("No sources yet: add VHDL or Verilog files, or create one.", 12.0)).padding([6, 10]));
    }
    for f in &app.hdl.files {
        let name = f.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.entry.clone());
        let lang = if f.missing { "missing" } else { f.lang.map_or("?", Lang::name) };
        let file = row![
            text(name).size(theme::fs(13.0)).color(if f.missing { c.negative } else { c.text }),
            Space::new().width(Fill),
            text(lang).size(theme::fs(11.0)).color(if f.missing { c.negative } else { c.muted }),
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
                Some(Some(h)) if h.stale => ("outdated", c.neutral, view()),
                Some(Some(_)) => ("synthesized", c.muted, view()),
                Some(None) => ("name taken", c.muted, Space::new().into()),
                None => ("", c.muted, link("Add", Msg::AddTop(u.name.clone())).into()),
            };
            let state = text(state).color(color);
            // The actions are inside the row, so that the whole row lights up.
            let entity = row![
                text(u.name.clone()).size(theme::fs(12.5)).font(Font::MONOSPACE),
                Space::new().width(Fill),
                state.size(theme::fs(11.0)),
                container(action).width(theme::fs(52.0)).align_right(theme::fs(52.0)),
            ]
            .spacing(6)
            .align_y(Alignment::Center);
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
            let guide = container(container(Space::new()).width(1).height(Fill).style(|_| container::Style {
                background: Some(iced::Background::Color(theme::colors().separator)),
                ..Default::default()
            }))
            .padding(Padding { left: 16.0, ..Padding::ZERO })
            .height(Fill);
            list = list.push(stack![units, guide].width(Fill));
        }
    }
    theme::scroll(container(list).padding(Padding { top: 4.0, ..Padding::ZERO })).height(Fill).into()
}

fn editor(app: &App) -> Element<'_, Message> {
    let c = theme::colors();
    let hdl = &app.hdl;
    let Some(tab) = hdl.active_tab() else {
        return container(muted("Open a source from the list, or add one with “Open files…”.", 13.0))
            .center(Fill)
            .into();
    };
    let action = |label: &str, m: Msg| {
        button(text(label.to_string()).size(theme::fs(12.0)))
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
                    button(text(label).size(theme::fs(12.5)))
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
                    text("The file changed on disk.").size(theme::fs(12.0)).width(Fill),
                    small("Reload", Msg::Reload).style(theme::push),
                    small("Keep my version", Msg::KeepMine).style(theme::push),
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
        .size(theme::fs(13.0))
        .height(Fill)
        .padding(Padding { top: 8.0, right: 8.0, bottom: 8.0, left: 12.0 })
        .style(theme::code_area)
        .key_binding(|kp: KeyPress| {
            if kp.modifiers.command() && matches!(kp.key.as_ref(), Key::Character("s")) {
                return Some(Binding::Custom(msg(Msg::Save)));
            }
            Binding::from_key_press(kp)
        })
        .highlight_with::<highlight::Hdl>(tab.lang, highlight::format);
    col.push(edit).height(Fill).into()
}

fn log(app: &App) -> Element<'_, Message> {
    let errors = app.hdl.log.iter().filter(|d| d.level == Level::Error).count();
    let warnings = app.hdl.log.iter().filter(|d| d.level == Level::Warning).count();
    let summary = match (errors, warnings) {
        (0, 0) => String::new(),
        (e, w) => format!("{e} errors, {w} warnings"),
    };
    let header = row![
        text("Messages").size(theme::fs(12.0)).font(theme::FONT_MEDIUM),
        muted(summary, 11.0),
        Space::new().width(Fill),
        button(text("Clear").size(theme::fs(12.0)))
            .on_press(msg(Msg::ClearLog))
            .padding([2, 4])
            .style(theme::bare(theme::colors().muted, theme::colors().text)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let mut lines = column![];
    for (i, d) in app.hdl.log.iter().enumerate() {
        let color = match d.level {
            Level::Error => theme::colors().negative,
            Level::Warning => theme::colors().neutral,
            Level::Info => theme::colors().muted,
        };
        let t = text(d.text.clone()).size(theme::fs(11.5)).font(Font::MONOSPACE).color(color);
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
        container(header).padding([4, 8]),
        theme::hline(),
        theme::scroll(container(lines).padding([4, 0])).height(Fill),
    ]
    .into()
}
