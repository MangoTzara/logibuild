//! Simulation behaviour of the primitive components.
//!
//! [`Prim::eval`] computes the outputs of a component from its inputs and its
//! internal [`State`]. Inputs and outputs follow the pin order of
//! [`super::layout::layout`]. `outs` holds the current output values on entry
//! (with the right widths), so a component may leave an output untouched.

use std::ops::Not;

use super::{ColorMode, Edge, Kind, ShiftMode};
use crate::circuit::Component;
use crate::io::parse_hex;
use crate::value::{Bit, Signal, mask};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GateOp {
    And,
    Or,
    Xor,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Prim {
    /// Sources: their output is `State::q`, set by the user or the engine.
    Input,
    Button,
    Constant,
    Clock {
        high: u32,
        low: u32,
    },
    /// Outputs, probes and displays: only read their inputs.
    Sink,
    Gate {
        op: GateOp,
        invert: bool,
    },
    Not,
    Buffer,
    TriBuffer,
    Mux,
    Demux,
    Decoder,
    PriorityEncoder,
    Adder,
    Subtractor,
    Comparator {
        signed: bool,
    },
    Shifter {
        mode: ShiftMode,
    },
    Splitter {
        pieces: Box<[u8]>,
    },
    Merger {
        pieces: Box<[u8]>,
    },
    Extend {
        signed: bool,
    },
    SrLatch,
    DLatch,
    DFlipFlop {
        edge: Edge,
    },
    TFlipFlop {
        edge: Edge,
    },
    JkFlipFlop {
        edge: Edge,
    },
    Register {
        edge: Edge,
    },
    Counter {
        edge: Edge,
    },
    ShiftRegister {
        edge: Edge,
        length: u8,
    },
    Rom,
    Ram {
        edge: Edge,
    },
    /// Keyboard: `mem` holds the number of characters typed, then the
    /// characters, oldest first.
    Keyboard {
        edge: Edge,
    },
    /// Text terminal: `mem` holds the characters, row by row, then the
    /// cursor position.
    Tty {
        cols: u32,
        rows: u32,
        edge: Edge,
    },
    /// Pixel screen: `mem` holds the pixels, row by row.
    Screen {
        w: u32,
        h: u32,
        colors: ColorMode,
        edge: Edge,
    },
}

/// Internal state of a component instance.
#[derive(Clone, PartialEq, Debug)]
pub struct State {
    /// Stored value (flip-flops, registers, sources).
    pub q: Signal,
    /// Clock value seen at the previous evaluation, for edge detection.
    pub last_clk: Bit,
    /// Memory words (ROM/RAM).
    pub mem: Vec<u64>,
}

impl State {
    pub fn new(q: Signal) -> Self {
        State { q, last_clk: Bit::X, mem: Vec::new() }
    }
}

impl Prim {
    /// Builds the primitive and its initial state for a component. Returns
    /// `None` for annotations, tunnels and subcircuits, which have no
    /// primitive.
    pub fn from_component(c: &Component) -> Option<(Prim, State)> {
        let p = &c.props;
        let w = p.width();
        let st = |q| State::new(q);
        let zero = || st(Signal::zero(w));
        let gate = |op, invert| Some((Prim::Gate { op, invert }, zero()));
        Some(match c.kind {
            Kind::Input => (Prim::Input, st(Signal::new(w, p.value))),
            Kind::Button => (Prim::Button, st(Signal::zero(1))),
            Kind::Constant => (Prim::Constant, st(Signal::new(w, p.value))),
            Kind::Clock => {
                let (high, low) = p.clock_phases();
                (Prim::Clock { high, low }, st(Signal::zero(1)))
            }
            Kind::Output
            | Kind::Led
            | Kind::Probe
            | Kind::NumDisplay
            | Kind::HexDisplay
            | Kind::SevenSeg
            | Kind::LedMatrix => (Prim::Sink, zero()),
            Kind::Keyboard => {
                let mem = vec![0; p.key_buffer() as usize + 1];
                (Prim::Keyboard { edge: p.edge }, State { q: Signal::zero(1), last_clk: Bit::X, mem })
            }
            Kind::Tty => {
                let (cols, rows) = p.size_of(c.kind);
                let (cols, rows) = (cols as u32, rows as u32);
                let mem = vec![0; (cols * rows) as usize + 1];
                (Prim::Tty { cols, rows, edge: p.edge }, State { q: Signal::zero(1), last_clk: Bit::X, mem })
            }
            Kind::Screen => {
                let (w, h) = p.size_of(c.kind);
                let (w, h) = (w as u32, h as u32);
                let mem = vec![0; (w * h) as usize];
                let prim = Prim::Screen { w, h, colors: p.colors, edge: p.edge };
                (prim, State { q: Signal::zero(1), last_clk: Bit::X, mem })
            }
            Kind::And => return gate(GateOp::And, false),
            Kind::Nand => return gate(GateOp::And, true),
            Kind::Or => return gate(GateOp::Or, false),
            Kind::Nor => return gate(GateOp::Or, true),
            Kind::Xor => return gate(GateOp::Xor, false),
            Kind::Xnor => return gate(GateOp::Xor, true),
            Kind::Not => (Prim::Not, zero()),
            Kind::Buffer => (Prim::Buffer, zero()),
            Kind::TriBuffer => (Prim::TriBuffer, zero()),
            Kind::Mux => (Prim::Mux, zero()),
            Kind::Demux => (Prim::Demux, zero()),
            Kind::Decoder => (Prim::Decoder, zero()),
            Kind::PriorityEncoder => (Prim::PriorityEncoder, zero()),
            Kind::Adder => (Prim::Adder, zero()),
            Kind::Subtractor => (Prim::Subtractor, zero()),
            Kind::Comparator => (Prim::Comparator { signed: p.signed }, zero()),
            Kind::Shifter => (Prim::Shifter { mode: p.shift }, zero()),
            Kind::Splitter => (Prim::Splitter { pieces: p.pieces().into() }, zero()),
            Kind::Merger => (Prim::Merger { pieces: p.pieces().into() }, zero()),
            Kind::Extend => (Prim::Extend { signed: p.signed }, zero()),
            Kind::SrLatch => (Prim::SrLatch, st(Signal::zero(1))),
            Kind::DLatch => (Prim::DLatch, st(Signal::zero(1))),
            Kind::DFlipFlop => (Prim::DFlipFlop { edge: p.edge }, st(Signal::zero(1))),
            Kind::TFlipFlop => (Prim::TFlipFlop { edge: p.edge }, st(Signal::zero(1))),
            Kind::JkFlipFlop => (Prim::JkFlipFlop { edge: p.edge }, st(Signal::zero(1))),
            Kind::Register => (Prim::Register { edge: p.edge }, zero()),
            Kind::Counter => (Prim::Counter { edge: p.edge }, zero()),
            Kind::ShiftRegister => {
                let length = p.length();
                (Prim::ShiftRegister { edge: p.edge, length }, st(Signal::zero(length)))
            }
            Kind::Rom | Kind::Ram => {
                let words = 1usize << p.addr_width();
                let mut mem = parse_hex(&p.data).unwrap_or_default();
                mem.resize(words, 0);
                for v in &mut mem {
                    *v &= mask(w);
                }
                let prim = if c.kind == Kind::Rom { Prim::Rom } else { Prim::Ram { edge: p.edge } };
                (prim, State { q: Signal::zero(w), last_clk: Bit::X, mem })
            }
            Kind::Subcircuit | Kind::Tunnel | Kind::Text | Kind::Rect => return None,
        })
    }

    /// Whether the component's outputs depend only on state set from outside
    /// (user or clock), never on inputs.
    pub fn is_source(&self) -> bool {
        matches!(self, Prim::Input | Prim::Button | Prim::Constant | Prim::Clock { .. })
    }

    /// Computes outputs from inputs and state.
    pub fn eval(&self, st: &mut State, ins: &[Signal], outs: &mut [Signal]) {
        match self {
            Prim::Input | Prim::Button | Prim::Constant | Prim::Clock { .. } => outs[0] = st.q,
            Prim::Sink => {}
            Prim::Gate { op, invert } => {
                let w = outs[0].width();
                let mut it = ins.iter().map(|s| s.resize(w).z_to_x());
                let r = match it.next() {
                    None => Signal::x(w),
                    Some(first) => it.fold(first, |acc, s| match op {
                        GateOp::And => acc.and(s),
                        GateOp::Or => acc.or(s),
                        GateOp::Xor => acc.xor(s),
                    }),
                };
                outs[0] = if *invert { r.not() } else { r };
            }
            Prim::Not => outs[0] = ins[0].not(),
            Prim::Buffer => outs[0] = ins[0].z_to_x(),
            Prim::TriBuffer => {
                let w = outs[0].width();
                outs[0] = match ins[1].bit(0) {
                    Bit::One => ins[0].z_to_x(),
                    Bit::Zero => Signal::z(w),
                    _ => Signal::x(w),
                };
            }
            Prim::Mux => {
                let n = ins.len() - 1;
                let w = outs[0].width();
                outs[0] = match ins[n].to_u64() {
                    Some(s) if (s as usize) < n => ins[s as usize].z_to_x(),
                    _ => Signal::x(w),
                };
            }
            Prim::Demux => {
                let w = outs[0].width();
                match ins[1].to_u64() {
                    Some(s) => {
                        for (i, o) in outs.iter_mut().enumerate() {
                            *o = if i as u64 == s { ins[0].z_to_x() } else { Signal::zero(w) };
                        }
                    }
                    None => outs.iter_mut().for_each(|o| *o = Signal::x(w)),
                }
            }
            Prim::Decoder => match ins[0].to_u64() {
                Some(s) => {
                    for (i, o) in outs.iter_mut().enumerate() {
                        *o = Signal::bool(i as u64 == s);
                    }
                }
                None => outs.iter_mut().for_each(|o| *o = Signal::x(1)),
            },
            Prim::PriorityEncoder => {
                let w = outs[0].width();
                let mut result = Some(None);
                for i in (0..ins.len()).rev() {
                    match ins[i].bit(0) {
                        Bit::One => {
                            result = Some(Some(i as u64));
                            break;
                        }
                        Bit::X => {
                            result = None;
                            break;
                        }
                        Bit::Zero | Bit::Z => {}
                    }
                }
                (outs[0], outs[1]) = match result {
                    Some(Some(i)) => (Signal::new(w, i), Signal::bool(true)),
                    Some(None) => (Signal::zero(w), Signal::bool(false)),
                    None => (Signal::x(w), Signal::x(1)),
                };
            }
            Prim::Adder | Prim::Subtractor => {
                let w = outs[0].width();
                let carry_in = if ins[2].is_all_z() { Some(0) } else { ins[2].to_u64() };
                let (a, b) = (ins[0].resize(w).to_u64(), ins[1].resize(w).to_u64());
                match (a, b, carry_in) {
                    (Some(a), Some(b), Some(c)) => {
                        let m = mask(w) as u128;
                        if matches!(self, Prim::Adder) {
                            let sum = a as u128 + b as u128 + c as u128;
                            outs[0] = Signal::new(w, sum as u64);
                            outs[1] = Signal::bool(sum > m);
                        } else {
                            let sub = b as u128 + c as u128;
                            outs[0] = Signal::new(w, (a as u128).wrapping_sub(sub) as u64);
                            outs[1] = Signal::bool((a as u128) < sub);
                        }
                    }
                    _ => {
                        outs[0] = Signal::x(w);
                        outs[1] = Signal::x(1);
                    }
                }
            }
            Prim::Comparator { signed } => {
                let ord = if *signed {
                    ins[0].to_i64().zip(ins[1].to_i64()).map(|(a, b)| a.cmp(&b))
                } else {
                    ins[0].to_u64().zip(ins[1].to_u64()).map(|(a, b)| a.cmp(&b))
                };
                match ord {
                    Some(o) => {
                        outs[0] = Signal::bool(o.is_lt());
                        outs[1] = Signal::bool(o.is_eq());
                        outs[2] = Signal::bool(o.is_gt());
                    }
                    None => outs.iter_mut().for_each(|o| *o = Signal::x(1)),
                }
            }
            Prim::Shifter { mode } => {
                let w = outs[0].width();
                let v = ins[0].resize(w);
                outs[0] = match ins[1].to_u64() {
                    None => Signal::x(w),
                    Some(n) => shift(v, n, *mode),
                };
            }
            Prim::Splitter { pieces } => {
                let mut lo = 0;
                for (o, &p) in outs.iter_mut().zip(pieces.iter()) {
                    *o = ins[0].slice(lo, p);
                    lo += p;
                }
            }
            Prim::Merger { pieces } => {
                let mut r = Signal::zero(outs[0].width());
                let mut lo = 0;
                for (i, &p) in ins.iter().zip(pieces.iter()) {
                    r = r.insert(lo, i.resize(p));
                    lo += p;
                }
                outs[0] = r;
            }
            Prim::Extend { signed } => {
                let w = outs[0].width();
                outs[0] = if *signed { ins[0].sign_extend(w) } else { ins[0].resize(w) };
            }
            Prim::SrLatch => {
                let (s, r) = (low_if_z(ins[0]), low_if_z(ins[1]));
                match (s, r) {
                    (Bit::One, Bit::Zero) => st.q = Signal::bool(true),
                    (Bit::Zero, Bit::One) | (Bit::One, Bit::One) => st.q = Signal::bool(false),
                    (Bit::Zero, Bit::Zero) => {}
                    _ => st.q = Signal::x(1),
                }
                outs[0] = st.q;
                // With both inputs active the NOR implementation drives both
                // outputs low.
                outs[1] = if (s, r) == (Bit::One, Bit::One) { st.q } else { st.q.not() };
            }
            Prim::DLatch => {
                match low_if_z(ins[1]) {
                    Bit::One => st.q = ins[0].z_to_x(),
                    Bit::Zero => {}
                    _ => {
                        if ins[0].z_to_x() != st.q {
                            st.q = Signal::x(1);
                        }
                    }
                }
                outs[0] = st.q;
                outs[1] = st.q.not();
            }
            Prim::DFlipFlop { edge } | Prim::TFlipFlop { edge } | Prim::JkFlipFlop { edge } => {
                let (clk_i, pre_i) = if matches!(self, Prim::JkFlipFlop { .. }) { (2, 3) } else { (1, 2) };
                let fired = clock_edge(st, ins[clk_i], *edge);
                let (pre, rst) = (low_if_z(ins[pre_i]), low_if_z(ins[pre_i + 1]));
                if rst == Bit::One {
                    st.q = Signal::bool(false);
                } else if pre == Bit::One {
                    st.q = Signal::bool(true);
                } else if fired {
                    st.q = match self {
                        Prim::DFlipFlop { .. } => ins[0].z_to_x(),
                        Prim::TFlipFlop { .. } => match low_if_z(ins[0]) {
                            Bit::Zero => st.q,
                            Bit::One => st.q.not(),
                            _ => Signal::x(1),
                        },
                        _ => match (low_if_z(ins[0]), low_if_z(ins[1])) {
                            (Bit::Zero, Bit::Zero) => st.q,
                            (Bit::Zero, Bit::One) => Signal::bool(false),
                            (Bit::One, Bit::Zero) => Signal::bool(true),
                            (Bit::One, Bit::One) => st.q.not(),
                            _ => Signal::x(1),
                        },
                    };
                }
                outs[0] = st.q;
                outs[1] = st.q.not();
            }
            Prim::Register { edge } => {
                let fired = clock_edge(st, ins[2], *edge);
                if low_if_z(ins[3]) == Bit::One {
                    st.q = Signal::zero(st.q.width());
                } else if fired && high_if_z(ins[1]) == Bit::One {
                    st.q = ins[0].resize(st.q.width()).z_to_x();
                }
                outs[0] = st.q;
            }
            Prim::Counter { edge } => {
                let fired = clock_edge(st, ins[3], *edge);
                let w = st.q.width();
                if low_if_z(ins[4]) == Bit::One {
                    st.q = Signal::zero(w);
                } else if fired {
                    if low_if_z(ins[1]) == Bit::One {
                        st.q = ins[0].resize(w).z_to_x();
                    } else if high_if_z(ins[2]) == Bit::One {
                        st.q = match st.q.to_u64() {
                            Some(v) => Signal::new(w, v.wrapping_add(1)),
                            None => Signal::x(w),
                        };
                    }
                }
                outs[0] = st.q;
                outs[1] = match st.q.to_u64() {
                    Some(v) => Signal::bool(v == mask(w)),
                    None => Signal::x(1),
                };
            }
            Prim::ShiftRegister { edge, length } => {
                let fired = clock_edge(st, ins[2], *edge);
                if low_if_z(ins[3]) == Bit::One {
                    st.q = Signal::zero(*length);
                } else if fired && high_if_z(ins[1]) == Bit::One {
                    let sin = ins[0].z_to_x().resize(1);
                    let shifted = Signal::from_parts(*length, st.q.val() << 1, st.q.unk() << 1, st.q.hiz() << 1);
                    st.q = shifted.insert(0, sin);
                }
                outs[0] = st.q;
                outs[1] = st.q.slice(length - 1, 1);
            }
            Prim::Rom => {
                let w = outs[0].width();
                outs[0] = read_mem(&st.mem, ins[0], w);
            }
            Prim::Ram { edge } => {
                let w = outs[0].width();
                let fired = clock_edge(st, ins[3], *edge);
                if fired
                    && ins[2].bit(0) == Bit::One
                    && let Some(a) = ins[0].to_u64()
                    && let Some(slot) = st.mem.get_mut(a as usize)
                {
                    *slot = ins[1].resize(w).val();
                }
                outs[0] = read_mem(&st.mem, ins[0], w);
            }
            Prim::Keyboard { edge } => {
                let fired = clock_edge(st, ins[1], *edge);
                if low_if_z(ins[2]) == Bit::One {
                    st.mem[0] = 0;
                } else if fired && high_if_z(ins[0]) == Bit::One && st.mem[0] > 0 {
                    // Consume the oldest character.
                    let n = st.mem[0] as usize;
                    st.mem.copy_within(2..=n, 1);
                    st.mem[0] -= 1;
                }
                let n = st.mem[0];
                outs[0] = Signal::new(7, if n > 0 { st.mem[1] } else { 0 });
                outs[1] = Signal::bool(n > 0);
            }
            Prim::Tty { cols, rows, edge } => {
                let fired = clock_edge(st, ins[2], *edge);
                if low_if_z(ins[3]) == Bit::One {
                    st.mem.fill(0);
                } else if fired
                    && ins[1].bit(0) == Bit::One
                    && let Some(ch) = ins[0].to_u64()
                {
                    tty_put(&mut st.mem, *cols as usize, *rows as usize, ch as u8);
                }
            }
            Prim::Screen { w, h, colors, edge } => {
                let fired = clock_edge(st, ins[4], *edge);
                if low_if_z(ins[5]) == Bit::One {
                    st.mem.fill(0);
                } else if fired
                    && ins[3].bit(0) == Bit::One
                    && let (Some(x), Some(y)) = (ins[0].to_u64(), ins[1].to_u64())
                    && x < *w as u64
                    && y < *h as u64
                {
                    // An unknown colour is drawn black.
                    let color = ins[2].resize(colors.bits()).to_u64().unwrap_or(0);
                    st.mem[(y * *w as u64 + x) as usize] = color;
                }
            }
        }
    }
}

/// Adds a typed character to the buffer of a keyboard (count then
/// characters in `mem`); returns false when the buffer is full.
pub fn keyboard_push(st: &mut State, ch: u8) -> bool {
    let n = st.mem[0] as usize;
    if n + 1 >= st.mem.len() {
        return false;
    }
    st.mem[n + 1] = (ch & 0x7f) as u64;
    st.mem[0] += 1;
    true
}

/// Characters waiting in the buffer of a keyboard, oldest first (none for
/// an empty `mem`, e.g. a component drawn without a simulation).
pub fn keyboard_buffer(mem: &[u64]) -> &[u64] {
    let Some((&n, keys)) = mem.split_first() else { return &[] };
    &keys[..(n as usize).min(keys.len())]
}

/// Writes character `ch` at the cursor of a terminal (characters then
/// cursor in `mem`). Printable ASCII is written and advances the cursor;
/// line feed and carriage return start a new line, backspace erases the
/// previous character, form feed clears the screen. Text scrolls up when
/// the cursor passes the last row.
fn tty_put(mem: &mut [u64], cols: usize, rows: usize, ch: u8) {
    let n = cols * rows;
    let mut cursor = (mem[n] as usize).min(n - 1);
    match ch {
        b'\n' | b'\r' => cursor = (cursor / cols + 1) * cols,
        8 | 127 => {
            if cursor > 0 {
                cursor -= 1;
                mem[cursor] = 0;
            }
        }
        12 => {
            mem.fill(0);
            return;
        }
        32..=126 => {
            mem[cursor] = ch as u64;
            cursor += 1;
        }
        _ => {}
    }
    if cursor >= n {
        mem.copy_within(cols..n, 0);
        mem[n - cols..n].fill(0);
        cursor -= cols;
    }
    mem[n] = cursor as u64;
}

fn read_mem(mem: &[u64], addr: Signal, w: u8) -> Signal {
    match addr.to_u64().and_then(|a| mem.get(a as usize)) {
        Some(&v) => Signal::new(w, v),
        None => Signal::x(w),
    }
}

/// Bit 0 of a control input, with a floating (unconnected) input read as 0.
fn low_if_z(s: Signal) -> Bit {
    match s.bit(0) {
        Bit::Z => Bit::Zero,
        b => b,
    }
}

/// Bit 0 of an enable input, with a floating (unconnected) input read as 1.
fn high_if_z(s: Signal) -> Bit {
    match s.bit(0) {
        Bit::Z => Bit::One,
        b => b,
    }
}

/// Records the clock value and reports whether the active edge occurred.
fn clock_edge(st: &mut State, clk: Signal, edge: Edge) -> bool {
    let now = clk.bit(0);
    let prev = std::mem::replace(&mut st.last_clk, now);
    match edge {
        Edge::Rising => prev == Bit::Zero && now == Bit::One,
        Edge::Falling => prev == Bit::One && now == Bit::Zero,
    }
}

fn shift(v: Signal, n: u64, mode: ShiftMode) -> Signal {
    let w = v.width();
    let n = n.min(64) as u32;
    let parts = [v.val(), v.unk(), v.hiz()];
    let shl = |x: u64| x.checked_shl(n).unwrap_or(0);
    let shr = |x: u64| x.checked_shr(n).unwrap_or(0);
    let r = |f: &dyn Fn(u64) -> u64| {
        let [a, b, c] = parts.map(f);
        Signal::from_parts(w, a, b, c)
    };
    match mode {
        ShiftMode::Lsl => r(&shl),
        ShiftMode::Lsr => r(&shr),
        ShiftMode::Asr => {
            if n >= w as u32 {
                return v.slice(w - 1, 1).sign_extend(w);
            }
            v.slice(n as u8, w - n as u8).sign_extend(w)
        }
        ShiftMode::RotLeft | ShiftMode::RotRight => {
            let k = n % w as u32;
            let k = if mode == ShiftMode::RotLeft { k } else { (w as u32 - k) % w as u32 };
            if k == 0 {
                return v;
            }
            let m = mask(w);
            r(&|x: u64| ((x << k) | (x >> (w as u32 - k))) & m)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Props;

    fn prim(kind: Kind, props: Props) -> (Prim, State) {
        let c = Component { id: 1, kind, pos: (0, 0), rot: 0, mirror: false, props };
        Prim::from_component(&c).unwrap()
    }

    fn run(kind: Kind, props: Props, ins: &[Signal], out_widths: &[u8]) -> Vec<Signal> {
        let (p, mut st) = prim(kind, props);
        let mut outs: Vec<Signal> = out_widths.iter().map(|&w| Signal::zero(w)).collect();
        p.eval(&mut st, ins, &mut outs);
        outs
    }

    fn b(v: bool) -> Signal {
        Signal::bool(v)
    }

    #[test]
    fn gate_truth_tables() {
        let table = [
            (Kind::And, [false, false, false, true]),
            (Kind::Or, [false, true, true, true]),
            (Kind::Nand, [true, true, true, false]),
            (Kind::Nor, [true, false, false, false]),
            (Kind::Xor, [false, true, true, false]),
            (Kind::Xnor, [true, false, false, true]),
        ];
        for (kind, expect) in table {
            for (i, e) in expect.into_iter().enumerate() {
                let (x, y) = (i & 2 != 0, i & 1 != 0);
                let out = run(kind, kind.default_props(), &[b(x), b(y)], &[1]);
                assert_eq!(out[0], b(e), "{kind:?}({x}, {y})");
            }
        }
    }

    #[test]
    fn multi_input_gates() {
        for n in 2..=8u8 {
            let props = Props { inputs: n, ..Props::default() };
            for v in 0..(1u32 << n) {
                let ins: Vec<Signal> = (0..n).map(|i| b(v >> i & 1 == 1)).collect();
                let all = v == (1 << n) - 1;
                let any = v != 0;
                let odd = v.count_ones() % 2 == 1;
                assert_eq!(run(Kind::And, props.clone(), &ins, &[1])[0], b(all));
                assert_eq!(run(Kind::Nand, props.clone(), &ins, &[1])[0], b(!all));
                assert_eq!(run(Kind::Or, props.clone(), &ins, &[1])[0], b(any));
                assert_eq!(run(Kind::Nor, props.clone(), &ins, &[1])[0], b(!any));
                assert_eq!(run(Kind::Xor, props.clone(), &ins, &[1])[0], b(odd));
                assert_eq!(run(Kind::Xnor, props.clone(), &ins, &[1])[0], b(!odd));
            }
        }
    }

    #[test]
    fn bus_gates_and_unknowns() {
        let p = Props { width: 4, ..Props::default() };
        let out = run(Kind::And, p.clone(), &[Signal::new(4, 0b1100), Signal::new(4, 0b1010)], &[4]);
        assert_eq!(out[0], Signal::new(4, 0b1000));
        let out = run(Kind::And, p.clone(), &[Signal::z(4), Signal::new(4, 0b0101)], &[4]);
        assert_eq!(out[0].to_bin_string(), "0x0x");
        let out = run(Kind::Not, p, &[Signal::x(4)], &[4]);
        assert_eq!(out[0], Signal::x(4));
    }

    #[test]
    fn not_buffer_tristate() {
        assert_eq!(run(Kind::Not, Props::default(), &[b(true)], &[1])[0], b(false));
        assert_eq!(run(Kind::Buffer, Props::default(), &[b(true)], &[1])[0], b(true));
        assert_eq!(run(Kind::Buffer, Props::default(), &[Signal::z(1)], &[1])[0], Signal::x(1));
        let t = |d, en| run(Kind::TriBuffer, Props::default(), &[d, en], &[1])[0];
        assert_eq!(t(b(true), b(true)), b(true));
        assert_eq!(t(b(false), b(true)), b(false));
        assert_eq!(t(b(true), b(false)), Signal::z(1));
        assert_eq!(t(b(true), Signal::x(1)), Signal::x(1));
    }

    #[test]
    fn mux_demux_decoder_encoder() {
        let p = Props { select: 2, width: 8, ..Props::default() };
        let data: Vec<Signal> = (0..4).map(|i| Signal::new(8, 10 + i)).collect();
        for s in 0..4 {
            let mut ins = data.clone();
            ins.push(Signal::new(2, s));
            assert_eq!(run(Kind::Mux, p.clone(), &ins, &[8])[0], Signal::new(8, 10 + s));
            let outs = run(Kind::Demux, p.clone(), &[Signal::new(8, 7), Signal::new(2, s)], &[8; 4]);
            for (i, o) in outs.iter().enumerate() {
                assert_eq!(*o, Signal::new(8, if i as u64 == s { 7 } else { 0 }));
            }
            let outs = run(Kind::Decoder, p.clone(), &[Signal::new(2, s)], &[1; 4]);
            for (i, o) in outs.iter().enumerate() {
                assert_eq!(*o, b(i as u64 == s));
            }
        }
        let mut ins = data.clone();
        ins.push(Signal::x(2));
        assert_eq!(run(Kind::Mux, p.clone(), &ins, &[8])[0], Signal::x(8));

        let enc = |bits: [bool; 4]| {
            let ins: Vec<Signal> = bits.iter().map(|&x| b(x)).collect();
            run(Kind::PriorityEncoder, p.clone(), &ins, &[2, 1])
        };
        assert_eq!(enc([true, false, true, false]), vec![Signal::new(2, 2), b(true)]);
        assert_eq!(enc([true, false, false, false]), vec![Signal::new(2, 0), b(true)]);
        assert_eq!(enc([false; 4]), vec![Signal::new(2, 0), b(false)]);
    }

    #[test]
    fn arithmetic() {
        let p = Props { width: 8, ..Props::default() };
        let add = |a, c, cin| run(Kind::Adder, p.clone(), &[Signal::new(8, a), Signal::new(8, c), cin], &[8, 1]);
        assert_eq!(add(100, 27, Signal::z(1)), vec![Signal::new(8, 127), b(false)]);
        assert_eq!(add(200, 100, b(true)), vec![Signal::new(8, 45), b(true)]);
        assert_eq!(add(255, 0, b(true)), vec![Signal::new(8, 0), b(true)]);
        let sub = |a, c, bin| run(Kind::Subtractor, p.clone(), &[Signal::new(8, a), Signal::new(8, c), bin], &[8, 1]);
        assert_eq!(sub(10, 3, b(false)), vec![Signal::new(8, 7), b(false)]);
        assert_eq!(sub(3, 10, b(false)), vec![Signal::new(8, 249), b(true)]);
        assert_eq!(sub(3, 3, b(true)), vec![Signal::new(8, 255), b(true)]);

        let cmp = |a, c, signed| {
            let p = Props { width: 8, signed, ..Props::default() };
            run(Kind::Comparator, p, &[Signal::new(8, a), Signal::new(8, c)], &[1, 1, 1])
        };
        assert_eq!(cmp(3, 5, false), vec![b(true), b(false), b(false)]);
        assert_eq!(cmp(5, 5, false), vec![b(false), b(true), b(false)]);
        assert_eq!(cmp(0xff, 1, false), vec![b(false), b(false), b(true)]);
        assert_eq!(cmp(0xff, 1, true), vec![b(true), b(false), b(false)]);
    }

    #[test]
    fn shifter_modes() {
        let v = Signal::new(8, 0b1001_0110);
        assert_eq!(shift(v, 2, ShiftMode::Lsl), Signal::new(8, 0b0101_1000));
        assert_eq!(shift(v, 2, ShiftMode::Lsr), Signal::new(8, 0b0010_0101));
        assert_eq!(shift(v, 2, ShiftMode::Asr), Signal::new(8, 0b1110_0101));
        assert_eq!(shift(v, 3, ShiftMode::RotLeft), Signal::new(8, 0b1011_0100));
        assert_eq!(shift(v, 3, ShiftMode::RotRight), Signal::new(8, 0b1101_0010));
        assert_eq!(shift(v, 9, ShiftMode::Lsl), Signal::new(8, 0));
        assert_eq!(shift(v, 9, ShiftMode::Asr), Signal::new(8, 0xff));
        assert_eq!(shift(Signal::new(64, 1), 64, ShiftMode::Lsl), Signal::new(64, 0));
    }

    #[test]
    fn split_merge_extend() {
        let p = Props { width: 8, pieces: vec![4, 4], ..Props::default() };
        let outs = run(Kind::Splitter, p.clone(), &[Signal::new(8, 0xa5)], &[4, 4]);
        assert_eq!(outs, vec![Signal::new(4, 5), Signal::new(4, 0xa)]);
        let out = run(Kind::Merger, p, &[Signal::new(4, 5), Signal::new(4, 0xa)], &[8]);
        assert_eq!(out[0], Signal::new(8, 0xa5));
        let p = Props { width: 4, out_width: 8, signed: true, ..Props::default() };
        assert_eq!(run(Kind::Extend, p, &[Signal::new(4, 0b1010)], &[8])[0], Signal::new(8, 0xfa));
    }

    #[test]
    fn d_flip_flop_edges_and_async() {
        let (p, mut st) = prim(Kind::DFlipFlop, Props::default());
        let mut outs = [Signal::zero(1), Signal::zero(1)];
        let z = Signal::z(1);
        let mut step = |d, clk, pre, rst| {
            p.eval(&mut st, &[b(d), b(clk), pre, rst], &mut outs);
            outs[0]
        };
        assert_eq!(step(true, false, z, z), b(false));
        assert_eq!(step(true, true, z, z), b(true), "rising edge captures d");
        assert_eq!(step(false, true, z, z), b(true), "no edge, hold");
        assert_eq!(step(false, false, z, z), b(true), "falling edge ignored");
        assert_eq!(step(false, true, z, z), b(false));
        assert_eq!(step(true, true, b(true), z), b(true), "async preset");
        assert_eq!(step(true, true, b(true), b(true)), b(false), "reset wins");

        let props = Props { edge: Edge::Falling, ..Props::default() };
        let (p, mut st) = prim(Kind::DFlipFlop, props);
        let mut outs = [Signal::zero(1), Signal::zero(1)];
        p.eval(&mut st, &[b(true), b(true), z, z], &mut outs);
        assert_eq!(outs[0], b(false));
        p.eval(&mut st, &[b(true), b(false), z, z], &mut outs);
        assert_eq!(outs, [b(true), b(false)]);
    }

    #[test]
    fn t_and_jk_flip_flops() {
        let z = Signal::z(1);
        let (p, mut st) = prim(Kind::TFlipFlop, Props::default());
        let mut outs = [Signal::zero(1); 2];
        let mut q = Vec::new();
        for _ in 0..4 {
            p.eval(&mut st, &[b(true), b(false), z, z], &mut outs);
            p.eval(&mut st, &[b(true), b(true), z, z], &mut outs);
            q.push(outs[0].is_one());
        }
        assert_eq!(q, [true, false, true, false]);

        let (p, mut st) = prim(Kind::JkFlipFlop, Props::default());
        let mut outs = [Signal::zero(1); 2];
        let mut pulse = |j, k| {
            p.eval(&mut st, &[b(j), b(k), b(false), z, z], &mut outs);
            p.eval(&mut st, &[b(j), b(k), b(true), z, z], &mut outs);
            outs[0].is_one()
        };
        assert!(pulse(true, false));
        assert!(pulse(false, false));
        assert!(!pulse(false, true));
        assert!(pulse(true, true));
        assert!(!pulse(true, true));
    }

    #[test]
    fn latches() {
        let (p, mut st) = prim(Kind::SrLatch, Props::default());
        let mut outs = [Signal::zero(1); 2];
        p.eval(&mut st, &[b(true), b(false)], &mut outs);
        assert_eq!(outs, [b(true), b(false)]);
        p.eval(&mut st, &[b(false), b(false)], &mut outs);
        assert_eq!(outs, [b(true), b(false)], "hold");
        p.eval(&mut st, &[b(false), b(true)], &mut outs);
        assert_eq!(outs, [b(false), b(true)]);

        let (p, mut st) = prim(Kind::DLatch, Props::default());
        let mut outs = [Signal::zero(1); 2];
        p.eval(&mut st, &[b(true), b(true)], &mut outs);
        assert_eq!(outs[0], b(true), "transparent");
        p.eval(&mut st, &[b(false), b(false)], &mut outs);
        assert_eq!(outs[0], b(true), "latched");
    }

    #[test]
    fn register_counter_shift_register() {
        let z = Signal::z(1);
        let p = Props { width: 4, ..Props::default() };
        let (reg, mut st) = prim(Kind::Register, p.clone());
        let mut outs = [Signal::zero(4)];
        reg.eval(&mut st, &[Signal::new(4, 9), z, b(false), z], &mut outs);
        reg.eval(&mut st, &[Signal::new(4, 9), z, b(true), z], &mut outs);
        assert_eq!(outs[0], Signal::new(4, 9));
        reg.eval(&mut st, &[Signal::new(4, 3), b(false), b(false), z], &mut outs);
        reg.eval(&mut st, &[Signal::new(4, 3), b(false), b(true), z], &mut outs);
        assert_eq!(outs[0], Signal::new(4, 9), "disabled");

        let (cnt, mut st) = prim(Kind::Counter, p);
        let mut outs = [Signal::zero(4), Signal::zero(1)];
        let mut vals = Vec::new();
        for _ in 0..17 {
            cnt.eval(&mut st, &[z, z, z, b(false), z], &mut outs);
            cnt.eval(&mut st, &[z, z, z, b(true), z], &mut outs);
            vals.push(outs[0].to_u64().unwrap());
        }
        assert_eq!(vals[..3], [1, 2, 3]);
        assert_eq!(vals[14], 15);
        assert_eq!(vals[15], 0, "wraps");
        cnt.eval(&mut st, &[Signal::new(4, 15), b(true), z, b(false), z], &mut outs);
        cnt.eval(&mut st, &[Signal::new(4, 15), b(true), z, b(true), z], &mut outs);
        assert_eq!(outs, [Signal::new(4, 15), b(true)], "load and carry");

        let (sr, mut st) = prim(Kind::ShiftRegister, Props { length: 4, ..Props::default() });
        let mut outs = [Signal::zero(4), Signal::zero(1)];
        for bit in [true, false, true, true] {
            sr.eval(&mut st, &[b(bit), z, b(false), z], &mut outs);
            sr.eval(&mut st, &[b(bit), z, b(true), z], &mut outs);
        }
        assert_eq!(outs, [Signal::new(4, 0b1011), b(true)]);
    }

    /// Text of a terminal's rows, `_` for empty cells.
    fn screen_text(st: &State, cols: usize) -> Vec<String> {
        let n = st.mem.len() - 1;
        st.mem[..n]
            .chunks(cols)
            .map(|r| r.iter().map(|&c| if c == 0 { '_' } else { c as u8 as char }).collect())
            .collect()
    }

    #[test]
    fn terminal_writes_wraps_and_scrolls() {
        let props = Props { size: (4, 2), ..Props::default() };
        let (tty, mut st) = prim(Kind::Tty, props);
        let z = Signal::z(1);
        let put = |st: &mut State, ch: u8| {
            let data = Signal::new(7, ch as u64);
            tty.eval(st, &[data, b(true), b(false), z], &mut []);
            tty.eval(st, &[data, b(true), b(true), z], &mut []);
        };
        for &c in b"AB\nCDEFG" {
            put(&mut st, c);
        }
        // "AB", new line, "CDEF" fills the second row, "G" scrolls.
        assert_eq!(screen_text(&st, 4), ["CDEF", "G___"]);
        put(&mut st, 8);
        assert_eq!(screen_text(&st, 4), ["CDEF", "____"], "backspace");
        put(&mut st, 7);
        assert_eq!(screen_text(&st, 4), ["CDEF", "____"], "bell ignored");
        // Write disabled, then clear.
        let data = Signal::new(7, b'X' as u64);
        tty.eval(&mut st, &[data, b(false), b(false), z], &mut []);
        tty.eval(&mut st, &[data, b(false), b(true), z], &mut []);
        assert_eq!(screen_text(&st, 4), ["CDEF", "____"]);
        tty.eval(&mut st, &[data, b(false), b(true), b(true)], &mut []);
        assert!(st.mem.iter().all(|&c| c == 0));
    }

    #[test]
    fn screen_draws_pixels() {
        let props = Props { size: (4, 4), colors: ColorMode::Rgb332, ..Props::default() };
        let (scr, mut st) = prim(Kind::Screen, props);
        let z = Signal::z(1);
        let px = |x: u64, y: u64, c: u64, clk: bool| {
            [Signal::new(2, x), Signal::new(2, y), Signal::new(8, c), b(true), b(clk), z]
        };
        scr.eval(&mut st, &px(1, 2, 0xe0, false), &mut []);
        scr.eval(&mut st, &px(1, 2, 0xe0, true), &mut []);
        assert_eq!(st.mem[2 * 4 + 1], 0xe0);
        assert_eq!(ColorMode::Rgb332.rgb(0xe0), (255, 0, 0));
        assert_eq!(st.mem.iter().filter(|&&p| p != 0).count(), 1);
        let mut clear = px(0, 0, 0, true);
        clear[5] = b(true);
        scr.eval(&mut st, &clear, &mut []);
        assert!(st.mem.iter().all(|&p| p == 0));
    }

    #[test]
    fn keyboard_buffer_of_any_memory() {
        assert!(keyboard_buffer(&[]).is_empty(), "no simulation");
        assert!(keyboard_buffer(&[0]).is_empty());
        assert_eq!(keyboard_buffer(&[2, 65, 66, 0]), [65, 66]);
        assert_eq!(keyboard_buffer(&[9, 65]), [65], "count beyond the buffer");
    }

    #[test]
    fn rom_and_ram() {
        let p = Props { addr_width: 2, width: 8, data: "11 22 33 44".into(), ..Props::default() };
        for a in 0..4 {
            let out = run(Kind::Rom, p.clone(), &[Signal::new(2, a)], &[8]);
            assert_eq!(out[0], Signal::new(8, 0x11 * (a + 1)));
        }
        let (ram, mut st) = prim(Kind::Ram, p);
        let mut outs = [Signal::zero(8)];
        let a = Signal::new(2, 3);
        ram.eval(&mut st, &[a, Signal::new(8, 0x99), b(true), b(false)], &mut outs);
        assert_eq!(outs[0], Signal::new(8, 0x44));
        ram.eval(&mut st, &[a, Signal::new(8, 0x99), b(true), b(true)], &mut outs);
        assert_eq!(outs[0], Signal::new(8, 0x99));
        ram.eval(&mut st, &[a, Signal::new(8, 0x55), b(false), b(false)], &mut outs);
        ram.eval(&mut st, &[a, Signal::new(8, 0x55), b(false), b(true)], &mut outs);
        assert_eq!(outs[0], Signal::new(8, 0x99), "write disabled");
    }
}
