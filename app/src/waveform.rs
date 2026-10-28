//! Waveform (timing diagram) panel.
//!
//! Draws the signals recorded by the simulator's [`Trace`]: one row per
//! signal, names on the left, a time ruler on top. Time is
//! scrolled and zoomed horizontally; the panel can follow the simulation.

use iced::alignment::Vertical;
use iced::mouse::{self, Cursor, Interaction, ScrollDelta};
use iced::widget::canvas::{self, Event, Frame, Geometry, Path, Stroke, Text};
use iced::widget::text::Alignment as TextAlign;
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, keyboard};

use lb_core::trace::{Trace, TraceSignal};
use lb_core::value::{Radix, Signal};

use crate::theme;

/// Width of the column of names, by default and at least.
pub const NAME_W: f32 = 190.0;
const MIN_NAME_W: f32 = 110.0;
/// Distance from the column edge within which it can be dragged.
const GRIP: f32 = 4.0;
pub const RULER_H: f32 = 22.0;
pub const ROW_H: f32 = 28.0;
/// Zoom limits, in pixels per tick.
const MIN_ZOOM: f64 = 1e-7;
const MAX_ZOOM: f64 = 200.0;

/// View settings of the waveform panel (kept by the application).
#[derive(Clone, Debug, PartialEq)]
pub struct WaveView {
    pub visible: bool,
    /// Pixels per tick.
    pub zoom: f64,
    /// Time at the left edge of the wave area.
    pub start: f64,
    pub follow: bool,
    pub selected: Option<usize>,
    /// Width of the column of names, set by dragging its edge.
    pub names_w: f32,
    pub size: Size,
    /// Time span requested before the panel was laid out.
    pending_span: Option<f64>,
}

impl Default for WaveView {
    fn default() -> Self {
        WaveView {
            visible: true,
            zoom: 4.0,
            start: 0.0,
            follow: true,
            selected: None,
            names_w: NAME_W,
            size: Size::ZERO,
            pending_span: None,
        }
    }
}

impl WaveView {
    pub fn wave_width(&self) -> f64 {
        (self.size.width - self.names_w).max(1.0) as f64
    }

    pub fn visible_ticks(&self) -> f64 {
        self.wave_width() / self.zoom
    }

    pub fn time_at(&self, x: f32) -> f64 {
        self.start + (x - self.names_w) as f64 / self.zoom
    }

    pub fn x_of(&self, t: f64) -> f32 {
        self.names_w + ((t - self.start) * self.zoom) as f32
    }

    /// Keeps the current time in view when following.
    pub fn follow_to(&mut self, now: u64) {
        if self.follow {
            let span = self.visible_ticks();
            self.start = (now as f64 - span * 0.95).max(0.0);
        }
    }

    pub fn resize(&mut self, size: Size) {
        self.size = size;
        if let Some(span) = self.pending_span.take() {
            self.set_span(span);
        }
    }

    /// Zooms so that `ticks` fill the visible width, keeping the right edge
    /// in place.
    pub fn set_span(&mut self, ticks: f64) {
        if self.size.width <= self.names_w {
            self.pending_span = Some(ticks);
            return;
        }
        let end = self.start + self.visible_ticks();
        self.zoom = (self.wave_width() / ticks).clamp(MIN_ZOOM, MAX_ZOOM);
        self.start = (end - self.visible_ticks()).max(0.0);
    }

    /// Sets the width of the column of names, leaving room for the waves.
    pub fn set_names_width(&mut self, w: f32) {
        let max = (self.size.width * 0.7).max(MIN_NAME_W);
        self.names_w = w.clamp(MIN_NAME_W, max);
    }

    pub fn zoom_at(&mut self, factor: f64, anchor_t: f64) {
        let new = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.start = (anchor_t - (anchor_t - self.start) * self.zoom / new).max(0.0);
        self.zoom = new;
    }

    /// Fits `[t0, t1]` in the visible width.
    pub fn fit(&mut self, t0: u64, t1: u64) {
        let span = (t1.saturating_sub(t0)).max(10) as f64;
        self.zoom = (self.wave_width() / (span * 1.05)).clamp(MIN_ZOOM, MAX_ZOOM);
        self.start = t0 as f64;
    }
}

#[derive(Debug, Clone)]
pub enum WaveMsg {
    Resized(Size),
    /// The edge of the column of names was dragged to this x.
    NamesWidth(f32),
    Select(usize),
    CycleRadix(usize),
    Remove(usize),
    Scroll(f64),
    Zoom {
        factor: f64,
        anchor: f64,
    },
    ToggleFollow,
    Fit,
    ExportCsv,
    ExportVcd,
    Hide,
}

pub struct Waveform<'a> {
    pub trace: Option<&'a Trace>,
    pub now: u64,
    pub view: &'a WaveView,
}

#[derive(Default)]
pub struct WaveState {
    drag: Option<(Point, f64, bool)>,
    /// The edge of the column of names is being dragged.
    resizing: bool,
    modifiers: keyboard::Modifiers,
}

pub fn content_height(n: usize) -> f32 {
    RULER_H + ROW_H * n.max(1) as f32 + 6.0
}

fn nice_step(min_ticks: f64) -> f64 {
    let p = 10f64.powf(min_ticks.max(1e-9).log10().floor());
    for m in [1.0, 2.0, 5.0, 10.0] {
        if p * m >= min_ticks {
            return (p * m).max(1.0);
        }
    }
    p * 10.0
}

fn short_value(v: Signal, radix: Radix) -> String {
    if v.width() == 1 { v.bit(0).to_char().to_string() } else { v.format(radix) }
}

impl canvas::Program<WaveMsg> for Waveform<'_> {
    type State = WaveState;

    fn update(
        &self,
        state: &mut WaveState,
        event: &Event,
        bounds: Rectangle,
        cursor: Cursor,
    ) -> Option<canvas::Action<WaveMsg>> {
        if (bounds.width - self.view.size.width).abs() > 0.5 {
            return Some(canvas::Action::publish(WaveMsg::Resized(bounds.size())));
        }
        let pos = cursor.position_in(bounds);
        let n = self.trace.map_or(0, |t| t.signals().len());
        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => state.modifiers = *m,
            Event::Mouse(mouse::Event::ButtonPressed(b)) => {
                let p = pos?;
                if self.on_edge(p) {
                    state.resizing = true;
                    return Some(canvas::Action::capture());
                }
                let names_w = self.view.names_w;
                if p.y > RULER_H && p.x < names_w {
                    let row = ((p.y - RULER_H) / ROW_H) as usize;
                    if row < n {
                        // Radix chip at the right of the name column, remove "×" at the left.
                        if p.x < 18.0 {
                            return Some(canvas::Action::publish(WaveMsg::Remove(row)).and_capture());
                        }
                        if p.x > names_w - 46.0 {
                            return Some(canvas::Action::publish(WaveMsg::CycleRadix(row)).and_capture());
                        }
                        return Some(canvas::Action::publish(WaveMsg::Select(row)).and_capture());
                    }
                    return None;
                }
                if p.x >= names_w && matches!(b, mouse::Button::Left | mouse::Button::Middle) {
                    state.drag = Some((p, self.view.start, false));
                    return Some(canvas::Action::capture());
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if state.resizing {
                    return Some(canvas::Action::publish(WaveMsg::NamesWidth(position.x - bounds.x)).and_capture());
                }
                if let (Some((p0, start, moved)), Some(p)) = (&mut state.drag, cursor.position_in(bounds).or(pos)) {
                    let dx = p.x - p0.x;
                    if dx.abs() > 3.0 || *moved {
                        *moved = true;
                        let target = (*start - dx as f64 / self.view.zoom).max(0.0);
                        return Some(canvas::Action::publish(WaveMsg::Scroll(target)));
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(_)) => {
                state.drag = None;
                state.resizing = false;
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let p = pos?;
                let (dx, dy) = match delta {
                    ScrollDelta::Lines { x, y } => (*x * 40.0, *y * 40.0),
                    ScrollDelta::Pixels { x, y } => (*x, *y),
                };
                if state.modifiers.control() {
                    let anchor = self.view.time_at(p.x.max(self.view.names_w));
                    let msg = WaveMsg::Zoom { factor: 1.2f64.powf(dy as f64 / 40.0), anchor };
                    return Some(canvas::Action::publish(msg).and_capture());
                }
                if state.modifiers.shift() || dx != 0.0 {
                    let d = if dx != 0.0 { dx } else { dy };
                    let target = (self.view.start - d as f64 / self.view.zoom).max(0.0);
                    return Some(canvas::Action::publish(WaveMsg::Scroll(target)).and_capture());
                }
            }
            _ => {}
        }
        None
    }

    fn draw(
        &self,
        state: &WaveState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: Cursor,
    ) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let size = bounds.size();
        f.fill_rectangle(Point::ORIGIN, size, theme::colors().view);
        let v = self.view;
        let wave_x0 = v.names_w;

        // Ruler.
        f.fill_rectangle(Point::ORIGIN, Size::new(size.width, RULER_H), theme::colors().view_alt);
        let step = nice_step(80.0 / v.zoom);
        let t0 = (v.start / step).floor() * step;
        let mut t = t0;
        let grid_color = theme::colors().wave_grid;
        while v.x_of(t) < size.width {
            let x = v.x_of(t);
            if x >= wave_x0 {
                f.fill_rectangle(Point::new(x, RULER_H), Size::new(1.0, size.height - RULER_H), grid_color);
                f.fill_text(Text {
                    content: format!("{}", t as u64),
                    position: Point::new(x + 3.0, RULER_H / 2.0),
                    color: theme::colors().muted,
                    size: theme::fs(theme::TINY).into(),
                    font: theme::FONT,
                    align_y: Vertical::Center,
                    ..Default::default()
                });
            }
            t += step;
        }

        let Some(trace) = self.trace else {
            return vec![f.into_geometry()];
        };
        let t_end = v.start + v.visible_ticks();
        for (row, s) in trace.signals().iter().enumerate() {
            let y0 = RULER_H + row as f32 * ROW_H;
            if Some(row) == v.selected {
                f.fill_rectangle(Point::new(0.0, y0), Size::new(size.width, ROW_H), theme::colors().accent_soft);
            }
            f.fill_rectangle(Point::new(0.0, y0 + ROW_H - 1.0), Size::new(size.width, 1.0), grid_color);
            self.draw_name(&mut f, s, y0);
            self.draw_signal(&mut f, s, y0, t_end, size.width);
        }

        // Separator and now marker.
        let edge = if state.resizing { theme::colors().accent } else { theme::colors().separator };
        f.fill_rectangle(Point::new(v.names_w - 1.0, 0.0), Size::new(1.0, size.height), edge);
        let now_x = v.x_of(self.now as f64);
        if now_x >= wave_x0 && now_x <= size.width {
            f.fill_rectangle(
                Point::new(now_x, RULER_H),
                Size::new(1.0, size.height),
                Color { a: 0.4, ..theme::colors().muted },
            );
        }
        vec![f.into_geometry()]
    }

    fn mouse_interaction(&self, state: &WaveState, bounds: Rectangle, cursor: Cursor) -> Interaction {
        if state.drag.is_some_and(|d| d.2) {
            return Interaction::Grabbing;
        }
        if state.resizing || cursor.position_in(bounds).is_some_and(|p| self.on_edge(p)) {
            return Interaction::ResizingHorizontally;
        }
        match cursor.position_in(bounds) {
            Some(p) if p.x < self.view.names_w && p.y > RULER_H => Interaction::Pointer,
            Some(_) => Interaction::Grab,
            None => Interaction::default(),
        }
    }
}

impl Waveform<'_> {
    /// `p` is on the edge between the names and the waves.
    fn on_edge(&self, p: Point) -> bool {
        (p.x - self.view.names_w).abs() <= GRIP
    }

    fn draw_name(&self, f: &mut Frame, s: &TraceSignal, y0: f32) {
        let cy = y0 + ROW_H / 2.0;
        let value = s.value_at(self.now).map(|v| short_value(v, s.radix)).unwrap_or_default();
        f.fill_text(Text {
            content: "×".into(),
            position: Point::new(9.0, cy),
            color: theme::colors().muted,
            size: theme::fs(14.0).into(),
            font: theme::FONT,
            align_x: TextAlign::Center,
            align_y: Vertical::Center,
            ..Default::default()
        });
        // Fewer characters fit with a larger text, more in a wider column.
        let max = ((16.0 * 13.0 / theme::fs(theme::BODY)) * (self.view.names_w / NAME_W)) as usize;
        let mut name = s.name.clone();
        if name.chars().count() > max {
            name = format!(
                "…{}",
                name.chars().rev().take(max - 1).collect::<Vec<_>>().into_iter().rev().collect::<String>()
            );
        }
        f.fill_text(Text {
            content: name,
            position: Point::new(20.0, cy),
            color: theme::colors().text,
            size: theme::fs(theme::BODY).into(),
            font: theme::FONT,
            align_y: Vertical::Center,
            ..Default::default()
        });
        f.fill_text(Text {
            content: value,
            position: Point::new(self.view.names_w - 52.0, cy),
            color: theme::colors().link,
            size: theme::fs(theme::SMALL).into(),
            font: theme::FONT,
            align_x: TextAlign::Right,
            align_y: Vertical::Center,
            ..Default::default()
        });
        let chip =
            Path::rounded_rectangle(Point::new(self.view.names_w - 44.0, cy - 8.0), Size::new(38.0, 16.0), 4.0.into());
        f.fill(&chip, theme::colors().hover);
        f.fill_text(Text {
            content: if s.width == 1 { "Bit".into() } else { s.radix.to_string() },
            position: Point::new(self.view.names_w - 25.0, cy),
            color: theme::colors().muted,
            size: theme::fs(10.0).into(),
            font: theme::FONT,
            align_x: TextAlign::Center,
            align_y: Vertical::Center,
            ..Default::default()
        });
    }

    fn draw_signal(&self, f: &mut Frame, s: &TraceSignal, y0: f32, t_end: f64, right: f32) {
        let v = self.view;
        let (hi, lo) = (y0 + 6.0, y0 + ROW_H - 7.0);
        let mid = (hi + lo) / 2.0;
        let samples = &s.samples;
        if samples.is_empty() {
            return;
        }
        // Segments [t_i, t_{i+1}) visible in the window.
        let first = samples.partition_point(|(t, _)| (*t as f64) <= v.start).saturating_sub(1);
        let end_time = self.now as f64;
        let mut i = first;
        let clamp_x = |x: f32| x.clamp(v.names_w, right);
        while i < samples.len() {
            let (t, val) = samples[i];
            if t as f64 > t_end {
                break;
            }
            let t_next = samples.get(i + 1).map_or(end_time, |s| s.0 as f64);
            if t_next < t as f64 {
                i += 1;
                continue;
            }
            let x0 = clamp_x(v.x_of(t as f64));
            let x1 = clamp_x(v.x_of(t_next));
            if x1 - x0 < 0.5 && i + 1 < samples.len() {
                // Many changes within one pixel: draw a dense band.
                let mut j = i + 1;
                while j < samples.len() && clamp_x(v.x_of(samples[j].0 as f64)) - x0 < 1.0 {
                    j += 1;
                }
                f.fill_rectangle(Point::new(x0, hi), Size::new(1.0, lo - hi), theme::colors().wire_bus);
                // Resume from the last change inside this pixel (its segment
                // extends beyond it), always making progress.
                i = (j - 1).max(i + 1);
                continue;
            }
            let color_x = Color { a: 0.18, ..theme::colors().wire_x };
            if val.has_x() {
                f.fill_rectangle(Point::new(x0, hi), Size::new(x1 - x0, lo - hi), color_x);
                f.stroke(
                    &Path::rectangle(Point::new(x0, hi), Size::new(x1 - x0, lo - hi)),
                    Stroke::default().with_color(theme::colors().wire_x).with_width(1.0),
                );
            } else if val.is_all_z() {
                f.fill_rectangle(Point::new(x0, mid - 0.75), Size::new(x1 - x0, 1.5), theme::colors().wire_z);
            } else if s.width == 1 {
                let y = if val.is_one() { hi } else { lo };
                f.fill_rectangle(Point::new(x0, y - 0.75), Size::new(x1 - x0, 1.5), theme::colors().wire_low);
                if val.is_one() {
                    f.fill_rectangle(
                        Point::new(x0, hi),
                        Size::new(x1 - x0, lo - hi),
                        Color { a: 0.10, ..theme::colors().wire_high },
                    );
                }
                // Vertical edge from the previous value.
                if i > 0 && (v.x_of(t as f64)) >= v.names_w {
                    f.fill_rectangle(Point::new(x0 - 0.75, hi), Size::new(1.5, lo - hi), theme::colors().wire_low);
                }
            } else {
                // Bus: hexagon-like segment with the value.
                let k = 3.0f32.min((x1 - x0) / 2.0);
                let mut b = canvas::path::Builder::new();
                b.move_to(Point::new(x0, mid));
                b.line_to(Point::new(x0 + k, hi));
                b.line_to(Point::new(x1 - k, hi));
                b.line_to(Point::new(x1, mid));
                b.line_to(Point::new(x1 - k, lo));
                b.line_to(Point::new(x0 + k, lo));
                b.close();
                let path = b.build();
                let color = if val.has_z() { theme::colors().wire_z } else { theme::colors().wire_bus };
                f.fill(&path, Color { a: 0.07, ..color });
                f.stroke(&path, Stroke::default().with_color(color).with_width(1.2));
                let text = short_value(val, s.radix);
                let room = x1 - x0 - 2.0 * k;
                if room > 7.0 * text.len() as f32 {
                    f.fill_text(Text {
                        content: text,
                        position: Point::new((x0 + x1) / 2.0, mid),
                        color: theme::colors().text,
                        size: theme::fs(theme::TINY).into(),
                        font: theme::FONT,
                        align_x: TextAlign::Center,
                        align_y: Vertical::Center,
                        ..Default::default()
                    });
                }
            }
            i += 1;
        }
    }
}
