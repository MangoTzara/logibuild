//! Pin geometry of every component, in grid units.
//!
//! Layouts are defined in a canonical orientation (facing east: inputs on the
//! left, outputs on the right) with the bounding box spanning `(0, 0)` to
//! `(w, h)`. A [`Transform`] applies mirroring and rotation; the component's
//! `pos` is the top-left corner of the transformed bounding box.
//!
//! The order of the pins matters: the n-th input (output) pin of a layout is
//! the n-th input (output) of the evaluation code in [`super::eval`].

use super::{Kind, Props, index_width, indexed};
use crate::circuit::Circuit;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PinDir {
    In,
    Out,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PinDef {
    pub name: String,
    /// Position in the canonical (untransformed) frame.
    pub pos: (i32, i32),
    pub dir: PinDir,
    pub width: u8,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Layout {
    /// Canonical width and height.
    pub w: i32,
    pub h: i32,
    pub pins: Vec<PinDef>,
}

impl Layout {
    fn new(w: i32, h: i32) -> Self {
        Layout { w, h, pins: Vec::new() }
    }

    fn input(&mut self, name: impl Into<String>, x: i32, y: i32, width: u8) -> &mut Self {
        self.pins.push(PinDef { name: name.into(), pos: (x, y), dir: PinDir::In, width });
        self
    }

    fn output(&mut self, name: impl Into<String>, x: i32, y: i32, width: u8) -> &mut Self {
        self.pins.push(PinDef { name: name.into(), pos: (x, y), dir: PinDir::Out, width });
        self
    }

    pub fn pin(&self, name: &str) -> Option<(usize, &PinDef)> {
        self.pins.iter().enumerate().find(|(_, p)| p.name == name)
    }

    pub fn inputs(&self) -> impl Iterator<Item = &PinDef> {
        self.pins.iter().filter(|p| p.dir == PinDir::In)
    }

    pub fn outputs(&self) -> impl Iterator<Item = &PinDef> {
        self.pins.iter().filter(|p| p.dir == PinDir::Out)
    }
}

/// Mirror (flip top/bottom in the canonical frame) followed by a clockwise
/// rotation of `rot` quarter turns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Transform {
    pub rot: u8,
    pub mirror: bool,
    pub w: i32,
    pub h: i32,
}

impl Transform {
    pub fn new(rot: u8, mirror: bool, layout: &Layout) -> Self {
        Transform { rot: rot % 4, mirror, w: layout.w, h: layout.h }
    }

    /// Size of the transformed bounding box.
    pub fn size(&self) -> (i32, i32) {
        if self.rot.is_multiple_of(2) { (self.w, self.h) } else { (self.h, self.w) }
    }

    /// Maps a canonical point into the transformed frame.
    pub fn apply(&self, (x, y): (i32, i32)) -> (i32, i32) {
        let y = if self.mirror { self.h - y } else { y };
        match self.rot {
            0 => (x, y),
            1 => (self.h - y, x),
            2 => (self.w - x, self.h - y),
            _ => (y, self.w - x),
        }
    }

    /// Floating-point version of [`Transform::apply`], for drawing.
    pub fn apply_f(&self, (x, y): (f32, f32)) -> (f32, f32) {
        let (w, h) = (self.w as f32, self.h as f32);
        let y = if self.mirror { h - y } else { y };
        match self.rot {
            0 => (x, y),
            1 => (h - y, x),
            2 => (w - x, h - y),
            _ => (y, w - x),
        }
    }
}

/// Positions for `n` stacked inputs, Logisim style: one unit apart, leaving
/// the middle free when `n` is even so the output can sit there.
/// Returns `(height, ys, middle_y)`.
pub fn stack(n: usize) -> (i32, Vec<i32>, i32) {
    let n = n.max(1) as i32;
    if n == 1 {
        return (2, vec![1], 1);
    }
    if n % 2 == 1 {
        (n + 1, (1..=n).collect(), (n + 1) / 2)
    } else {
        let half = n / 2;
        let ys = (1..=half).chain(half + 2..=n + 1).collect();
        (n + 2, ys, half + 1)
    }
}

fn value_box_width(width: u8) -> i32 {
    if width == 1 {
        2
    } else {
        let digits = width.div_ceil(4) as i32;
        (digits * 4 + 4) / 5 + 2
    }
}

/// Segment pins of a one-digit 7-segment display, in bit order of the
/// 8-bit digit inputs of a multi-digit one.
pub const SEGMENTS: [&str; 8] = ["a", "b", "c", "d", "e", "f", "g", "dp"];

/// Size of a pixel of a `w` x `h` screen, in grid units.
pub fn screen_pixel(w: i32, h: i32) -> f32 {
    match w.max(h) {
        ..=16 => 1.0,
        17..=32 => 0.5,
        _ => 0.25,
    }
}

/// Bits needed to express a shift amount for a `width`-bit operand.
pub fn shift_amount_width(width: u8) -> u8 {
    let mut bits = 1;
    while (1u32 << bits) < width as u32 {
        bits += 1;
    }
    bits
}

/// Label used as pin name for a subcircuit terminal.
pub fn terminal_name(c: &crate::circuit::Component) -> String {
    if c.props.label.trim().is_empty() {
        let p = if c.kind == Kind::Input { "in" } else { "out" };
        format!("{p}#{}", c.id)
    } else {
        c.props.label.trim().to_string()
    }
}

/// Input/Output components of `circuit`, in terminal order (top to bottom,
/// then left to right) with unique names.
/// A subcircuit terminal: `(pin name, width, id of the Input/Output component)`.
pub type Terminal = (String, u8, u32);

pub fn terminals(circuit: &Circuit) -> (Vec<Terminal>, Vec<Terminal>) {
    let mut ins: Vec<&crate::circuit::Component> =
        circuit.components.iter().filter(|c| c.kind == Kind::Input).collect();
    let mut outs: Vec<&crate::circuit::Component> =
        circuit.components.iter().filter(|c| c.kind == Kind::Output).collect();
    let key = |c: &&crate::circuit::Component| (c.pos.1, c.pos.0, c.id);
    ins.sort_by_key(key);
    outs.sort_by_key(key);
    let mut seen = std::collections::HashSet::new();
    let mut named = |list: Vec<&crate::circuit::Component>| {
        list.into_iter()
            .map(|c| {
                let mut name = terminal_name(c);
                if !seen.insert(name.clone()) {
                    name = format!("{name}#{}", c.id);
                    seen.insert(name.clone());
                }
                (name, c.props.width(), c.id)
            })
            .collect::<Vec<_>>()
    };
    let i = named(ins);
    let o = named(outs);
    (i, o)
}

/// Computes the layout of a component. `sub` must be the referenced circuit
/// for [`Kind::Subcircuit`] (a missing circuit yields a pinless box).
pub fn layout(kind: Kind, props: &Props, sub: Option<&Circuit>) -> Layout {
    use Kind::*;
    let w = props.width();
    match kind {
        Input => {
            let bw = value_box_width(w);
            let mut l = Layout::new(bw, 2);
            l.output("out", bw, 1, w);
            l
        }
        Constant => {
            let bw = value_box_width(w);
            let mut l = Layout::new(bw, 2);
            l.output("out", bw, 1, w);
            l
        }
        Button | Clock => {
            let mut l = Layout::new(2, 2);
            l.output("out", 2, 1, 1);
            l
        }
        Output | Probe => {
            let bw = value_box_width(w);
            let mut l = Layout::new(bw, 2);
            l.input("in", 0, 1, w);
            l
        }
        Led => {
            let mut l = Layout::new(2, 2);
            l.input("in", 0, 1, 1);
            l
        }
        Tunnel => {
            // A flag pointing at its pin, long enough for the label.
            let len = props.label.trim().chars().count() as i32;
            let mut l = Layout::new(1 + ((len + 2) / 2).max(2), 2);
            l.input("in", 0, 1, w);
            l
        }
        Keyboard => {
            let mut l = Layout::new(12, 4);
            l.input("read", 0, 1, 1).input("clk", 0, 2, 1).input("clear", 0, 3, 1);
            l.output("data", 12, 1, 7).output("avail", 12, 2, 1);
            l
        }
        NumDisplay | HexDisplay => {
            // A digit every 3 units; one digit is the 4 x 6 hex display of
            // older files.
            let n = props.num_digits() as i32;
            let mut l = Layout::new(3 * n + 1, 6);
            l.input("in", 0, 3, w);
            l
        }
        SevenSeg => {
            let n = props.seg_digits() as i32;
            let mut l = Layout::new(4 * n + 1, 9);
            if n == 1 {
                for (i, s) in SEGMENTS.iter().enumerate() {
                    l.input(*s, 0, i as i32 + 1, 1);
                }
            } else {
                // One 8-bit input under each digit, `dig0` the rightmost.
                for k in 0..n {
                    l.input(indexed("dig", (n - 1 - k) as usize), 4 * k + 3, 9, 8);
                }
            }
            l
        }
        LedMatrix => {
            let (cols, rows) = props.size_of(kind);
            let mut l = Layout::new(cols + 1, rows + 1);
            for r in 0..rows {
                l.input(indexed("row", r as usize), 0, r + 1, cols as u8);
            }
            l
        }
        Tty => {
            let (cols, rows) = props.size_of(kind);
            // Pin names on the left, then the text area.
            let mut l = Layout::new((cols * 3 + 4) / 5 + 5, (rows + 2).max(5));
            l.input("data", 0, 1, 7).input("write", 0, 2, 1).input("clk", 0, 3, 1).input("clear", 0, 4, 1);
            l
        }
        Screen => {
            let (pw, ph) = props.size_of(kind);
            let s = screen_pixel(pw, ph);
            // Pin names on the left, then the pixel area.
            let w = (pw as f32 * s).ceil() as i32 + 4;
            let h = ((ph as f32 * s).ceil() as i32 + 2).max(7);
            let mut l = Layout::new(w, h);
            l.input("x", 0, 1, index_width(pw)).input("y", 0, 2, index_width(ph));
            l.input("color", 0, 3, props.colors.bits()).input("write", 0, 4, 1);
            l.input("clk", 0, 5, 1).input("clear", 0, 6, 1);
            l
        }
        And | Or | Nand | Nor | Xor | Xnor => {
            let n = props.inputs() as usize;
            let (h, ys, mid) = stack(n);
            let bw = if n >= 5 { 5 } else { 4 };
            let mut l = Layout::new(bw, h);
            for (i, y) in ys.into_iter().enumerate() {
                l.input(indexed("in", i), 0, y, w);
            }
            l.output("out", bw, mid, w);
            l
        }
        Not | Buffer => {
            let mut l = Layout::new(3, 2);
            l.input("in", 0, 1, w).output("out", 3, 1, w);
            l
        }
        TriBuffer => {
            let mut l = Layout::new(4, 2);
            l.input("in", 0, 1, w).input("en", 2, 2, 1).output("out", 4, 1, w);
            l
        }
        Mux => {
            let s = props.select();
            let n = 1usize << s;
            let (h, ys, mid) = stack(n);
            let mut l = Layout::new(4, h);
            for (i, y) in ys.into_iter().enumerate() {
                l.input(indexed("in", i), 0, y, w);
            }
            l.input("sel", 2, h, s).output("out", 4, mid, w);
            l
        }
        Demux => {
            let s = props.select();
            let n = 1usize << s;
            let (h, ys, mid) = stack(n);
            let mut l = Layout::new(4, h);
            l.input("in", 0, mid, w).input("sel", 2, h, s);
            for (i, y) in ys.into_iter().enumerate() {
                l.output(indexed("out", i), 4, y, w);
            }
            l
        }
        Decoder => {
            let s = props.select();
            let n = 1usize << s;
            let (h, ys, _) = stack(n);
            let mut l = Layout::new(4, h);
            l.input("sel", 2, h, s);
            for (i, y) in ys.into_iter().enumerate() {
                l.output(indexed("out", i), 4, y, 1);
            }
            l
        }
        PriorityEncoder => {
            let s = props.select();
            let n = 1usize << s;
            let (h, ys, mid) = stack(n);
            let mut l = Layout::new(4, h);
            for (i, y) in ys.into_iter().enumerate() {
                l.input(indexed("in", i), 0, y, 1);
            }
            l.output("out", 4, mid, s).output("v", 2, h, 1);
            l
        }
        Adder => {
            let mut l = Layout::new(4, 4);
            l.input("a", 0, 1, w).input("b", 0, 3, w).input("cin", 2, 0, 1);
            l.output("out", 4, 2, w).output("cout", 2, 4, 1);
            l
        }
        Subtractor => {
            let mut l = Layout::new(4, 4);
            l.input("a", 0, 1, w).input("b", 0, 3, w).input("bin", 2, 0, 1);
            l.output("out", 4, 2, w).output("bout", 2, 4, 1);
            l
        }
        Comparator => {
            let mut l = Layout::new(4, 4);
            l.input("a", 0, 1, w).input("b", 0, 3, w);
            l.output("lt", 4, 1, 1).output("eq", 4, 2, 1).output("gt", 4, 3, 1);
            l
        }
        Shifter => {
            let mut l = Layout::new(4, 4);
            l.input("in", 0, 1, w).input("sh", 0, 3, shift_amount_width(w));
            l.output("out", 4, 2, w);
            l
        }
        Splitter => {
            let pieces = props.pieces();
            let mut l = Layout::new(2, pieces.len() as i32);
            l.input("in", 0, 0, w);
            for (i, p) in pieces.iter().enumerate() {
                l.output(indexed("out", i), 2, i as i32 + 1, *p);
            }
            l
        }
        Merger => {
            let pieces = props.pieces();
            let mut l = Layout::new(2, pieces.len() as i32);
            for (i, p) in pieces.iter().enumerate() {
                l.input(indexed("in", i), 0, i as i32 + 1, *p);
            }
            l.output("out", 2, 0, w);
            l
        }
        Extend => {
            let mut l = Layout::new(4, 2);
            l.input("in", 0, 1, w).output("out", 4, 1, props.out_width());
            l
        }
        SrLatch => {
            let mut l = Layout::new(4, 4);
            l.input("s", 0, 1, 1).input("r", 0, 3, 1);
            l.output("q", 4, 1, 1).output("nq", 4, 3, 1);
            l
        }
        DLatch => {
            let mut l = Layout::new(4, 4);
            l.input("d", 0, 1, 1).input("en", 0, 3, 1);
            l.output("q", 4, 1, 1).output("nq", 4, 3, 1);
            l
        }
        DFlipFlop | TFlipFlop => {
            let mut l = Layout::new(4, 4);
            let d = if kind == DFlipFlop { "d" } else { "t" };
            l.input(d, 0, 1, 1).input("clk", 0, 3, 1);
            l.input("pre", 2, 0, 1).input("rst", 2, 4, 1);
            l.output("q", 4, 1, 1).output("nq", 4, 3, 1);
            l
        }
        JkFlipFlop => {
            let mut l = Layout::new(4, 4);
            l.input("j", 0, 1, 1).input("k", 0, 3, 1).input("clk", 0, 2, 1);
            l.input("pre", 2, 0, 1).input("rst", 2, 4, 1);
            l.output("q", 4, 1, 1).output("nq", 4, 3, 1);
            l
        }
        Register => {
            let mut l = Layout::new(4, 4);
            l.input("d", 0, 1, w).input("en", 0, 2, 1).input("clk", 0, 3, 1);
            l.input("rst", 2, 4, 1);
            l.output("q", 4, 2, w);
            l
        }
        Counter => {
            let mut l = Layout::new(4, 5);
            l.input("d", 0, 1, w).input("load", 0, 2, 1).input("en", 0, 3, 1);
            l.input("clk", 0, 4, 1).input("rst", 2, 5, 1);
            l.output("q", 4, 2, w).output("carry", 4, 3, 1);
            l
        }
        ShiftRegister => {
            let n = props.length();
            let mut l = Layout::new(4, 5);
            l.input("sin", 0, 1, 1).input("en", 0, 2, 1).input("clk", 0, 4, 1);
            l.input("rst", 2, 5, 1);
            l.output("q", 4, 2, n).output("sout", 4, 3, 1);
            l
        }
        Rom => {
            let mut l = Layout::new(6, 4);
            l.input("addr", 0, 2, props.addr_width()).output("data", 6, 2, w);
            l
        }
        Ram => {
            let mut l = Layout::new(6, 6);
            l.input("addr", 0, 1, props.addr_width()).input("din", 0, 2, w);
            l.input("we", 0, 3, 1).input("clk", 0, 5, 1);
            l.output("dout", 6, 2, w);
            l
        }
        Subcircuit => {
            let name_len = props.circuit.chars().count() as i32;
            let bw = (name_len * 3 / 5 + 2).clamp(4, 14);
            let Some(sub) = sub else {
                return Layout::new(bw, 2);
            };
            let (ins, outs) = terminals(sub);
            let n = ins.len().max(outs.len()).max(1) as i32;
            // Synthesized from HDL: room for the "HDL" mark under the name.
            let h = if sub.hdl.is_some() { (2 * n).max(3) } else { 2 * n };
            let mut l = Layout::new(bw, h);
            for (i, (name, width, _)) in ins.into_iter().enumerate() {
                l.input(name, 0, 1 + 2 * i as i32, width);
            }
            for (i, (name, width, _)) in outs.into_iter().enumerate() {
                l.output(name, bw, 1 + 2 * i as i32, width);
            }
            l
        }
        Text => {
            let len = props.label.chars().count().max(1) as i32;
            Layout::new(((len * 3 + 4) / 5).max(1), 2)
        }
        Rect => {
            let (w, h) = props.rect_size();
            Layout::new(w, h)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdl_subcircuits_have_room_for_their_mark() {
        let mut sub = Circuit::new("dec");
        for (id, kind, y) in [(1, Kind::Input, 0), (2, Kind::Output, 0)] {
            let props = kind.default_props();
            sub.components.push(crate::circuit::Component { id, kind, pos: (0, y), rot: 0, mirror: false, props });
        }
        let props = Props { circuit: "dec".into(), ..Kind::Subcircuit.default_props() };
        assert_eq!(layout(Kind::Subcircuit, &props, Some(&sub)).h, 2);
        sub.hdl = Some(Default::default());
        let l = layout(Kind::Subcircuit, &props, Some(&sub));
        assert_eq!(l.h, 3);
        assert_eq!(l.pins.iter().map(|p| p.pos.1).collect::<Vec<_>>(), [1, 1], "pins stay put");
    }

    #[test]
    fn stacking() {
        assert_eq!(stack(2), (4, vec![1, 3], 2));
        assert_eq!(stack(3), (4, vec![1, 2, 3], 2));
        assert_eq!(stack(4), (6, vec![1, 2, 4, 5], 3));
        assert_eq!(stack(5), (6, vec![1, 2, 3, 4, 5], 3));
    }

    #[test]
    fn transform_rotations() {
        let l = layout(Kind::And, &Kind::And.default_props(), None);
        let out = l.pin("out").unwrap().1.pos;
        assert_eq!(out, (4, 2));
        let t = |rot, mirror| Transform::new(rot, mirror, &l);
        assert_eq!(t(1, false).apply(out), (2, 4)); // facing south
        assert_eq!(t(2, false).apply(out), (0, 2)); // facing west
        assert_eq!(t(3, false).apply(out), (2, 0)); // facing north
        assert_eq!(t(0, true).apply((0, 1)), (0, 3));
        // Every transformed pin stays within the transformed bounding box.
        for rot in 0..4 {
            for mirror in [false, true] {
                let tr = t(rot, mirror);
                let (w, h) = tr.size();
                for p in &l.pins {
                    let (x, y) = tr.apply(p.pos);
                    assert!((0..=w).contains(&x) && (0..=h).contains(&y));
                }
            }
        }
    }

    #[test]
    fn pins_unique_and_on_border() {
        for kind in Kind::ALL {
            let l = layout(kind, &kind.default_props(), None);
            let mut names: Vec<_> = l.pins.iter().map(|p| &p.name).collect();
            names.sort();
            names.dedup();
            assert_eq!(names.len(), l.pins.len(), "{kind:?}");
            for p in &l.pins {
                let (x, y) = p.pos;
                let on_border = x == 0 || y == 0 || x == l.w || y == l.h;
                assert!(on_border, "{kind:?} pin {} not on border", p.name);
            }
        }
    }

    #[test]
    fn shift_widths() {
        assert_eq!(shift_amount_width(1), 1);
        assert_eq!(shift_amount_width(8), 3);
        assert_eq!(shift_amount_width(9), 4);
        assert_eq!(shift_amount_width(64), 6);
    }
}
