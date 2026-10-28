//! Drawing of components on the editor canvas.
//!
//! Shapes are described in the component's canonical frame (facing east,
//! grid units) and mapped through its transform to world and then screen
//! coordinates. Text always stays upright.

use iced::alignment::Vertical;
use iced::widget::canvas::{Frame, LineDash, Path, Stroke, Text, path::Builder};
use iced::widget::text::Alignment as TextAlign;
use iced::{Color, Pixels, Point, Size, Vector};

use lb_core::circuit::{CompGeom, Component};
use lb_core::component::layout::screen_pixel;
use lb_core::component::{Kind, PinDir, ShiftMode};
use lb_core::value::{Bit, Radix, Signal};

use crate::routing::pin_direction;
use crate::theme;

/// World (grid) to screen mapping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Pixels per grid unit.
    pub scale: f32,
    /// Screen position of the world origin.
    pub offset: Vector,
}

impl View {
    pub fn to_screen(self, (x, y): (f32, f32)) -> Point {
        Point::new(x * self.scale + self.offset.x, y * self.scale + self.offset.y)
    }

    pub fn to_world(self, p: Point) -> (f32, f32) {
        ((p.x - self.offset.x) / self.scale, (p.y - self.offset.y) / self.scale)
    }
}

/// Live simulation data for one component.
#[derive(Clone, Debug, Default)]
pub struct Live {
    /// Value of each pin's net, by layout pin index.
    pub pins: Vec<Option<Signal>>,
    /// Internal state value (input switch, register...).
    pub q: Option<Signal>,
    /// Contents of a text terminal or a pixel screen.
    pub mem: Vec<u64>,
}

impl Live {
    fn pin(&self, i: usize) -> Option<Signal> {
        self.pins.get(i).copied().flatten()
    }
}

pub struct Painter<'a, 'f> {
    pub frame: &'f mut Frame,
    pub view: View,
    pub geom: &'a CompGeom,
    pub alpha: f32,
}

const K: f32 = 0.552_284_8;

impl Painter<'_, '_> {
    fn c(&self, color: Color) -> Color {
        Color { a: color.a * self.alpha, ..color }
    }

    /// Canonical point to screen.
    fn m(&self, x: f32, y: f32) -> Point {
        let (tx, ty) = self.geom.transform.apply_f((x, y));
        self.view.to_screen((self.geom.pos.0 as f32 + tx, self.geom.pos.1 as f32 + ty))
    }

    fn line_width(&self) -> f32 {
        (self.view.scale * 0.11).clamp(1.0, 3.0)
    }

    fn stroke_path(&mut self, path: &Path, color: Color) {
        let w = self.line_width();
        let color = self.c(color);
        self.frame.stroke(path, Stroke::default().with_color(color).with_width(w));
    }

    fn fill_path(&mut self, path: &Path, color: Color) {
        let color = self.c(color);
        self.frame.fill(path, color);
    }

    fn body(&mut self, path: &Path) {
        self.fill_path(path, theme::colors().body);
        self.stroke_path(path, theme::colors().stroke);
    }

    fn poly(&self, pts: &[(f32, f32)], close: bool) -> Path {
        let mut b = Builder::new();
        for (i, &(x, y)) in pts.iter().enumerate() {
            if i == 0 {
                b.move_to(self.m(x, y));
            } else {
                b.line_to(self.m(x, y));
            }
        }
        if close {
            b.close();
        }
        b.build()
    }

    fn line(&mut self, a: (f32, f32), b: (f32, f32), color: Color) {
        let p = self.poly(&[a, b], false);
        self.stroke_path(&p, color);
    }

    /// Screen rectangle of a canonical box (axis aligned after rotation).
    fn screen_rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> (Point, Size) {
        let a = self.m(x0, y0);
        let b = self.m(x1, y1);
        let tl = Point::new(a.x.min(b.x), a.y.min(b.y));
        (tl, Size::new((a.x - b.x).abs(), (a.y - b.y).abs()))
    }

    fn rounded(&self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> Path {
        let (tl, size) = self.screen_rect(x0, y0, x1, y1);
        Path::rounded_rectangle(tl, size, (r * self.view.scale).into())
    }

    fn circle(&self, x: f32, y: f32, r: f32) -> Path {
        Path::circle(self.m(x, y), r * self.view.scale)
    }

    fn text(
        &mut self,
        content: impl Into<String>,
        at: Point,
        size: f32,
        color: Color,
        align: TextAlign,
        valign: Vertical,
    ) {
        let px = size * self.view.scale;
        if px < 5.0 {
            return;
        }
        let color = self.c(color);
        self.frame.fill_text(Text {
            content: content.into(),
            position: at,
            color,
            size: Pixels(px),
            font: theme::FONT,
            align_x: align,
            align_y: valign,
            ..Default::default()
        });
    }

    fn center_text(&mut self, content: impl Into<String>, x: f32, y: f32, size: f32, color: Color) {
        let at = self.m(x, y);
        self.text(content, at, size, color, TextAlign::Center, Vertical::Center);
    }
}

fn value_text(v: Option<Signal>, width: u8, radix: Radix) -> String {
    match v {
        Some(v) if width == 1 => v.bit(0).to_char().to_string(),
        Some(v) => v.format(radix),
        None => "-".into(),
    }
}

/// Whether the look of a component depends on simulation values (it is then
/// redrawn every frame instead of being cached).
pub fn is_live(kind: Kind) -> bool {
    use Kind::*;
    matches!(
        kind,
        Input
            | Output
            | Led
            | Button
            | Clock
            | Probe
            | Keyboard
            | NumDisplay
            | HexDisplay
            | SevenSeg
            | LedMatrix
            | Tty
            | Screen
            | Register
            | Counter
            | ShiftRegister
    )
}

/// Flat colour of a live component when drawn as a block (far zoom).
pub fn lod_color(comp: &Component, live: &Live) -> Color {
    let v = live.q.or(live.pin(0));
    match comp.kind {
        Kind::Led => {
            if v.is_some_and(|v| v.is_one()) {
                Color::from_rgb8(0xff, 0x3b, 0x30)
            } else {
                Color::from_rgb8(0x5a, 0x1f, 0x1c)
            }
        }
        Kind::Input | Kind::Output | Kind::Clock | Kind::Button
            if comp.props.width() == 1 || comp.kind == Kind::Clock =>
        {
            v.map(theme::bit_fill).unwrap_or(theme::colors().wire_idle)
        }
        Kind::NumDisplay | Kind::HexDisplay | Kind::SevenSeg | Kind::LedMatrix => DISPLAY_BG,
        Kind::Tty => TTY_BG,
        Kind::Screen => Color::BLACK,
        _ => theme::colors().lod_body,
    }
}

/// Draws a component. `alpha` < 1 draws a ghost (placement preview).
/// Marks an instance of a circuit synthesized from HDL: a small tag, and an
/// orange outline when it is out of date.
pub fn draw_hdl_mark(frame: &mut Frame, view: View, geom: &CompGeom, stale: bool) {
    let mut p = Painter { frame, view, geom, alpha: 1.0 };
    let (w, h) = (geom.layout.w as f32, geom.layout.h as f32);
    let color = if stale { theme::colors().neutral } else { theme::colors().muted };
    if stale {
        let path = p.rounded(-0.25, -0.25, w + 0.25, h + 0.25, 0.4);
        p.stroke_path(&path, color);
    }
    p.center_text("HDL", w / 2.0, h - 0.55, 0.5, color);
}

pub fn draw_component(frame: &mut Frame, view: View, comp: &Component, geom: &CompGeom, live: &Live, alpha: f32) {
    let mut p = Painter { frame, view, geom, alpha };
    let (w, h) = (geom.layout.w as f32, geom.layout.h as f32);
    let props = &comp.props;
    let width = props.width();
    use Kind::*;
    match comp.kind {
        Input => {
            let v = live.q.or(live.pin(0));
            let path = p.rounded(0.15, 0.15, w - 0.15, h - 0.15, 0.25);
            if width == 1 {
                let fill = v.map(theme::bit_fill).unwrap_or(theme::colors().wire_idle);
                p.fill_path(&path, fill);
                p.stroke_path(&path, theme::colors().stroke);
                p.center_text(value_text(v, 1, Radix::Bin), w / 2.0, h / 2.0, 1.1, Color::WHITE);
            } else {
                p.body(&path);
                p.center_text(value_text(v, width, props.radix), w / 2.0, h / 2.0, 1.0, theme::colors().text);
            }
        }
        Output | Probe => {
            let v = live.pin(0);
            let r = if comp.kind == Output { h / 2.0 - 0.15 } else { 0.2 };
            let path = p.rounded(0.15, 0.15, w - 0.15, h - 0.15, r);
            if width == 1 && comp.kind == Output {
                let fill = v.map(theme::bit_fill).unwrap_or(theme::colors().wire_idle);
                p.fill_path(&path, fill);
                p.stroke_path(&path, theme::colors().stroke);
                p.center_text(value_text(v, 1, Radix::Bin), w / 2.0, h / 2.0, 1.1, Color::WHITE);
            } else {
                p.fill_path(&path, if comp.kind == Probe { theme::colors().accent_soft } else { theme::colors().body });
                p.stroke_path(&path, if comp.kind == Probe { theme::colors().accent } else { theme::colors().stroke });
                p.center_text(value_text(v, width, props.radix), w / 2.0, h / 2.0, 1.0, theme::colors().text);
            }
        }
        Led => {
            let v = live.pin(0);
            let on = v.is_some_and(|v| v.is_one());
            let c = p.circle(1.0, 1.0, 0.8);
            let fill = match v {
                Some(v) if v.has_x() => theme::colors().wire_x,
                _ if on => Color::from_rgb8(0xff, 0x3b, 0x30),
                _ => Color::from_rgb8(0x5a, 0x1f, 0x1c),
            };
            p.fill_path(&c, fill);
            p.stroke_path(&c, theme::colors().stroke);
            if on {
                let glow = p.circle(1.0, 1.0, 1.0);
                let col = Color { a: 0.25, ..Color::from_rgb8(0xff, 0x3b, 0x30) };
                p.fill_path(&glow, col);
            }
        }
        Button => {
            let pressed = live.q.is_some_and(|q| q.is_one());
            let outer = p.rounded(0.15, 0.15, 1.85, 1.85, 0.25);
            p.body(&outer);
            let inner = p.circle(1.0, 1.0, 0.55);
            p.fill_path(&inner, if pressed { theme::colors().accent } else { theme::colors().hover });
            p.stroke_path(&inner, theme::colors().stroke);
        }
        Clock => {
            let v = live.q.or(live.pin(0));
            let outer = p.rounded(0.15, 0.15, 1.85, 1.85, 0.25);
            p.body(&outer);
            let wave = p.poly(&[(0.4, 1.4), (0.75, 1.4), (0.75, 0.6), (1.25, 0.6), (1.25, 1.4), (1.6, 1.4)], false);
            let color = v.map(theme::bit_fill).unwrap_or(theme::colors().stroke);
            p.stroke_path(&wave, color);
        }
        Constant => {
            let v = Signal::new(width, props.value);
            let at = p.m(w - 0.3, h / 2.0);
            let s = if width == 1 { v.bit(0).to_char().to_string() } else { format!("0x{}", v.to_hex_string()) };
            p.text(s, at, 1.0, theme::colors().stroke, TextAlign::Right, Vertical::Center);
            p.line((w - 0.2, h / 2.0), (w, h / 2.0), theme::colors().stroke);
        }
        Keyboard => {
            let r = p.rounded(0.0, 0.0, w, h, 0.3);
            p.body(&r);
            p.center_text("KEYBOARD", w / 2.0, 0.75, 0.5, theme::colors().muted);
            // The characters waiting, oldest first, in a text field.
            let (x0, x1) = (2.6, w - 2.6);
            let field = p.rounded(x0, 1.4, x1, 3.0, 0.2);
            p.fill_path(&field, theme::colors().view);
            p.stroke_path(&field, theme::colors().frame);
            let keys = lb_core::component::eval::keyboard_buffer(&live.mem);
            let cw = 0.5;
            let fit = ((x1 - x0 - 0.4) / cw) as usize;
            for (i, &k) in keys.iter().take(fit).enumerate() {
                let s = match k as u8 {
                    b'\n' | b'\r' => "↵".to_string(),
                    8 | 127 => "←".to_string(),
                    c @ 33..=126 => (c as char).to_string(),
                    _ => continue,
                };
                let x = x0 + 0.2 + cw * (i as f32 + 0.5);
                p.center_text(s, x, 2.2, 0.75, theme::colors().text);
            }
            if !live.mem.is_empty() {
                let count = format!("{}/{}", keys.len(), live.mem.len() - 1);
                p.center_text(count, w / 2.0, 3.5, 0.45, theme::colors().muted);
            }
            draw_box_pin_labels(&mut p, comp.kind);
        }
        NumDisplay | HexDisplay => {
            let r = p.rounded(0.0, 0.0, w, h, 0.3);
            p.fill_path(&r, DISPLAY_BG);
            p.stroke_path(&r, theme::colors().stroke);
            let n = props.num_digits() as usize;
            let masks = number_segments(live.pin(0), props.radix, n);
            let sw = p.view.scale * 0.28;
            for (k, &mask) in masks.iter().enumerate() {
                draw_digit(&mut p, 1.2 + 3.0 * k as f32, 1.2, 1.6, 3.6, mask as u16, sw);
            }
        }
        SevenSeg => {
            let r = p.rounded(0.0, 0.0, w, h, 0.3);
            p.fill_path(&r, DISPLAY_BG);
            p.stroke_path(&r, theme::colors().stroke);
            let n = props.seg_digits() as usize;
            let sw = (p.view.scale * 0.35).max(1.5);
            for k in 0..n {
                // One pin per segment, or one 8-bit pin per digit, under it
                // (the pins go left to right, `dig0` being the rightmost).
                let mask = if n == 1 {
                    (0..8).filter(|&i| live.pin(i).is_some_and(|v| v.is_one())).fold(0, |m, i| m | 1 << i)
                } else {
                    let v = live.pin(k);
                    (0..8).filter(|&i| v.is_some_and(|v| v.bit(i) == Bit::One)).fold(0, |m, i| m | 1 << i)
                };
                draw_digit(&mut p, 1.4 + 4.0 * k as f32, 1.3, 2.4, 6.4, mask | DP_SHOWN, sw);
            }
        }
        LedMatrix => {
            let r = p.rounded(0.0, 0.0, w, h, 0.3);
            p.fill_path(&r, DISPLAY_BG);
            p.stroke_path(&r, theme::colors().stroke);
            let (cols, rows) = props.size_of(LedMatrix);
            let radius = 0.36 * p.view.scale;
            let mut paths = [Builder::new(), Builder::new(), Builder::new()];
            for row in 0..rows {
                let v = live.pin(row as usize);
                for col in 0..cols {
                    // The most significant bit is on the left.
                    let bit = v.map(|v| v.bit((cols - 1 - col) as u8));
                    let k = match bit {
                        Some(Bit::One) => 0,
                        Some(Bit::X) => 2,
                        _ => 1,
                    };
                    paths[k].circle(p.m(col as f32 + 1.0, row as f32 + 1.0), radius);
                }
            }
            let [lit, dim, unknown] = paths.map(Builder::build);
            p.fill_path(&dim, SEG_DIM);
            p.fill_path(&unknown, theme::colors().wire_x);
            p.fill_path(&lit, SEG_LIT);
        }
        Tty => {
            let r = p.rounded(0.0, 0.0, w, h, 0.3);
            p.body(&r);
            let (cols, rows) = props.size_of(Tty);
            let (x0, y0) = (3.0, 1.0);
            let (cw, x1, y1) = (0.6, w - 0.4, y0 + rows as f32);
            let screen = p.rounded(x0 - 0.4, y0 - 0.4, x1, y1 + 0.4, 0.2);
            p.fill_path(&screen, TTY_BG);
            let (cols, rows) = (cols as usize, rows as usize);
            if live.mem.len() == cols * rows + 1 {
                for (i, &ch) in live.mem[..cols * rows].iter().enumerate() {
                    if ch > 32 && ch < 127 {
                        let (x, y) = (x0 + cw * ((i % cols) as f32 + 0.5), y0 + (i / cols) as f32 + 0.5);
                        p.center_text((ch as u8 as char).to_string(), x, y, 0.9, TTY_TEXT);
                    }
                }
                // Cursor.
                let i = live.mem[cols * rows] as usize;
                if i < cols * rows {
                    let (x, y) = (x0 + cw * (i % cols) as f32, y0 + (i / cols) as f32 + 0.85);
                    let (tl, size) = p.screen_rect(x + 0.05, y, x + cw - 0.05, y + 0.1);
                    p.fill_path(&Path::rectangle(tl, size), TTY_TEXT);
                }
            }
            draw_box_pin_labels(&mut p, comp.kind);
        }
        Screen => {
            let r = p.rounded(0.0, 0.0, w, h, 0.3);
            p.body(&r);
            let (pw, ph) = props.size_of(Screen);
            let s = screen_pixel(pw, ph);
            let (x0, y0) = (3.0, 1.0);
            let (tl, size) = p.screen_rect(x0, y0, x0 + pw as f32 * s, y0 + ph as f32 * s);
            p.fill_path(&Path::rectangle(tl, size), Color::BLACK);
            if live.mem.len() == (pw * ph) as usize {
                // One path per colour.
                let mut by_color: std::collections::BTreeMap<u64, Builder> = Default::default();
                for (i, &v) in live.mem.iter().enumerate() {
                    if v == 0 {
                        continue;
                    }
                    let (x, y) = ((i as i32 % pw) as f32 * s + x0, (i as i32 / pw) as f32 * s + y0);
                    let (tl, size) = p.screen_rect(x, y, x + s, y + s);
                    by_color.entry(v).or_default().rectangle(tl, size);
                }
                for (v, b) in by_color {
                    let (r, g, bl) = props.colors.rgb(v);
                    p.fill_path(&b.build(), Color::from_rgb8(r, g, bl));
                }
            }
            draw_box_pin_labels(&mut p, comp.kind);
        }
        And | Nand | Or | Nor | Xor | Xnor => draw_gate(&mut p, comp.kind, w, h),
        Not | Buffer => {
            let tri = p.poly(&[(0.3, 0.15), (2.4, 1.0), (0.3, 1.85)], true);
            p.body(&tri);
            p.line((0.0, 1.0), (0.3, 1.0), theme::colors().stroke);
            if comp.kind == Not {
                let b = p.circle(2.65, 1.0, 0.25);
                p.body(&b);
                p.line((2.9, 1.0), (3.0, 1.0), theme::colors().stroke);
            } else {
                p.line((2.4, 1.0), (3.0, 1.0), theme::colors().stroke);
            }
        }
        TriBuffer => {
            let tri = p.poly(&[(0.5, 0.15), (3.2, 1.0), (0.5, 1.85)], true);
            p.body(&tri);
            p.line((0.0, 1.0), (0.5, 1.0), theme::colors().stroke);
            p.line((3.2, 1.0), (4.0, 1.0), theme::colors().stroke);
            p.line((2.0, 2.0), (2.0, 1.47), theme::colors().stroke);
        }
        Mux | Demux | Decoder => {
            let shape = if comp.kind == Mux {
                p.poly(&[(0.0, 0.0), (w - 0.3, 1.0), (w - 0.3, h - 1.0), (0.0, h)], true)
            } else {
                p.poly(&[(0.3, 1.0), (w, 0.0), (w, h), (0.3, h - 1.0)], true)
            };
            p.body(&shape);
            let label = match comp.kind {
                Mux => "MUX",
                Demux => "DMX",
                _ => "DEC",
            };
            p.center_text(label, w / 2.0, h / 2.0, 0.75, theme::colors().muted);
            draw_pin_stubs(&mut p, comp.kind, w, h);
            draw_index_labels(&mut p, comp.kind);
        }
        Splitter | Merger => {
            let n = h;
            let bus = p.c(theme::colors().stroke);
            let thick = Stroke::default().with_color(bus).with_width(p.line_width() * 2.0);
            if comp.kind == Splitter {
                let d = p.poly(&[(0.0, 0.0), (1.0, 1.0), (1.0, n)], false);
                p.frame.stroke(&d, thick);
            } else {
                let d = p.poly(&[(2.0, 0.0), (1.0, 1.0), (1.0, n)], false);
                p.frame.stroke(&d, thick);
            }
            let pieces = props.pieces();
            let mut lo = 0u8;
            for (i, piece) in pieces.iter().enumerate() {
                let y = i as f32 + 1.0;
                let (a, b) = if comp.kind == Splitter { ((1.0, y), (2.0, y)) } else { ((0.0, y), (1.0, y)) };
                p.line(a, b, theme::colors().stroke);
                if p.view.scale >= 10.0 {
                    let label = if *piece == 1 { format!("{lo}") } else { format!("{}:{lo}", lo + piece - 1) };
                    let at = p.m(1.5, y - 0.3);
                    p.text(label, at, 0.55, theme::colors().muted, TextAlign::Center, Vertical::Center);
                }
                lo += piece;
            }
        }
        Tunnel => {
            let path = p.poly(&[(0.0, 1.0), (0.8, 0.2), (w - 0.15, 0.2), (w - 0.15, 1.8), (0.8, 1.8)], true);
            p.body(&path);
            let (label, color) = match props.label.trim() {
                "" => ("?".to_string(), theme::colors().muted),
                l => (l.to_string(), theme::colors().text),
            };
            p.center_text(label, (0.8 + w - 0.15) / 2.0, h / 2.0, 0.8, color);
        }
        Text => {
            let at = p.m(0.0, h / 2.0);
            let color = theme::colors().text;
            p.text(props.label.clone(), at, 1.0, color, TextAlign::Left, Vertical::Center);
        }
        Rect => {
            let r = p.rounded(0.0, 0.0, w, h, 0.4);
            p.fill_path(&r, Color { a: 0.035, ..theme::colors().accent });
            let col = p.c(theme::colors().annotation);
            let dash = [6.0, 4.0];
            p.frame.stroke(
                &r,
                Stroke {
                    line_dash: LineDash { segments: &dash, offset: 0 },
                    ..Stroke::default().with_color(col).with_width(1.2)
                },
            );
            if !props.label.is_empty() {
                let (tl, _) = p.screen_rect(0.0, 0.0, w, h);
                let at = Point::new(tl.x + 0.4 * p.view.scale, tl.y + 0.35 * p.view.scale);
                p.text(props.label.clone(), at, 0.8, theme::colors().annotation, TextAlign::Left, Vertical::Top);
            }
        }
        _ => {
            // Box components.
            let r = p.rounded(0.0, 0.0, w, h, 0.3);
            p.body(&r);
            let title: String = match comp.kind {
                Adder => "+".into(),
                Subtractor => "−".into(),
                Comparator => "CMP".into(),
                Shifter => match props.shift {
                    ShiftMode::Lsl => "<<",
                    ShiftMode::Lsr => ">>",
                    ShiftMode::Asr => ">>>",
                    ShiftMode::RotLeft => "ROL",
                    ShiftMode::RotRight => "ROR",
                }
                .into(),
                Extend => if props.signed { "SEXT" } else { "EXT" }.into(),
                PriorityEncoder => "PRI".into(),
                SrLatch => "SR".into(),
                DLatch => "D".into(),
                DFlipFlop => "D".into(),
                TFlipFlop => "T".into(),
                JkFlipFlop => "JK".into(),
                Register | Counter => live.q.map(|q| q.to_hex_string()).unwrap_or_else(|| "REG".into()),
                ShiftRegister => live.q.map(|q| q.to_bin_string()).unwrap_or_else(|| "SHR".into()),
                Rom => "ROM".into(),
                Ram => "RAM".into(),
                Subcircuit => {
                    if props.circuit.is_empty() {
                        "?".into()
                    } else {
                        props.circuit.clone()
                    }
                }
                _ => String::new(),
            };
            let big = matches!(comp.kind, Adder | Subtractor);
            let cy = if matches!(comp.kind, Register | Counter | ShiftRegister) { h / 2.0 + 0.6 } else { h / 2.0 };
            let size = if big { 1.6 } else { 0.8 };
            let color = if matches!(comp.kind, Register | Counter | ShiftRegister) {
                theme::colors().text
            } else {
                theme::colors().muted
            };
            p.center_text(title, w / 2.0, cy, size, color);
            if matches!(comp.kind, Register | Counter | ShiftRegister) {
                let t = match comp.kind {
                    Register => "REG",
                    Counter => "CTR",
                    _ => "SHR",
                };
                p.center_text(t, w / 2.0, 0.9, 0.55, theme::colors().muted);
            }
            draw_box_pin_labels(&mut p, comp.kind);
        }
    }

    // Label above the component.
    if !props.label.is_empty() && !matches!(comp.kind, Tunnel | Text | Rect) {
        let (x0, y0, x1, _) = geom.bounds();
        let at = view.to_screen(((x0 + x1) as f32 / 2.0, y0 as f32 - 0.25));
        p.text(props.label.clone(), at, 0.75, theme::colors().muted, TextAlign::Center, Vertical::Bottom);
    }
}

/// Background of LED displays.
const DISPLAY_BG: Color = Color::from_rgb8(0x1e, 0x22, 0x2b);
const SEG_LIT: Color = Color::from_rgb8(0xff, 0x45, 0x3a);
const SEG_DIM: Color = Color::from_rgb8(0x3a, 0x2a, 0x2c);
const TTY_BG: Color = Color::from_rgb8(0x0d, 0x11, 0x17);
const TTY_TEXT: Color = Color::from_rgb8(0x7e, 0xe7, 0x87);

/// Segments (bit 0 = a ... bit 6 = g) of the hex digits.
const HEX_SEGMENTS: [u8; 16] =
    [0x3f, 0x06, 0x5b, 0x4f, 0x66, 0x6d, 0x7d, 0x07, 0x7f, 0x6f, 0x77, 0x7c, 0x39, 0x5e, 0x79, 0x71];
const MINUS: u8 = 0x40;
/// Draws the decimal point of a digit (bit 7 tells whether it is lit).
const DP_SHOWN: u16 = 0x100;

/// Segments of the `n` digits of a number display showing `v`: the lowest
/// digits of the value, zero padded in hex and binary, blank padded in
/// decimal; dashes for an unknown value, all off without a value.
fn number_segments(v: Option<Signal>, radix: Radix, n: usize) -> Vec<u8> {
    let Some(v) = v else { return vec![0; n] };
    let text = match (radix, v.to_u64()) {
        (_, None) => return vec![MINUS; n],
        (Radix::Hex, Some(x)) => format!("{x:0n$x}"),
        (Radix::Bin, Some(x)) => format!("{x:0n$b}"),
        (Radix::Dec, Some(x)) => format!("{x:>n$}"),
        (Radix::Signed, _) => format!("{:>n$}", v.to_i64().unwrap_or(0)),
    };
    let chars: Vec<char> = text.chars().collect();
    chars[chars.len() - n..]
        .iter()
        .map(|&c| match c {
            '-' => MINUS,
            c => c.to_digit(16).map_or(0, |d| HEX_SEGMENTS[d as usize]),
        })
        .collect()
}

/// Draws a 7-segment digit in the box from `(x, y)`, `w` x `h` (canonical
/// units), lit as `mask` tells (bit 0 = a ... bit 6 = g, bit 7 = dp; the
/// point is drawn only with [`DP_SHOWN`]).
fn draw_digit(p: &mut Painter, x: f32, y: f32, w: f32, h: f32, mask: u16, sw: f32) {
    let (x1, ym, y2) = (x + w, y + h / 2.0, y + h);
    let segs = [
        ((x, y), (x1, y)),
        ((x1, y), (x1, ym)),
        ((x1, ym), (x1, y2)),
        ((x, y2), (x1, y2)),
        ((x, ym), (x, y2)),
        ((x, y), (x, ym)),
        ((x, ym), (x1, ym)),
    ];
    let (mut lit, mut dim) = (Builder::new(), Builder::new());
    for (i, (a, b)) in segs.iter().enumerate() {
        let path = if mask & 1 << i != 0 { &mut lit } else { &mut dim };
        path.move_to(p.m(a.0, a.1));
        path.line_to(p.m(b.0, b.1));
    }
    for (path, color) in [(dim, SEG_DIM), (lit, SEG_LIT)] {
        let color = p.c(color);
        p.frame.stroke(&path.build(), Stroke::default().with_color(color).with_width(sw));
    }
    if mask & DP_SHOWN != 0 {
        let dp = p.circle(x1 + 0.6, y2 + 0.4, 0.25);
        p.fill_path(&dp, if mask & 0x80 != 0 { SEG_LIT } else { SEG_DIM });
    }
}

fn draw_gate(p: &mut Painter, kind: Kind, w: f32, h: f32) {
    let inverted = matches!(kind, Kind::Nand | Kind::Nor | Kind::Xnor);
    let x1 = w - 0.5;
    let mid = h / 2.0;
    match kind {
        Kind::And | Kind::Nand => {
            let xs = x1 * 0.45;
            let mut b = Builder::new();
            b.move_to(p.m(0.0, 0.0));
            b.line_to(p.m(xs, 0.0));
            b.bezier_curve_to(p.m(xs + K * (x1 - xs), 0.0), p.m(x1, mid - K * mid), p.m(x1, mid));
            b.bezier_curve_to(p.m(x1, mid + K * mid), p.m(xs + K * (x1 - xs), h), p.m(xs, h));
            b.line_to(p.m(0.0, h));
            b.close();
            let path = b.build();
            p.body(&path);
        }
        _ => {
            let xor = matches!(kind, Kind::Xor | Kind::Xnor);
            let x0 = if xor { 0.45 } else { 0.0 };
            let c = (x1 - x0) * 0.28;
            let mut b = Builder::new();
            b.move_to(p.m(x0, 0.0));
            b.bezier_curve_to(p.m(x0 + (x1 - x0) * 0.5, 0.0), p.m(x0 + (x1 - x0) * 0.85, h * 0.12), p.m(x1, mid));
            b.bezier_curve_to(p.m(x0 + (x1 - x0) * 0.85, h * 0.88), p.m(x0 + (x1 - x0) * 0.5, h), p.m(x0, h));
            b.quadratic_curve_to(p.m(x0 + c, mid), p.m(x0, 0.0));
            b.close();
            let path = b.build();
            p.body(&path);
            if xor {
                let mut b = Builder::new();
                b.move_to(p.m(0.0, h));
                b.quadratic_curve_to(p.m(c, mid), p.m(0.0, 0.0));
                let back = b.build();
                p.stroke_path(&back, theme::colors().stroke);
            }
            // Input stubs up to the curved back.
            for pin in p.geom.layout.pins.clone().iter().filter(|q| q.dir == PinDir::In) {
                let y = pin.pos.1 as f32;
                let t = y / h;
                let xb = 2.0 * t * (1.0 - t) * c + if xor { 0.45 } else { 0.0 };
                p.line((0.0, y), (xb, y), theme::colors().stroke);
            }
        }
    }
    if inverted {
        let b = p.circle(x1 + 0.25, mid, 0.25);
        p.body(&b);
    } else {
        p.line((x1, mid), (w, mid), theme::colors().stroke);
    }
}

/// Short lines joining slanted shapes (mux, demux) to their pins.
fn draw_pin_stubs(p: &mut Painter, kind: Kind, w: f32, h: f32) {
    for pin in p.geom.layout.pins.clone() {
        let (x, y) = (pin.pos.0 as f32, pin.pos.1 as f32);
        match (kind, pin.dir) {
            (Kind::Mux, PinDir::Out) => p.line((w - 0.3, y), (w, y), theme::colors().stroke),
            (_, PinDir::In) if pin.name == "sel" => {
                let edge = if kind == Kind::Mux { h - (x / (w - 0.3)) } else { h - 1.0 + (x - 0.3) / (w - 0.3) };
                p.line((x, y), (x, edge.min(h)), theme::colors().stroke);
            }
            (Kind::Demux, PinDir::In) => p.line((0.0, y), (0.3, y), theme::colors().stroke),
            _ => {}
        }
    }
}

/// Small index numbers next to the data pins of mux/demux/decoder.
fn draw_index_labels(p: &mut Painter, kind: Kind) {
    if p.view.scale < 11.0 {
        return;
    }
    let pins = p.geom.layout.pins.clone();
    for pin in pins.iter() {
        let Some(idx) = pin.name.strip_prefix(if kind == Kind::Mux { "in" } else { "out" }) else { continue };
        let (x, y) = (pin.pos.0 as f32, pin.pos.1 as f32);
        let inward = if pin.dir == PinDir::In { 0.45 } else { -0.45 };
        p.center_text(idx.to_string(), x + inward, y, 0.5, theme::colors().muted);
    }
}

/// Pin names inside box components and the clock triangle.
fn draw_box_pin_labels(p: &mut Painter, kind: Kind) {
    let show_names = !matches!(kind, Kind::Adder | Kind::Subtractor | Kind::Extend);
    let pins = p.geom.layout.pins.clone();
    for (i, pin) in pins.iter().enumerate() {
        let d = pin_direction(p.geom, i);
        let world = p.geom.pins[i];
        let (px, py) = (world.0 as f32, world.1 as f32);
        if pin.name == "clk" {
            // Triangle pointing inward.
            let perp = (d.1 as f32, d.0 as f32);
            let a = (px + perp.0 * 0.35, py + perp.1 * 0.35);
            let b = (px - perp.0 * 0.35, py - perp.1 * 0.35);
            let apex = (px - d.0 as f32 * 0.45, py - d.1 as f32 * 0.45);
            let mut bl = Builder::new();
            bl.move_to(p.view.to_screen(a));
            bl.line_to(p.view.to_screen(apex));
            bl.line_to(p.view.to_screen(b));
            let path = bl.build();
            p.stroke_path(&path, theme::colors().stroke);
            continue;
        }
        if !show_names || p.view.scale < 11.0 {
            continue;
        }
        // Primitive pins are shown in upper case; subcircuit pins keep the
        // labels typed by the user.
        let name = match pin.name.as_str() {
            "nq" => "Q'".to_string(),
            n if kind == Kind::Subcircuit => n.to_string(),
            n => n.to_uppercase(),
        };
        let at = p.view.to_screen((px - d.0 as f32 * 0.25, py - d.1 as f32 * 0.25));
        let (align, valign) = match d {
            (-1, 0) => (TextAlign::Left, Vertical::Center),
            (1, 0) => (TextAlign::Right, Vertical::Center),
            (0, -1) => (TextAlign::Center, Vertical::Top),
            _ => (TextAlign::Center, Vertical::Bottom),
        };
        p.text(name.to_string(), at, 0.5, theme::colors().muted, align, valign);
    }
}

/// Draws the pin markers of a component: a small dot coloured by value for
/// connected pins, a hollow ring for unconnected ones.
pub fn draw_pins(frame: &mut Frame, view: View, geom: &CompGeom, live: &Live, connected: &dyn Fn(usize) -> bool) {
    let r = (view.scale * 0.16).clamp(1.5, 4.0);
    for (i, &(x, y)) in geom.pins.iter().enumerate() {
        let at = view.to_screen((x as f32, y as f32));
        if connected(i) {
            continue;
        }
        let ring = Path::circle(at, r);
        frame.fill(&ring, theme::colors().body);
        let col = match live.pin(i) {
            Some(v) if v.has_x() => theme::colors().wire_x,
            _ => theme::colors().muted,
        };
        frame.stroke(&ring, Stroke::default().with_color(col).with_width(1.0));
    }
}
