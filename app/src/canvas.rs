//! The circuit editor: an `iced` canvas program.
//!
//! The canvas keeps only transient interaction state (drags, hover, double
//! clicks). Every change to the circuit or the view is published as an
//! [`EditorEvent`] and applied by the application, which owns the project,
//! the selection and the undo history.

use std::cell::Cell;
use std::collections::{BTreeSet, HashSet};
use std::time::{Duration, Instant};

use iced::mouse::{self, Cursor, Interaction, ScrollDelta};
use iced::widget::canvas::{self, Cache, Event, Frame, Geometry, LineDash, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, Vector, keyboard};

use lb_core::circuit::{CompGeom, CompId, Component, Geometry as CircuitGeometry, PinRef, Project, Wire};
use lb_core::component::{Kind, PinDir, Props};
use lb_core::netlist::NetId;
use lb_core::sim::Simulator;

use crate::routing::{self, Routes, dist_to_segment};
use crate::shapes::{Live, View, draw_component, draw_hdl_mark, draw_pins, is_live, lod_color};
use crate::theme;

/// Precomputed geometry of the circuit shown in the editor.
#[derive(Clone, Debug, Default)]
pub struct CircuitCache {
    pub geom: CircuitGeometry,
    pub routes: Routes,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub comps: BTreeSet<CompId>,
    pub wires: Vec<Wire>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.comps.is_empty() && self.wires.is_empty()
    }

    pub fn clear(&mut self) {
        self.comps.clear();
        self.wires.clear();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tool {
    Select,
    Place { kind: Kind, props: Box<Props>, rot: u8, mirror: bool },
}

impl Tool {
    /// The component the tool would place at grid position `pos`.
    pub fn ghost(&self, pos: (i32, i32)) -> Option<Component> {
        match self {
            Tool::Select => None,
            Tool::Place { kind, props, rot, mirror } => {
                Some(Component { id: 0, kind: *kind, pos, rot: *rot, mirror: *mirror, props: (**props).clone() })
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum EditorEvent {
    Many(Vec<EditorEvent>),
    Resized(Size),
    Pan(Vector),
    Zoom {
        factor: f32,
        anchor: Point,
    },
    Select {
        id: CompId,
        additive: bool,
    },
    SelectWire {
        wire: Wire,
        additive: bool,
    },
    SelectRect {
        from: (f32, f32),
        to: (f32, f32),
        additive: bool,
    },
    ClearSelection,
    MoveSelection {
        dx: i32,
        dy: i32,
    },
    Place {
        pos: (i32, i32),
    },
    Connect {
        from: PinRef,
        to: PinRef,
    },
    ConnectToWire {
        from: PinRef,
        wire: Wire,
    },
    Poke {
        id: CompId,
    },
    Button {
        id: CompId,
        pressed: bool,
    },
    OpenInstance {
        id: CompId,
    },
    /// Add the net of `(component, layout pin index)` to the waveform.
    Trace {
        comp: CompId,
        pin: usize,
    },
    /// Replace the waypoints of a wire.
    SetWirePath {
        wire: Wire,
        path: Vec<(i32, i32)>,
    },
    CancelTool,
    SpacePan,
}

#[derive(Debug, Clone, Default)]
enum Drag {
    #[default]
    None,
    Pan {
        last: Point,
    },
    Move {
        start: (f32, f32),
        id: CompId,
        delta: (i32, i32),
        moved: bool,
        /// The component is a button currently held down.
        held: bool,
    },
    Wire {
        from: PinRef,
        at: (i32, i32),
    },
    Rubber {
        start: (f32, f32),
        additive: bool,
    },
    Signal {
        comp: CompId,
        pin: usize,
        start: Point,
    },
    Waypoint {
        wire: Wire,
        index: usize,
        at: (i32, i32),
    },
}

#[derive(Default)]
pub struct EditorState {
    drag: Drag,
    cursor: Option<Point>,
    last_click: Option<(Instant, Point)>,
    modifiers: keyboard::Modifiers,
    grid: Cache,
    /// View, size, dark scheme and grid shown when the grid was drawn.
    grid_key: Cell<Option<(View, Size, bool, bool)>>,
    statics: Cache,
    static_key: Cell<Option<(View, Size, u64)>>,
}

pub struct Editor<'a> {
    pub project: &'a Project,
    pub circuit: &'a lb_core::circuit::Circuit,
    pub cache: &'a CircuitCache,
    pub sim: Option<&'a Simulator>,
    pub inst: &'a [CompId],
    pub selection: &'a Selection,
    pub tool: &'a Tool,
    pub view: View,
    pub size: Size,
    pub space_down: bool,
    pub oscillating: &'a HashSet<NetId>,
    pub show_grid: bool,
    /// Changes whenever the circuit or its geometry changes.
    pub version: u64,
}

const PIN_RADIUS: f32 = 0.45;
/// Below this zoom (pixels per grid unit) components are drawn as blocks.
const LOD_SCALE: f32 = 5.0;
const WIRE_RADIUS: f32 = 0.35;
/// Pixels per scrolled line, to treat line and pixel scrolling alike.
const LINE_PX: f32 = 40.0;

fn snap((x, y): (f32, f32)) -> (i32, i32) {
    (x.round() as i32, y.round() as i32)
}

fn shifted(g: &CompGeom, d: (i32, i32)) -> CompGeom {
    let mut g = g.clone();
    g.pos = (g.pos.0 + d.0, g.pos.1 + d.1);
    for p in &mut g.pins {
        *p = (p.0 + d.0, p.1 + d.1);
    }
    g
}

impl Editor<'_> {
    fn pin_at(&self, w: (f32, f32)) -> Option<(PinRef, (i32, i32), usize)> {
        let mut best: Option<(f32, PinRef, (i32, i32), usize)> = None;
        for c in &self.circuit.components {
            let Some(g) = self.cache.geom.items.get(&c.id) else { continue };
            for (i, &(x, y)) in g.pins.iter().enumerate() {
                let d = ((x as f32 - w.0).powi(2) + (y as f32 - w.1).powi(2)).sqrt();
                if d < PIN_RADIUS && best.as_ref().is_none_or(|b| d < b.0) {
                    best = Some((d, PinRef(c.id, g.layout.pins[i].name.clone()), (x, y), i));
                }
            }
        }
        best.map(|(_, p, pos, i)| (p, pos, i))
    }

    fn comp_at(&self, w: (f32, f32)) -> Option<&Component> {
        // Topmost first; annotations last so that they never hide parts.
        let hit = |c: &&Component| {
            let Some(g) = self.cache.geom.items.get(&c.id) else { return false };
            let (x0, y0, x1, y1) = g.bounds();
            let (x0, y0, x1, y1) = (x0 as f32, y0 as f32, x1 as f32, y1 as f32);
            let inside = w.0 >= x0 - 0.1 && w.0 <= x1 + 0.1 && w.1 >= y0 - 0.1 && w.1 <= y1 + 0.1;
            if c.kind == Kind::Rect {
                let near_border = (w.0 - x0).abs() < 0.4
                    || (w.0 - x1).abs() < 0.4
                    || (w.1 - y0).abs() < 0.4
                    || (w.1 - y1).abs() < 0.4;
                inside && near_border
            } else {
                inside
            }
        };
        let comps = &self.circuit.components;
        comps
            .iter()
            .rev()
            .filter(|c| c.kind != Kind::Rect)
            .find(hit)
            .or_else(|| comps.iter().rev().filter(|c| c.kind == Kind::Rect).find(hit))
    }

    fn wire_at(&self, w: (f32, f32)) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for (i, r) in self.cache.routes.wires.iter().enumerate() {
            for s in r.points.windows(2) {
                let d = dist_to_segment(w, s[0], s[1]);
                if d < WIRE_RADIUS && best.is_none_or(|b| d < b.0) {
                    best = Some((d, i));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// A waypoint handle of a selected wire under the cursor.
    fn waypoint_at(&self, w: (f32, f32)) -> Option<(Wire, usize)> {
        for wire in &self.selection.wires {
            for (k, &(x, y)) in wire.path.iter().enumerate() {
                if (x as f32 - w.0).abs() < PIN_RADIUS && (y as f32 - w.1).abs() < PIN_RADIUS {
                    return Some((wire.clone(), k));
                }
            }
        }
        None
    }

    /// `path` for `wire` without waypoints on straight runs or at the tip of
    /// a spur (see [`routing::tidy_path`]). A route crossing itself gets its
    /// loops cut out instead (see [`routing::pin_corners`]).
    fn tidy(&self, wire: &Wire, path: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
        match (self.cache.geom.pin_pos(&wire.from), self.cache.geom.pin_pos(&wire.to)) {
            (Some(a), Some(b)) => {
                let tidy = routing::tidy_path(a, &path, b);
                let corners = routing::pin_corners(a, &tidy, b);
                let same = routing::route_through(a, &corners, b) == routing::route_through(a, &tidy, b);
                if same { tidy } else { corners }
            }
            _ => path,
        }
    }

    /// The waypoints of `wire` after dragging waypoint `index` to `at`: its
    /// segments move along (see [`routing::move_waypoint`]) and each corner
    /// of the new route gets a waypoint.
    fn dragged_path(&self, wire: &Wire, index: usize, at: (i32, i32)) -> Vec<(i32, i32)> {
        let pin = |p| routing::pin_info(&self.cache.geom, p, &|_| (0, 0)).map(|(pos, dir, _)| (pos, dir));
        match (pin(&wire.from), pin(&wire.to)) {
            (Some(a), Some(b)) => routing::with_stubs(a, &routing::move_waypoint(a, &wire.path, index, at, b), b),
            _ => {
                let mut path = wire.path.clone();
                path[index] = at;
                path
            }
        }
    }

    /// The hovered component and the selected ones.
    fn focused(&self, state: &EditorState) -> impl Iterator<Item = &Component> {
        let hovered = state
            .cursor
            .filter(|_| *self.tool == Tool::Select && matches!(state.drag, Drag::None))
            .and_then(|p| self.comp_at(self.view.to_world(p)));
        hovered.into_iter().chain(self.selection.comps.iter().filter_map(|&id| self.circuit.component(id)))
    }

    /// Screen rectangle of a component's bounding box.
    fn screen_box(&self, g: &CompGeom) -> (Point, Size) {
        let (x0, y0, x1, y1) = g.bounds();
        let a = self.view.to_screen((x0 as f32, y0 as f32));
        let b = self.view.to_screen((x1 as f32, y1 as f32));
        (a, Size::new((b.x - a.x).max(1.0), (b.y - a.y).max(1.0)))
    }

    fn ghost_pos(&self, w: (f32, f32)) -> Option<Component> {
        let probe = self.tool.ghost((0, 0))?;
        let g = CompGeom::new(self.project, &probe);
        let (gw, gh) = g.size();
        let pos = snap((w.0 - gw as f32 / 2.0, w.1 - gh as f32 / 2.0));
        self.tool.ghost(pos)
    }

    fn net_of(&self, comp: CompId, pin: usize) -> Option<NetId> {
        self.sim?.netlist().pin_net(self.inst, comp, pin)
    }

    fn live(&self, c: &Component, g: &CompGeom) -> Live {
        let Some(sim) = self.sim else { return Live::default() };
        let nl = sim.netlist();
        let pins = (0..g.pins.len()).map(|i| nl.pin_net(self.inst, c.id, i).map(|n| sim.net_value(n))).collect();
        let q = match c.kind {
            Kind::Input | Kind::Button | Kind::Clock | Kind::Register | Kind::Counter | Kind::ShiftRegister => {
                nl.comp_index(self.inst, c.id).map(|i| sim.state(i).q)
            }
            _ => None,
        };
        let mem = match c.kind {
            Kind::Tty | Kind::Screen | Kind::Keyboard => {
                nl.comp_index(self.inst, c.id).map(|i| sim.state(i).mem.clone()).unwrap_or_default()
            }
            _ => Vec::new(),
        };
        Live { pins, q, mem }
    }

    fn wire_color(&self, from: Option<(CompId, usize)>) -> (Color, f32, bool) {
        let base = (self.view.scale * 0.12).clamp(1.2, 3.0);
        let Some(sim) = self.sim else { return (theme::colors().wire_idle, base, false) };
        let Some(net) = from.and_then(|(c, p)| self.net_of(c, p)) else {
            return (theme::colors().wire_idle, base, false);
        };
        let info = &sim.netlist().nets[net as usize];
        let color = theme::wire_color(sim.net_value(net), info);
        let width = if info.width > 1 { base * 1.8 } else { base };
        (color, width, self.oscillating.contains(&net))
    }

    fn visible(&self) -> (f32, f32, f32, f32) {
        let a = self.view.to_world(Point::ORIGIN);
        let b = self.view.to_world(Point::new(self.size.width, self.size.height));
        (a.0 - 2.0, a.1 - 2.0, b.0 + 2.0, b.1 + 2.0)
    }

    fn draw_grid(&self, state: &EditorState, renderer: &Renderer, bounds: Rectangle) -> Geometry {
        let key = (self.view, bounds.size(), theme::is_dark(), self.show_grid);
        if state.grid_key.get() != Some(key) {
            state.grid.clear();
            state.grid_key.set(Some(key));
        }
        state.grid.draw(renderer, bounds.size(), |frame| {
            frame.fill_rectangle(Point::ORIGIN, frame.size(), theme::colors().canvas_bg);
            if !self.show_grid || self.view.scale < 4.0 {
                return;
            }
            let step = if self.view.scale >= 8.0 { 1 } else { 5 };
            let (x0, y0, x1, y1) = self.visible();
            let r = (self.view.scale * 0.07).clamp(0.6, 1.4);
            let mut b = canvas::path::Builder::new();
            let mut x = (x0.floor() as i32).div_euclid(step) * step;
            while x as f32 <= x1 {
                let mut y = (y0.floor() as i32).div_euclid(step) * step;
                while y as f32 <= y1 {
                    let p = self.view.to_screen((x as f32, y as f32));
                    b.rectangle(Point::new(p.x - r, p.y - r), Size::new(2.0 * r, 2.0 * r));
                    y += step;
                }
                x += step;
            }
            frame.fill(&b.build(), theme::colors().grid);
        })
    }

    fn draw_wire(&self, frame: &mut Frame, pts: &[(i32, i32)], color: Color, width: f32) {
        if pts.len() < 2 {
            return;
        }
        let mut b = canvas::path::Builder::new();
        b.move_to(self.view.to_screen((pts[0].0 as f32, pts[0].1 as f32)));
        for p in &pts[1..] {
            b.line_to(self.view.to_screen((p.0 as f32, p.1 as f32)));
        }
        let path = b.build();
        frame.stroke(
            &path,
            Stroke::default()
                .with_color(color)
                .with_width(width)
                .with_line_join(canvas::LineJoin::Round)
                .with_line_cap(canvas::LineCap::Round),
        );
    }

    fn moving(&self, state: &EditorState) -> Option<(i32, i32)> {
        match state.drag {
            Drag::Move { delta, moved: true, .. } if delta != (0, 0) => Some(delta),
            _ => None,
        }
    }
}

fn local(cursor: Cursor, bounds: Rectangle) -> Option<Point> {
    cursor.position().map(|p| Point::new(p.x - bounds.x, p.y - bounds.y))
}

impl canvas::Program<EditorEvent> for Editor<'_> {
    type State = EditorState;

    fn update(
        &self,
        state: &mut EditorState,
        event: &Event,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> Option<canvas::Action<EditorEvent>> {
        let mut out = Vec::new();
        if bounds.size() != self.size {
            out.push(EditorEvent::Resized(bounds.size()));
        }
        let inside = cursor.is_over(bounds);
        let pos = local(cursor, bounds);
        let world = pos.map(|p| self.view.to_world(p));
        let mut redraw = false;
        let mut capture = false;

        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => state.modifiers = *m,
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                state.cursor = pos;
                match &mut state.drag {
                    Drag::Pan { last } => {
                        if let Some(p) = pos {
                            out.push(EditorEvent::Pan(p - *last));
                            *last = p;
                        }
                    }
                    Drag::Move { start, delta, moved, held, id } => {
                        if let Some(w) = world {
                            let d = snap((w.0 - start.0, w.1 - start.1));
                            if d != (0, 0) {
                                *moved = true;
                                if *held {
                                    *held = false;
                                    out.push(EditorEvent::Button { id: *id, pressed: false });
                                }
                            }
                            *delta = d;
                        }
                        redraw = true;
                    }
                    Drag::Wire { at, .. } | Drag::Waypoint { at, .. } => {
                        if let Some(w) = world {
                            *at = snap(w);
                        }
                        redraw = true;
                    }
                    _ => redraw = inside || !matches!(state.drag, Drag::None),
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(button)) if inside => {
                let (Some(p), Some(w)) = (pos, world) else { return None };
                capture = true;
                let now = Instant::now();
                let double =
                    state.last_click.is_some_and(|(t, q)| now - t < Duration::from_millis(400) && q.distance(p) < 5.0);
                state.last_click = Some((now, p));
                let additive = state.modifiers.shift() || state.modifiers.control();
                match button {
                    mouse::Button::Middle => state.drag = Drag::Pan { last: p },
                    mouse::Button::Right => {
                        if matches!(self.tool, Tool::Place { .. }) {
                            out.push(EditorEvent::CancelTool);
                        } else {
                            state.drag = Drag::Pan { last: p };
                        }
                    }
                    mouse::Button::Left if self.space_down => {
                        out.push(EditorEvent::SpacePan);
                        state.drag = Drag::Pan { last: p };
                    }
                    mouse::Button::Left => match self.tool {
                        Tool::Place { .. } => {
                            if let Some(g) = self.ghost_pos(w) {
                                out.push(EditorEvent::Place { pos: g.pos });
                            }
                        }
                        Tool::Select => {
                            let handle = self.waypoint_at(w);
                            if let Some((wire, index)) = handle {
                                if double {
                                    let mut path = wire.path.clone();
                                    path.remove(index);
                                    let path = self.tidy(&wire, path);
                                    out.push(EditorEvent::SetWirePath { wire, path });
                                } else {
                                    let at = wire.path[index];
                                    state.drag = Drag::Waypoint { wire, index, at };
                                }
                            } else if state.modifiers.alt()
                                && let Some(i) = self.wire_at(w)
                            {
                                let wire = self.circuit.wires[i].clone();
                                let path =
                                    routing::insert_waypoint(&self.cache.routes.wires[i].points, &wire.path, snap(w));
                                out.push(EditorEvent::SelectWire { wire: wire.clone(), additive: false });
                                out.push(EditorEvent::SetWirePath { wire, path });
                            } else if double {
                                if let Some(c) = self.comp_at(w) {
                                    match c.kind {
                                        Kind::Subcircuit => out.push(EditorEvent::OpenInstance { id: c.id }),
                                        Kind::Probe | Kind::Output | Kind::Led => {
                                            out.push(EditorEvent::Trace { comp: c.id, pin: 0 })
                                        }
                                        _ => {}
                                    }
                                } else if let Some(i) = self.wire_at(w)
                                    && let Some((c, pin)) = self.cache.routes.wires[i].from
                                {
                                    out.push(EditorEvent::Trace { comp: c, pin });
                                }
                                state.drag = Drag::None;
                            } else if let Some((pin, at, _)) = self.pin_at(w) {
                                state.drag = Drag::Wire { from: pin, at };
                            } else if let Some(c) = self.comp_at(w) {
                                if additive {
                                    out.push(EditorEvent::Select { id: c.id, additive: true });
                                } else {
                                    if !self.selection.comps.contains(&c.id) {
                                        out.push(EditorEvent::Select { id: c.id, additive: false });
                                    }
                                    // A button is pressed while the mouse is down,
                                    // until the drag turns into a move.
                                    let held = c.kind == Kind::Button;
                                    if held {
                                        out.push(EditorEvent::Button { id: c.id, pressed: true });
                                    }
                                    state.drag = Drag::Move { start: w, id: c.id, delta: (0, 0), moved: false, held };
                                }
                            } else if let Some(i) = self.wire_at(w) {
                                let wire = self.circuit.wires[i].clone();
                                out.push(EditorEvent::SelectWire { wire, additive });
                                if let Some((comp, pin)) = self.cache.routes.wires[i].from {
                                    state.drag = Drag::Signal { comp, pin, start: p };
                                }
                            } else {
                                state.drag = Drag::Rubber { start: w, additive };
                            }
                        }
                    },
                    _ => {}
                }
                redraw = true;
            }
            Event::Mouse(mouse::Event::ButtonReleased(_)) => {
                let drag = std::mem::take(&mut state.drag);
                match drag {
                    Drag::None => {}
                    Drag::Pan { .. } => {}
                    Drag::Move { id, delta, moved, held, .. } => {
                        if held {
                            out.push(EditorEvent::Button { id, pressed: false });
                        }
                        if moved && delta != (0, 0) {
                            out.push(EditorEvent::MoveSelection { dx: delta.0, dy: delta.1 });
                        } else {
                            let kind = self.circuit.component(id).map(|c| c.kind);
                            if kind == Some(Kind::Input) {
                                out.push(EditorEvent::Poke { id });
                            }
                            if self.selection.comps.len() > 1 {
                                out.push(EditorEvent::Select { id, additive: false });
                            }
                        }
                    }
                    Drag::Wire { from, .. } => {
                        if let Some(w) = world {
                            if let Some((to, _, _)) = self.pin_at(w) {
                                if to != from {
                                    out.push(EditorEvent::Connect { from, to });
                                }
                            } else if let Some(i) = self.wire_at(w) {
                                let wire = self.circuit.wires[i].clone();
                                if wire.from != from && wire.to != from {
                                    out.push(EditorEvent::ConnectToWire { from, wire });
                                }
                            }
                        }
                    }
                    Drag::Rubber { start, additive } => {
                        if let Some(w) = world {
                            let tiny = (w.0 - start.0).abs() < 0.2 && (w.1 - start.1).abs() < 0.2;
                            if tiny {
                                if !additive {
                                    out.push(EditorEvent::ClearSelection);
                                }
                            } else {
                                out.push(EditorEvent::SelectRect { from: start, to: w, additive });
                            }
                        }
                    }
                    Drag::Signal { comp, pin, start } => {
                        // Dropped below the canvas, on the waveform panel.
                        if let Some(p) = pos
                            && p.y > bounds.height
                            && p.distance(start) > 10.0
                        {
                            out.push(EditorEvent::Trace { comp, pin });
                        }
                    }
                    Drag::Waypoint { wire, index, at } => {
                        if wire.path.get(index) != Some(&at) {
                            let path = self.dragged_path(&wire, index, at);
                            out.push(EditorEvent::SetWirePath { wire, path });
                        }
                    }
                }
                redraw = true;
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if inside => {
                // A notched mouse wheel scrolls whole lines and zooms; touchpads
                // scroll by pixels (or by fractions of a line on X11) and pan,
                // with Ctrl to zoom. Shift+wheel pans horizontally.
                let (d, wheel) = match *delta {
                    ScrollDelta::Lines { x, y } => {
                        (Vector::new(x, y) * LINE_PX, x == 0.0 && y != 0.0 && y.fract() == 0.0)
                    }
                    ScrollDelta::Pixels { x, y } => (Vector::new(x, y), false),
                };
                if state.modifiers.control() || (wheel && !state.modifiers.shift()) {
                    if let Some(p) = pos {
                        out.push(EditorEvent::Zoom { factor: 1.15f32.powf(d.y / LINE_PX), anchor: p });
                    }
                } else if wheel {
                    out.push(EditorEvent::Pan(Vector::new(d.y, 0.0)));
                } else {
                    out.push(EditorEvent::Pan(d));
                }
                capture = true;
            }
            Event::Mouse(mouse::Event::CursorLeft) => {
                state.cursor = None;
                redraw = true;
            }
            _ => {}
        }

        let action = match out.len() {
            0 if redraw => Some(canvas::Action::request_redraw()),
            0 => None,
            1 => Some(canvas::Action::publish(out.pop().unwrap())),
            _ => Some(canvas::Action::publish(EditorEvent::Many(out))),
        };
        match action {
            Some(a) if capture => Some(a.and_capture()),
            None if capture => Some(canvas::Action::capture()),
            a => a,
        }
    }

    fn draw(
        &self,
        state: &EditorState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: Cursor,
    ) -> Vec<Geometry> {
        let grid = self.draw_grid(state, renderer, bounds);
        let (vx0, vy0, vx1, vy1) = self.visible();
        let in_view = |g: &CompGeom| {
            let (x0, y0, x1, y1) = g.bounds();
            (x1 as f32) >= vx0 && (x0 as f32) <= vx1 && (y1 as f32) >= vy0 && (y0 as f32) <= vy1
        };
        let moving = self.moving(state);
        let moved: HashSet<CompId> =
            if moving.is_some() { self.selection.comps.iter().copied().collect() } else { HashSet::new() };
        let geom_of = |c: &Component| {
            let g = self.cache.geom.items.get(&c.id)?;
            Some(if moved.contains(&c.id) { shifted(g, moving.unwrap()) } else { g.clone() })
        };

        // Static layer: components whose look does not depend on the
        // simulation, cached until the circuit, the view or the size change.
        let key = (self.view, bounds.size(), self.version);
        if moving.is_some() || state.static_key.get() != Some(key) {
            state.statics.clear();
            state.static_key.set(if moving.is_some() { None } else { Some(key) });
        }
        let lod = self.view.scale < LOD_SCALE;
        let statics = state.statics.draw(renderer, bounds.size(), |frame| {
            if lod {
                // Far away: plain blocks, one path for all.
                let mut b = canvas::path::Builder::new();
                for c in &self.circuit.components {
                    if c.kind.is_annotation() || is_live(c.kind) || moved.contains(&c.id) {
                        continue;
                    }
                    if let Some(g) = self.cache.geom.items.get(&c.id).filter(|g| in_view(g)) {
                        let (tl, size) = self.screen_box(g);
                        b.rectangle(tl, size);
                    }
                }
                frame.fill(&b.build(), theme::colors().lod_body);
                return;
            }
            let rects = self.circuit.components.iter().filter(|c| c.kind == Kind::Rect);
            let others = self.circuit.components.iter().filter(|c| c.kind != Kind::Rect);
            for c in rects.chain(others) {
                if moved.contains(&c.id) {
                    continue;
                }
                let Some(g) = self.cache.geom.items.get(&c.id) else { continue };
                if !in_view(g) {
                    continue;
                }
                if !is_live(c.kind) {
                    draw_component(frame, self.view, c, g, &Live::default(), 1.0);
                }
                if c.kind == Kind::Subcircuit
                    && let Some(h) = self.project.circuit(&c.props.circuit).and_then(|x| x.hdl.as_ref())
                {
                    draw_hdl_mark(frame, self.view, g, h.stale);
                }
                let connected = |i: usize| self.cache.routes.connected.contains(&(c.id, i));
                draw_pins(frame, self.view, g, &Live::default(), &connected);
            }
        });

        let mut frame = Frame::new(renderer, bounds.size());
        // Moved boxes under everything else.
        for c in self.circuit.components.iter().filter(|c| c.kind == Kind::Rect && moved.contains(&c.id)) {
            if let Some(g) = geom_of(c) {
                draw_component(&mut frame, self.view, c, &g, &Live::default(), 1.0);
            }
        }

        // Wires attached to components being dragged are re-routed live.
        let touches_moved = |w: &Wire| moved.contains(&w.from.0) || moved.contains(&w.to.0);
        let moving_routes = moving.map(|d| {
            let touching: Vec<Wire> = self.circuit.wires.iter().filter(|w| touches_moved(w)).cloned().collect();
            let routes = routing::compute(&self.cache.geom, &touching, &moved, d);
            (touching, routes)
        });

        // Selected wires underlay, following the live routes while dragging.
        let underlay = (self.view.scale * 0.45).clamp(4.0, 10.0);
        let highlight = Color { a: 0.35, ..theme::colors().accent };
        for (i, r) in self.cache.routes.wires.iter().enumerate() {
            let w = &self.circuit.wires[i];
            if self.selection.wires.contains(w) && !(moving.is_some() && touches_moved(w)) {
                self.draw_wire(&mut frame, &r.points, highlight, underlay);
            }
        }
        if let Some((touching, routes)) = &moving_routes {
            for (w, r) in touching.iter().zip(&routes.wires) {
                if self.selection.wires.contains(w) {
                    self.draw_wire(&mut frame, &r.points, highlight, underlay);
                }
            }
        }

        // Wires, batched into one path per colour and width.
        let mut batches: Vec<(Color, f32, canvas::path::Builder)> = Vec::new();
        let mut add_polyline = |pts: &[(i32, i32)], color: Color, width: f32| {
            if pts.len() < 2 {
                return;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
            for p in pts {
                (x0, y0, x1, y1) = (x0.min(p.0), y0.min(p.1), x1.max(p.0), y1.max(p.1));
            }
            if (x1 as f32) < vx0 || (x0 as f32) > vx1 || (y1 as f32) < vy0 || (y0 as f32) > vy1 {
                return;
            }
            let i = match batches.iter().position(|(c, w, _)| *c == color && *w == width) {
                Some(i) => i,
                None => {
                    batches.push((color, width, canvas::path::Builder::new()));
                    batches.len() - 1
                }
            };
            let b = &mut batches[i].2;
            b.move_to(self.view.to_screen((pts[0].0 as f32, pts[0].1 as f32)));
            for p in &pts[1..] {
                b.line_to(self.view.to_screen((p.0 as f32, p.1 as f32)));
            }
        };
        for (i, r) in self.cache.routes.wires.iter().enumerate() {
            let w = &self.circuit.wires[i];
            if moving.is_some() && (moved.contains(&w.from.0) || moved.contains(&w.to.0)) {
                continue;
            }
            let (color, width, osc) = self.wire_color(r.from);
            if osc {
                add_polyline(&r.points, Color { a: 0.45, ..theme::colors().oscillation }, width * 3.5);
            }
            add_polyline(&r.points, color, width);
        }
        if let Some((_, routes)) = &moving_routes {
            for r in &routes.wires {
                let (color, width, _) = self.wire_color(r.from);
                add_polyline(&r.points, color, width);
            }
        }
        // Wide (highlight) strokes first.
        batches.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (color, width, b) in batches {
            // Round joins cost tessellation work that is invisible from afar.
            let (join, cap) = if lod {
                (canvas::LineJoin::Miter, canvas::LineCap::Butt)
            } else {
                (canvas::LineJoin::Round, canvas::LineCap::Round)
            };
            frame.stroke(
                &b.build(),
                Stroke::default()
                    .with_color(color)
                    .with_width(if lod { 1.0 } else { width })
                    .with_line_join(join)
                    .with_line_cap(cap),
            );
        }
        // Junction dots.
        let dot_r = (self.view.scale * 0.2).clamp(2.0, 5.0);
        if moving.is_none() && !lod {
            let mut dots: Vec<(Color, canvas::path::Builder)> = Vec::new();
            for &((x, y), wire) in &self.cache.routes.dots {
                if (x as f32) < vx0 || (x as f32) > vx1 || (y as f32) < vy0 || (y as f32) > vy1 {
                    continue;
                }
                let p = self.view.to_screen((x as f32, y as f32));
                let color = self.wire_color(self.cache.routes.wires[wire].from).0;
                let i = match dots.iter().position(|(c, _)| *c == color) {
                    Some(i) => i,
                    None => {
                        dots.push((color, canvas::path::Builder::new()));
                        dots.len() - 1
                    }
                };
                dots[i].1.circle(p, dot_r);
            }
            for (color, b) in dots {
                frame.fill(&b.build(), color);
            }
        }

        // Live components (and the ones being moved).
        let mut lod_batches: Vec<(Color, canvas::path::Builder)> = Vec::new();
        for c in self.circuit.components.iter().filter(|c| c.kind != Kind::Rect) {
            if !is_live(c.kind) && !moved.contains(&c.id) {
                continue;
            }
            let Some(g) = geom_of(c) else { continue };
            if !in_view(&g) {
                continue;
            }
            let live = self.live(c, &g);
            if lod {
                if c.kind.is_annotation() {
                    continue;
                }
                let color = lod_color(c, &live);
                let i = match lod_batches.iter().position(|(k, _)| *k == color) {
                    Some(i) => i,
                    None => {
                        lod_batches.push((color, canvas::path::Builder::new()));
                        lod_batches.len() - 1
                    }
                };
                let (tl, size) = self.screen_box(&g);
                lod_batches[i].1.rectangle(tl, size);
                continue;
            }
            draw_component(&mut frame, self.view, c, &g, &live, 1.0);
            if moved.contains(&c.id) {
                let connected = |i: usize| self.cache.routes.connected.contains(&(c.id, i));
                draw_pins(&mut frame, self.view, &g, &live, &connected);
            }
        }

        for (color, b) in lod_batches {
            frame.fill(&b.build(), color);
        }

        // Tunnel groups: links under every component, outlines on top.
        let (links, group) = tunnel_links(self.circuit, self.focused(state));
        let mut under = Frame::new(renderer, bounds.size());
        if !links.is_empty() {
            let center = |id: CompId| {
                let (x0, y0, x1, y1) = geom_of(self.circuit.component(id)?)?.bounds();
                Some(self.view.to_screen(((x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0)))
            };
            let mut b = canvas::path::Builder::new();
            for (a, z) in links.into_iter().filter_map(|(a, z)| Some((center(a)?, center(z)?))) {
                b.move_to(a);
                b.line_to(z);
            }
            let dash = [6.0, 4.0];
            under.stroke(
                &b.build(),
                Stroke {
                    line_dash: LineDash { segments: &dash, offset: 0 },
                    ..Stroke::default().with_color(theme::colors().accent).with_width(1.5)
                },
            );
        }
        for id in group.difference(&self.selection.comps) {
            let Some(g) = self.circuit.component(*id).and_then(geom_of) else { continue };
            let (x0, y0, x1, y1) = g.bounds();
            let a = self.view.to_screen((x0 as f32 - 0.25, y0 as f32 - 0.25));
            let b = self.view.to_screen((x1 as f32 + 0.25, y1 as f32 + 0.25));
            let path = Path::rounded_rectangle(a, Size::new(b.x - a.x, b.y - a.y), (0.4 * self.view.scale).into());
            frame.stroke(
                &path,
                Stroke::default().with_color(Color { a: 0.6, ..theme::colors().accent }).with_width(1.0),
            );
        }

        // Selection outlines.
        for id in &self.selection.comps {
            let Some(g) = self.cache.geom.items.get(id) else { continue };
            let g = if moved.contains(id) { shifted(g, moving.unwrap()) } else { g.clone() };
            let (x0, y0, x1, y1) = g.bounds();
            let a = self.view.to_screen((x0 as f32 - 0.35, y0 as f32 - 0.35));
            let b = self.view.to_screen((x1 as f32 + 0.35, y1 as f32 + 0.35));
            let path = Path::rounded_rectangle(a, Size::new(b.x - a.x, b.y - a.y), (0.4 * self.view.scale).into());
            frame.fill(&path, Color { a: 0.06, ..theme::colors().accent });
            frame.stroke(&path, Stroke::default().with_color(theme::colors().accent).with_width(1.5));
        }

        // Waypoint handles of selected wires (and the wire being reshaped).
        let dragged = match &state.drag {
            Drag::Waypoint { wire, index, at } => Some((wire, *index, *at)),
            _ => None,
        };
        for wire in &self.selection.wires {
            let mut path = wire.path.clone();
            if let Some((w, i, at)) = dragged
                && w == wire
            {
                // The route and handles as they will be on release.
                path = self.dragged_path(wire, i, at);
                if let (Some(a), Some(b)) = (self.cache.geom.pin_pos(&wire.from), self.cache.geom.pin_pos(&wire.to)) {
                    let pts = routing::route_through(a, &path, b);
                    self.draw_wire(&mut frame, &pts, theme::colors().accent, 2.0);
                }
            }
            let r = (self.view.scale * 0.22).clamp(3.0, 6.0);
            for &(x, y) in &path {
                let p = self.view.to_screen((x as f32, y as f32));
                let sq = Path::rectangle(Point::new(p.x - r, p.y - r), Size::new(2.0 * r, 2.0 * r));
                frame.fill(&sq, theme::colors().body);
                frame.stroke(&sq, Stroke::default().with_color(theme::colors().accent).with_width(1.5));
            }
        }

        // Overlays.
        let cursor_world = state.cursor.map(|p| self.view.to_world(p));
        match &state.drag {
            Drag::Wire { from, at } => {
                if let Some(a) = self.cache.geom.pin_pos(from) {
                    let pts = routing::route_through(a, &[], *at);
                    let mut b = canvas::path::Builder::new();
                    b.move_to(self.view.to_screen((pts[0].0 as f32, pts[0].1 as f32)));
                    for p in &pts[1..] {
                        b.line_to(self.view.to_screen((p.0 as f32, p.1 as f32)));
                    }
                    let dash = [6.0, 4.0];
                    frame.stroke(
                        &b.build(),
                        Stroke {
                            line_dash: LineDash { segments: &dash, offset: 0 },
                            ..Stroke::default().with_color(theme::colors().accent).with_width(2.0)
                        },
                    );
                }
            }
            Drag::Rubber { start, .. } => {
                if let Some(w) = cursor_world {
                    let a = self.view.to_screen(*start);
                    let b = self.view.to_screen(w);
                    let tl = Point::new(a.x.min(b.x), a.y.min(b.y));
                    let size = Size::new((a.x - b.x).abs(), (a.y - b.y).abs());
                    frame.fill_rectangle(tl, size, Color { a: 0.08, ..theme::colors().accent });
                    frame.stroke(
                        &Path::rectangle(tl, size),
                        Stroke::default().with_color(theme::colors().accent).with_width(1.0),
                    );
                }
            }
            Drag::Signal { start, .. } => {
                if let Some(p) = state.cursor
                    && p.distance(*start) > 10.0
                {
                    let label = canvas::Text {
                        content: "↓ Drop on the waveform panel".into(),
                        position: Point::new(p.x + 12.0, p.y + 4.0),
                        color: theme::colors().accent,
                        size: theme::fs(13.0).into(),
                        font: theme::FONT,
                        ..Default::default()
                    };
                    frame.fill_text(label);
                }
            }
            _ => {}
        }
        if let Some(w) = cursor_world {
            match self.tool {
                Tool::Place { .. } => {
                    if let Some(ghost) = self.ghost_pos(w) {
                        let g = CompGeom::new(self.project, &ghost);
                        draw_component(&mut frame, self.view, &ghost, &g, &Live::default(), 0.5);
                    }
                }
                Tool::Select => {
                    if matches!(state.drag, Drag::None | Drag::Wire { .. })
                        && let Some((_, (x, y), _)) = self.pin_at(w)
                    {
                        let p = self.view.to_screen((x as f32, y as f32));
                        let r = (self.view.scale * 0.4).max(5.0);
                        frame.stroke(
                            &Path::circle(p, r),
                            Stroke::default().with_color(theme::colors().accent).with_width(2.0),
                        );
                    }
                }
            }
        }
        vec![grid, under.into_geometry(), statics, frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &EditorState, bounds: Rectangle, cursor: Cursor) -> Interaction {
        match state.drag {
            Drag::Pan { .. } => return Interaction::Grabbing,
            Drag::Move { moved: true, .. } => return Interaction::Grabbing,
            Drag::Wire { .. } => return Interaction::Crosshair,
            _ => {}
        }
        let Some(p) = cursor.position_in(bounds) else { return Interaction::default() };
        if self.space_down {
            return Interaction::Grab;
        }
        let w = self.view.to_world(p);
        match self.tool {
            Tool::Place { .. } => Interaction::Crosshair,
            Tool::Select => {
                if self.pin_at(w).is_some() {
                    Interaction::Crosshair
                } else if let Some(c) = self.comp_at(w) {
                    if matches!(c.kind, Kind::Input | Kind::Button) { Interaction::Pointer } else { Interaction::Grab }
                } else if self.wire_at(w).is_some() {
                    Interaction::Pointer
                } else {
                    Interaction::default()
                }
            }
        }
    }
}

/// Tunnels shown as one group with each of the tunnels `from` (the hovered
/// and the selected components; others are ignored): the pairs to link with
/// dashed lines, and every tunnel of the groups.
pub fn tunnel_links<'a>(
    circuit: &lb_core::circuit::Circuit,
    from: impl Iterator<Item = &'a Component>,
) -> (BTreeSet<(CompId, CompId)>, BTreeSet<CompId>) {
    let (mut links, mut group) = (BTreeSet::new(), BTreeSet::new());
    let tunnels = || circuit.components.iter().filter(|c| c.kind == Kind::Tunnel);
    for from in from.filter(|c| c.kind == Kind::Tunnel) {
        let label = from.props.label.trim();
        if label.is_empty() {
            continue;
        }
        for to in tunnels().filter(|c| c.id != from.id && c.props.label.trim() == label) {
            links.insert((from.id.min(to.id), from.id.max(to.id)));
            group.extend([from.id, to.id]);
        }
    }
    (links, group)
}

/// Normalizes a new wire so that an output pin is the `from` end.
pub fn orient_wire(project: &Project, circuit: &lb_core::circuit::Circuit, a: PinRef, b: PinRef) -> Wire {
    let dir = |p: &PinRef| circuit.component(p.0).and_then(|c| project.layout(c).pin(&p.1).map(|(_, d)| d.dir));
    if dir(&a) != Some(PinDir::Out) && dir(&b) == Some(PinDir::Out) { Wire::new(b, a) } else { Wire::new(a, b) }
}
