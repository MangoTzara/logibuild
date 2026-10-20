//! Properties panel on the right: component properties, multi-selection
//! tools, circuit settings, diagnostics and simulation settings.

use std::fmt;

use iced::widget::{Row, Space, button, checkbox, column, container, pick_list, row, text, text_editor, text_input};
use iced::{Alignment, Color, Element, Fill, Font};

use lb_core::circuit::{CompId, Component};
use lb_core::component::layout::Terminal;
use lb_core::component::{ColorMode, Edge, Kind, MAX_DIGITS, MAX_KEY_BUFFER, PropKey, ShiftMode};
use lb_core::history::Command;
use lb_core::value::{Radix, Signal};

use crate::app::{Align, App, Message};
use crate::icons::{Emblem, Icon};
use crate::routing;
use crate::theme;

#[derive(Debug, Clone)]
pub enum Edit {
    Text(PropKey, String),
    Width(u8),
    Inputs(u8),
    Select(u8),
    OutWidth(u8),
    AddrWidth(u8),
    Length(u8),
    Edge(EdgeOpt),
    Signed(bool),
    Shift(ShiftOpt),
    Radix(Radix),
    RectW(String),
    RectH(String),
    Digits(DigitsOpt),
    Colors(ColorOpt),
    LiveValue(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeOpt(pub Edge);

impl fmt::Display for EdgeOpt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            Edge::Rising => "Rising",
            Edge::Falling => "Falling",
        })
    }
}

/// Number of digits; 0 is "as many as needed", shown with that number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DigitsOpt {
    pub digits: u8,
    auto: u8,
}

impl fmt::Display for DigitsOpt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.digits == 0 { write!(f, "Auto ({})", self.auto) } else { write!(f, "{}", self.digits) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorOpt(pub ColorMode);

impl fmt::Display for ColorOpt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShiftOpt(pub ShiftMode);

impl fmt::Display for ShiftOpt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.name())
    }
}

fn draft_key(id: u32, key: PropKey) -> String {
    format!("{id}.{key:?}")
}

/// Text shown in a field: the draft being typed, or the property value.
fn prop_text(c: &Component, key: PropKey) -> String {
    let p = &c.props;
    match key {
        PropKey::Label => p.label.clone(),
        PropKey::Value => {
            let v = Signal::new(p.width(), p.value);
            if p.width() == 1 { v.bit(0).to_char().to_string() } else { format!("0x{}", v.to_hex_string()) }
        }
        PropKey::Period => p.period().to_string(),
        PropKey::Duty => p.duty().to_string(),
        PropKey::Pieces => p.pieces().iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" "),
        PropKey::Data => p.data.clone(),
        PropKey::DataFile => p.data_file.clone(),
        PropKey::Delay => p.delay.to_string(),
        PropKey::Buffer => p.key_buffer().to_string(),
        _ => String::new(),
    }
}

/// Multi-line editor for the contents of the selected ROM or RAM.
#[derive(Default)]
pub struct HexEditor {
    id: Option<CompId>,
    /// Contents property the text was last loaded from or written to.
    synced: String,
    pub content: text_editor::Content,
}

impl HexEditor {
    /// Whether another memory is selected or its contents changed elsewhere
    /// (undo, import).
    pub fn stale(&self, c: Option<&Component>) -> bool {
        match c.filter(|c| matches!(c.kind, Kind::Rom | Kind::Ram)) {
            Some(c) => self.id != Some(c.id) || self.synced != c.props.data,
            None => self.id.is_some(),
        }
    }

    /// Reloads the text from the selected component.
    pub fn sync(&mut self, c: Option<&Component>) {
        match c.filter(|c| matches!(c.kind, Kind::Rom | Kind::Ram)) {
            Some(c) => {
                self.id = Some(c.id);
                self.synced = c.props.data.clone();
                self.content = text_editor::Content::with_text(&c.props.data);
            }
            None => self.id = None,
        }
    }
}

/// Performs an action in the contents editor, storing the text if valid.
pub fn edit_hex(app: &mut App, action: text_editor::Action) -> Option<Command> {
    let edit = action.is_edit();
    app.hex_editor.content.perform(action);
    if !edit {
        return None;
    }
    let text = app.hex_editor.content.text();
    let cmd = apply(app, Edit::Text(PropKey::Data, text.clone()));
    if cmd.is_some() {
        // What `apply` stores, so that `sync` keeps the text as typed.
        app.hex_editor.synced = text.trim().to_string();
    }
    cmd
}

/// Applies an edit to the selected component; returns the command to run.
pub fn apply(app: &mut App, edit: Edit) -> Option<Command> {
    let comp = app.selected_component()?.clone();
    let mut c = comp.clone();
    let p = &mut c.props;
    let mut merge = None;
    match edit {
        Edit::Text(key, s) => {
            app.drafts.insert(draft_key(comp.id, key), s.clone());
            merge = Some(draft_key(comp.id, key));
            let t = s.trim();
            match key {
                PropKey::Label => p.label = s.clone(),
                PropKey::Value => p.value = Signal::parse(p.width(), t)?.to_u64()?,
                PropKey::Period => p.period = t.parse::<u32>().ok()?.max(2),
                PropKey::Duty => p.duty = t.parse::<u8>().ok()?.clamp(1, 99),
                PropKey::Pieces => {
                    let v: Option<Vec<u8>> =
                        t.split([' ', ',']).filter(|x| !x.is_empty()).map(|x| x.parse().ok()).collect();
                    p.pieces = v?.into_iter().filter(|&x| x > 0).collect();
                }
                PropKey::Data => {
                    lb_core::io::parse_hex(t).ok()?;
                    p.data = t.to_string();
                }
                PropKey::DataFile => p.data_file = t.to_string(),
                PropKey::Delay => p.delay = t.parse().ok()?,
                PropKey::Buffer => {
                    p.buffer = t.parse::<u32>().ok()?.clamp(1, MAX_KEY_BUFFER as u32) as u16;
                }
                _ => return None,
            }
        }
        Edit::Width(w) => p.width = w,
        Edit::Inputs(n) => p.inputs = n,
        Edit::Select(n) => p.select = n,
        Edit::OutWidth(n) => p.out_width = n,
        Edit::AddrWidth(n) => p.addr_width = n,
        Edit::Length(n) => p.length = n,
        Edit::Edge(e) => p.edge = e.0,
        Edit::Signed(b) => p.signed = b,
        Edit::Shift(s) => p.shift = s.0,
        Edit::Radix(r) => p.radix = r,
        Edit::RectW(s) | Edit::RectH(s) if s.trim().parse::<u16>().is_err() => return None,
        Edit::RectW(ref s) | Edit::RectH(ref s) => {
            let (_, lo, hi) = comp.kind.size_limits();
            let (w, h) = p.size_of(comp.kind);
            let n = s.trim().parse::<i32>().ok()?;
            p.size = if matches!(edit, Edit::RectW(_)) {
                (n.clamp(lo.0, hi.0) as u16, h as u16)
            } else {
                (w as u16, n.clamp(lo.1, hi.1) as u16)
            };
        }
        Edit::Digits(d) => p.digits = d.digits,
        Edit::Colors(c) => p.colors = c.0,
        Edit::LiveValue(s) => {
            app.drafts.insert(format!("{}.live", comp.id), s.clone());
            let v = Signal::parse(comp.props.width(), &s)?;
            app.set_live_input(comp.id, v);
            return None;
        }
    }
    if c == comp {
        return None;
    }
    let mut cmd = Command::update(&app.project, app.circuit(), vec![c]);
    cmd.label = "Edit properties".into();
    cmd.merge = merge;
    Some(cmd)
}

/// A row of a form: the label right aligned against the field, as in KDE
/// forms.
fn field<'a>(label: &str, widget: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    row![
        text(format!("{label}:")).size(theme::fs(13.0)).width(theme::fs(LABEL_W)).align_x(Alignment::End),
        container(widget).width(Fill)
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// Width of the label column at the default text size.
const LABEL_W: f32 = 104.0;

/// Section title followed by a hairline.
fn section<'a>(title: &str) -> Element<'a, Message> {
    column![
        Space::new().height(8),
        row![text(title.to_string()).size(theme::fs(13.0)).font(theme::FONT_MEDIUM), theme::hline()]
            .spacing(8)
            .align_y(Alignment::Center),
    ]
    .into()
}

fn title<'a>(t: impl ToString) -> Element<'a, Message> {
    text(t.to_string()).size(theme::fs(16.0)).font(theme::FONT_MEDIUM).into()
}

/// Explanatory text.
fn note<'a>(t: impl ToString) -> Element<'a, Message> {
    text(t.to_string()).size(theme::fs(12.0)).color(theme::colors().muted).into()
}

fn push<'a>(label: &str, msg: Message) -> button::Button<'a, Message> {
    button(text(label.to_string()).size(theme::fs(13.0))).on_press(msg).padding([4, 10]).style(theme::push)
}

fn delete_button<'a>(label: &str) -> Element<'a, Message> {
    button(theme::icon_label(Icon::Delete, label, true))
        .on_press(Message::Delete)
        .padding([4, 10])
        .style(theme::push)
        .into()
}

fn input<'a>(app: &App, draft: String, value: String, on: impl Fn(String) -> Message + 'a) -> Element<'a, Message> {
    let shown = app.drafts.get(&draft).cloned().unwrap_or(value);
    text_input("", &shown).on_input(on).size(theme::fs(13.0)).padding([4, 8]).style(theme::input).into()
}

fn numbers(range: std::ops::RangeInclusive<u8>) -> Vec<u8> {
    range.collect()
}

fn component_view<'a>(app: &'a App, c: &'a Component) -> Element<'a, Message> {
    let mut col = column![
        title(c.kind.name()),
        note(format!(
            "ID {} · Position ({}, {}) · Rotation {}°{}",
            c.id,
            c.pos.0,
            c.pos.1,
            c.rot as u32 * 90,
            if c.mirror { " · Flipped" } else { "" }
        )),
    ]
    .spacing(8);
    let p = &c.props;
    for &key in c.kind.prop_keys() {
        let id = c.id;
        let txt = |key: PropKey| {
            input(app, draft_key(id, key), prop_text(c, key), move |s| Message::Prop(Edit::Text(key, s)))
        };
        let el: Element<Message> = match key {
            PropKey::Label => field(key.name(), txt(key)),
            PropKey::Value => field(if c.kind == Kind::Input { "Initial value" } else { key.name() }, txt(key)),
            PropKey::Period
            | PropKey::Duty
            | PropKey::Pieces
            | PropKey::DataFile
            | PropKey::Delay
            | PropKey::Buffer => field(key.name(), txt(key)),
            PropKey::Data => {
                let line = theme::fs(13.0) * 1.3;
                column![
                    text(format!("{}:", key.name())).size(theme::fs(13.0)),
                    text_editor(&app.hex_editor.content)
                        .on_action(Message::HexEdit)
                        .font(Font::MONOSPACE)
                        .size(theme::fs(13.0))
                        .padding([4, 8])
                        // Six lines high whatever the contents; it scrolls.
                        .height(line * 6.0 + 8.0)
                        .style(theme::text_area),
                ]
                .spacing(4)
                .into()
            }
            PropKey::Width => field(
                key.name(),
                pick_list(numbers(1..=64), Some(p.width()), |w| Message::Prop(Edit::Width(w)))
                    .text_size(theme::fs(13.0))
                    .padding([4, 10])
                    .style(theme::dropdown)
                    .menu_style(theme::dropdown_menu)
                    .handle(theme::dropdown_handle()),
            ),
            PropKey::Inputs => field(
                key.name(),
                pick_list(numbers(2..=8), Some(p.inputs()), |w| Message::Prop(Edit::Inputs(w)))
                    .text_size(theme::fs(13.0))
                    .padding([4, 10])
                    .style(theme::dropdown)
                    .menu_style(theme::dropdown_menu)
                    .handle(theme::dropdown_handle()),
            ),
            PropKey::Select => field(
                key.name(),
                pick_list(numbers(1..=5), Some(p.select()), |w| Message::Prop(Edit::Select(w)))
                    .text_size(theme::fs(13.0))
                    .padding([4, 10])
                    .style(theme::dropdown)
                    .menu_style(theme::dropdown_menu)
                    .handle(theme::dropdown_handle()),
            ),
            PropKey::OutWidth => field(
                key.name(),
                pick_list(numbers(1..=64), Some(p.out_width()), |w| Message::Prop(Edit::OutWidth(w)))
                    .text_size(theme::fs(13.0))
                    .padding([4, 10])
                    .style(theme::dropdown)
                    .menu_style(theme::dropdown_menu)
                    .handle(theme::dropdown_handle()),
            ),
            PropKey::AddrWidth => field(
                key.name(),
                pick_list(numbers(1..=16), Some(p.addr_width()), |w| Message::Prop(Edit::AddrWidth(w)))
                    .text_size(theme::fs(13.0))
                    .padding([4, 10])
                    .style(theme::dropdown)
                    .menu_style(theme::dropdown_menu)
                    .handle(theme::dropdown_handle()),
            ),
            PropKey::Length => field(
                key.name(),
                pick_list(numbers(1..=64), Some(p.length()), |w| Message::Prop(Edit::Length(w)))
                    .text_size(theme::fs(13.0))
                    .padding([4, 10])
                    .style(theme::dropdown)
                    .menu_style(theme::dropdown_menu)
                    .handle(theme::dropdown_handle()),
            ),
            PropKey::Edge => field(
                key.name(),
                pick_list([EdgeOpt(Edge::Rising), EdgeOpt(Edge::Falling)], Some(EdgeOpt(p.edge)), |e| {
                    Message::Prop(Edit::Edge(e))
                })
                .text_size(theme::fs(13.0))
                .padding([4, 10])
                .style(theme::dropdown)
                .menu_style(theme::dropdown_menu)
                .handle(theme::dropdown_handle()),
            ),
            PropKey::Signed => field(
                key.name(),
                checkbox(p.signed).on_toggle(|b| Message::Prop(Edit::Signed(b))).size(16).style(theme::check),
            ),
            PropKey::Shift => field(
                key.name(),
                pick_list(ShiftMode::ALL.map(ShiftOpt), Some(ShiftOpt(p.shift)), |s| Message::Prop(Edit::Shift(s)))
                    .text_size(theme::fs(13.0))
                    .padding([4, 10])
                    .style(theme::dropdown)
                    .menu_style(theme::dropdown_menu)
                    .handle(theme::dropdown_handle()),
            ),
            PropKey::Radix => field(
                key.name(),
                pick_list(&Radix::ALL[..], Some(p.radix), |r| Message::Prop(Edit::Radix(r)))
                    .text_size(theme::fs(13.0))
                    .padding([4, 10])
                    .style(theme::dropdown)
                    .menu_style(theme::dropdown_menu)
                    .handle(theme::dropdown_handle()),
            ),
            PropKey::Digits => {
                let choices: Vec<DigitsOpt> = if c.kind == Kind::SevenSeg {
                    (1..=8).map(|digits| DigitsOpt { digits, auto: 1 }).collect()
                } else {
                    let auto = lb_core::component::Props { digits: 0, ..p.clone() }.num_digits();
                    (0..=MAX_DIGITS).map(|digits| DigitsOpt { digits, auto }).collect()
                };
                let current = choices.iter().copied().find(|o| o.digits == p.digits).or(choices.first().copied());
                field(
                    key.name(),
                    pick_list(choices, current, |d| Message::Prop(Edit::Digits(d)))
                        .text_size(theme::fs(13.0))
                        .padding([4, 10])
                        .style(theme::dropdown)
                        .menu_style(theme::dropdown_menu)
                        .handle(theme::dropdown_handle()),
                )
            }
            PropKey::Colors => field(
                key.name(),
                pick_list(ColorMode::ALL.map(ColorOpt), Some(ColorOpt(p.colors)), |c| Message::Prop(Edit::Colors(c)))
                    .text_size(theme::fs(13.0))
                    .padding([4, 10])
                    .style(theme::dropdown)
                    .menu_style(theme::dropdown_menu)
                    .handle(theme::dropdown_handle()),
            ),
            PropKey::Size => {
                let (w, h) = p.size_of(c.kind);
                let label = match c.kind {
                    Kind::LedMatrix | Kind::Tty => "Columns × rows",
                    Kind::Screen => "Pixels",
                    _ => key.name(),
                };
                field(
                    label,
                    row![
                        input(app, format!("{id}.w"), w.to_string(), |s| Message::Prop(Edit::RectW(s))),
                        text("×").size(theme::fs(13.0)),
                        input(app, format!("{id}.h"), h.to_string(), |s| Message::Prop(Edit::RectH(s))),
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center),
                )
            }
        };
        col = col.push(el);
    }

    // Live value of inputs.
    if c.kind == Kind::Input
        && let Some(v) = app.live_input(c.id)
    {
        let shown = if p.width() == 1 { v.bit(0).to_char().to_string() } else { format!("0x{}", v.to_hex_string()) };
        col = col.push(field(
            "Current value",
            input(app, format!("{}.live", c.id), shown, |s| Message::Prop(Edit::LiveValue(s))),
        ));
        if p.width() == 1 {
            col = col.push(note("Tip: click the input on the canvas to toggle it."));
        }
    }

    if c.kind == Kind::Keyboard {
        let waiting = app.keyboard_keys(c.id);
        col = col.push(
            row![
                button(text("Clear buffer").size(theme::fs(13.0)))
                    .on_press_maybe(waiting.is_some_and(|n| n > 0).then_some(Message::ClearKeyboard(c.id)))
                    .padding([4, 10])
                    .style(theme::push),
                text(format!("{}/{}", waiting.unwrap_or(0), p.key_buffer()))
                    .size(theme::fs(12.0))
                    .color(theme::colors().muted),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }
    // Tunnel: how many it joins, and the labels already used in the circuit.
    if c.kind == Kind::Tunnel {
        let label = p.label.trim();
        let mut used = std::collections::BTreeMap::new();
        for t in app.circuit().components.iter().filter(|t| t.kind == Kind::Tunnel && t.id != c.id) {
            *used.entry(t.props.label.trim()).or_insert(0) += 1;
        }
        col = col.push(note(match used.get(label) {
            _ if label.is_empty() => "Tunnels with the same label are connected.".to_string(),
            Some(1) => format!("Connected to 1 other tunnel “{label}”."),
            Some(n) => format!("Connected to {n} other tunnels “{label}”."),
            None => format!("No other tunnel is labelled “{label}”."),
        }));
        let others: Vec<&str> = used.into_keys().filter(|l| !l.is_empty() && *l != label).collect();
        if !others.is_empty() {
            let chips = others.into_iter().map(|l| {
                button(text(l.to_string()).size(theme::fs(12.0)))
                    .on_press(Message::Prop(Edit::Text(PropKey::Label, l.to_string())))
                    .padding([2, 8])
                    .style(theme::push)
                    .into()
            });
            col = col.push(field("Existing", Row::with_children(chips).spacing(4).wrap().vertical_spacing(4)));
        }
    }

    let small = push;
    if matches!(c.kind, Kind::Rom | Kind::Ram) {
        col = col.push(
            row![small("Import .hex…", Message::ImportHex), small("Export .hex…", Message::ExportHex)].spacing(6),
        );
        col = col.push(note("Contents can stay inline in the .lbc file or live in a separate .hex file."));
    }
    let mut actions = row![small("Rotate", Message::Rotate), small("Flip", Message::Mirror)].spacing(6);
    if !c.kind.is_annotation() {
        actions = actions.push(small("To waveform", Message::TraceComponent(c.id)));
    }
    // One line, wrapping the buttons that do not fit.
    col = col.push(Space::new().height(4)).push(actions.push(delete_button("Delete")).wrap().vertical_spacing(6));
    if c.kind == Kind::Subcircuit {
        col = col.push(small("Open circuit", Message::OpenCircuit(p.circuit.clone())));
        col = col.push(note("Double-click the instance to look inside it with live simulation values."));
    }
    col.into()
}

fn multi_view(app: &App) -> Element<'_, Message> {
    let n = app.selection.comps.len();
    let w = app.selection.wires.len();
    let small = push;
    let mut col = column![title("Selection"), note(format!("{n} components, {w} wires"))].spacing(8);
    if n > 1 {
        col = col.push(section("Align"));
        col = col.push(
            row![
                small("Left", Message::Align(Align::Left)),
                small("Center", Message::Align(Align::CenterX)),
                small("Right", Message::Align(Align::Right))
            ]
            .spacing(6),
        );
        col = col.push(
            row![
                small("Top", Message::Align(Align::Top)),
                small("Center", Message::Align(Align::CenterY)),
                small("Bottom", Message::Align(Align::Bottom))
            ]
            .spacing(6),
        );
    }
    if n == 0 && w == 1 {
        let wire = app.selection.wires[0].clone();
        col = col.push(
            text(format!("Wire from {}.{} to {}.{}", wire.from.0, wire.from.1, wire.to.0, wire.to.1))
                .size(theme::fs(13.0)),
        );
        col = col.push(small("Add to waveform", Message::TraceWire(wire.clone())));
        col = col.push(section("Routing"));
        let auto = wire.path.is_empty();
        col = col.push(note(if auto {
            "Routed automatically. Alt+click the wire to add a bend point, or freeze the current route."
        } else {
            "Manual waypoints: drag the square handles, double-click one to remove it, Alt+click the wire to add one."
        }));
        let frozen: Vec<(i32, i32)> = app
            .wire_route(&wire)
            .filter(|pts| pts.len() > 2)
            .map(|pts| routing::tidy_path(pts[0], &pts[1..pts.len() - 1], pts[pts.len() - 1]))
            .unwrap_or_default();
        let mut buttons = row![].spacing(6);
        if auto && !frozen.is_empty() {
            buttons = buttons.push(small("Freeze route", Message::SetWirePath(wire.clone(), frozen)));
        }
        if !auto {
            buttons = buttons.push(small("Automatic route", Message::SetWirePath(wire.clone(), Vec::new())));
        }
        col = col.push(buttons);
    }
    col = col.push(section("Actions"));
    col = col.push(
        row![
            small("Rotate", Message::Rotate),
            small("Flip", Message::Mirror),
            small("Duplicate", Message::Duplicate),
            delete_button("Delete")
        ]
        .spacing(6)
        .wrap()
        .vertical_spacing(6),
    );
    col.into()
}

fn circuit_view(app: &App) -> Element<'_, Message> {
    let c = app.circuit();
    let is_main = app.project.circuit_index(&c.name) == Some(0);
    let mut col = column![
        title("Circuit"),
        field("Name", input(app, "circuit.name".into(), c.name.clone(), Message::RenameCircuit)),
        note(format!(
            "{} components, {} wires{}",
            c.components.len(),
            c.wires.len(),
            if is_main { " · Main circuit" } else { "" }
        )),
    ]
    .spacing(8);
    let (ins, outs) = lb_core::component::layout::terminals(c);
    if !ins.is_empty() || !outs.is_empty() {
        col = col.push(section("Pins"));
        col = col.push(pin_table(app, &ins, &outs));
        col =
            col.push(note("Inputs and outputs become the terminals of this circuit when it is used as a subcircuit."));
    }
    if !is_main {
        col = col.push(
            row![
                push("Make main", Message::MakeMain),
                button(theme::icon_label(Icon::Delete, "Delete circuit", true))
                    .on_press(Message::DeleteCircuit)
                    .padding([4, 10])
                    .style(theme::push),
            ]
            .spacing(6),
        );
    }

    // Diagnostics of the compiled netlist.
    if let Some(sim) = app.sim() {
        let diags = &sim.netlist().diagnostics;
        if !diags.is_empty() {
            col = col.push(section("Warnings"));
            for d in diags.iter().take(12) {
                col = col.push(theme::message(Emblem::Warning, text(d.message.clone()).size(theme::fs(12.0))));
            }
            if diags.len() > 12 {
                col = col.push(note(format!("… and {} more", diags.len() - 12)));
            }
        }
    }

    // Simulation settings.
    col = col.push(section("Simulation"));
    col = col.push(field(
        "Zero-delay",
        checkbox(app.zero_delay)
            .on_toggle(Message::SetZeroDelay)
            .size(16)
            .label("Delta cycles only")
            .text_size(theme::fs(13.0))
            .style(theme::check),
    ));
    if !app.zero_delay {
        col = col.push(field(
            "Gate delay",
            input(app, "cfg.delay".into(), app.sim_cfg.gate_delay.to_string(), Message::SetGateDelay),
        ));
    }
    col = col.push(field(
        "Delta limit",
        input(app, "cfg.delta".into(), app.sim_cfg.delta_limit.to_string(), Message::SetDeltaLimit),
    ));
    col = col.push(field(
        "Trace samples",
        input(app, "cfg.trace".into(), app.trace_capacity.to_string(), Message::SetTraceCapacity),
    ));
    col = col.push(note("Each component’s delay can be changed in its properties."));
    col.into()
}

/// Table of the circuit's Input/Output pins: name, direction, width and live
/// value. Clicking a row selects the pin on the canvas.
fn pin_table<'a>(app: &'a App, ins: &[Terminal], outs: &[Terminal]) -> Element<'a, Message> {
    use iced::widget::container::Style;
    use iced::{Background, Border};

    let value_of = |id: u32| -> String {
        let Some(sim) = app.sim() else { return "–".into() };
        match sim.netlist().pin_net(&app.loc().inst, id, 0) {
            Some(net) => {
                let v = sim.net_value(net);
                if v.width() == 1 { v.bit(0).to_char().to_string() } else { format!("0x{}", v.to_hex_string()) }
            }
            None => "–".into(),
        }
    };
    let cell = |content: Element<'a, Message>, width: iced::Length| container(content).width(width).padding([4, 8]);
    let head = |label: &str| text(label.to_string()).size(theme::fs(12.0)).font(theme::FONT_MEDIUM).into();
    // Header of a Qt item view: window colour, separated by a hairline.
    let header = row![
        cell(head("Pin"), Fill),
        cell(head("Dir"), 52.into()),
        cell(head("Width"), 58.into()),
        cell(head("Value"), 64.into()),
    ];
    let mut table = column![
        container(header)
            .width(Fill)
            .style(|_| Style { background: Some(Background::Color(theme::colors().window)), ..Default::default() }),
        theme::hline(),
    ];

    let rows = ins.iter().map(|t| (t, true)).chain(outs.iter().map(|t| (t, false)));
    for (k, ((name, width, id), is_in)) in rows.enumerate() {
        let (label, color) = if is_in { ("IN", theme::colors().link) } else { ("OUT", theme::colors().positive) };
        let chip = container(text(label).size(theme::fs(10.0)).font(theme::FONT_MEDIUM).color(color))
            .padding([0, 5])
            .style(theme::chip(color));
        let bits = if *width == 1 { "1 bit".to_string() } else { format!("{width} bits") };
        let line = row![
            cell(text(name.clone()).size(theme::fs(13.0)).into(), Fill),
            cell(chip.into(), 52.into()),
            cell(text(bits).size(theme::fs(12.0)).color(theme::colors().muted).into(), 58.into()),
            cell(text(value_of(*id)).size(theme::fs(13.0)).into(), 64.into()),
        ]
        .align_y(Alignment::Center);
        // Alternating row colours.
        let stripe = if k % 2 == 1 { theme::colors().view_alt } else { theme::colors().view };
        let selected = app.selection.comps.contains(id);
        table = table.push(
            button(line)
                .on_press(Message::Editor(crate::canvas::EditorEvent::Select { id: *id, additive: false }))
                .padding(0)
                .width(Fill)
                .style(move |t, status| {
                    let mut s = theme::item(selected)(t, status);
                    s.border = Border::default();
                    s.background = Some(Background::Color(match s.background {
                        Some(Background::Color(tint)) => blend(stripe, tint),
                        _ => stripe,
                    }));
                    s
                }),
        );
    }
    container(table)
        .width(Fill)
        .padding(1)
        .clip(true)
        .style(|_| Style {
            background: Some(Background::Color(theme::colors().view)),
            border: Border { color: theme::colors().frame, width: 1.0, radius: theme::ITEM_RADIUS.into() },
            ..Default::default()
        })
        .into()
}

/// `top` composited over the opaque `base`.
fn blend(base: Color, top: Color) -> Color {
    let mix = |b: f32, t: f32| b * (1.0 - top.a) + t * top.a;
    Color::from_rgb(mix(base.r, top.r), mix(base.g, top.g), mix(base.b, top.b))
}

pub fn view(app: &App) -> Element<'_, Message> {
    let content: Element<Message> = if let Some(c) = app.selected_component() {
        component_view(app, c)
    } else if !app.selection.is_empty() {
        multi_view(app)
    } else {
        circuit_view(app)
    };
    theme::scroll(container(content).padding([12, 14]).width(Fill)).height(Fill).into()
}
