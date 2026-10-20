//! Component kinds, their properties, pin geometry ([`layout`]) and
//! simulation behaviour ([`eval`]).

pub mod eval;
pub mod layout;

use serde::{Deserialize, Serialize};

use crate::value::Radix;

pub use layout::{Layout, PinDef, PinDir, Transform};

/// Every component type known to the editor and the simulator.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, PartialOrd, Ord)]
pub enum Kind {
    // Inputs / outputs
    Input,
    Button,
    Constant,
    Clock,
    Output,
    Led,
    Probe,
    /// Joins every tunnel with the same label in a circuit into one net.
    Tunnel,
    Keyboard,
    // Displays
    NumDisplay,
    SevenSeg,
    LedMatrix,
    Tty,
    Screen,
    /// Single hex digit of older files, converted to a [`Kind::NumDisplay`]
    /// on loading (see [`crate::circuit::Circuit::normalize`]).
    #[doc(hidden)]
    HexDisplay,
    // Gates
    And,
    Or,
    Nand,
    Nor,
    Xor,
    Xnor,
    Not,
    Buffer,
    TriBuffer,
    // Combinational
    Mux,
    Demux,
    Decoder,
    PriorityEncoder,
    Adder,
    Subtractor,
    Comparator,
    Shifter,
    Splitter,
    Merger,
    Extend,
    // Sequential
    SrLatch,
    DLatch,
    DFlipFlop,
    TFlipFlop,
    JkFlipFlop,
    Register,
    Counter,
    ShiftRegister,
    Rom,
    Ram,
    // Hierarchy
    Subcircuit,
    // Annotations
    Text,
    Rect,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Category {
    Io,
    Displays,
    Gates,
    Combinational,
    Sequential,
    Memory,
    Annotations,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::Io,
        Category::Displays,
        Category::Gates,
        Category::Combinational,
        Category::Sequential,
        Category::Memory,
        Category::Annotations,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Category::Io => "Inputs & outputs",
            Category::Displays => "Displays",
            Category::Gates => "Gates",
            Category::Combinational => "Combinational",
            Category::Sequential => "Sequential",
            Category::Memory => "Memory",
            Category::Annotations => "Text & annotations",
        }
    }
}

/// Property editable in the properties panel.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PropKey {
    Label,
    Width,
    Inputs,
    Select,
    Value,
    Period,
    Duty,
    Edge,
    Signed,
    OutWidth,
    Shift,
    Pieces,
    AddrWidth,
    Length,
    Data,
    DataFile,
    Radix,
    Size,
    Delay,
    Digits,
    Colors,
    Buffer,
}

impl PropKey {
    pub fn name(self) -> &'static str {
        match self {
            PropKey::Label => "Label",
            PropKey::Width => "Width (bits)",
            PropKey::Inputs => "Inputs",
            PropKey::Select => "Select bits",
            PropKey::Value => "Value",
            PropKey::Period => "Period (ticks)",
            PropKey::Duty => "Duty cycle (%)",
            PropKey::Edge => "Active edge",
            PropKey::Signed => "Signed",
            PropKey::OutWidth => "Output width",
            PropKey::Shift => "Shift type",
            PropKey::Pieces => "Split (bits)",
            PropKey::AddrWidth => "Address bits",
            PropKey::Length => "Stages",
            PropKey::Data => "Contents (hex)",
            PropKey::DataFile => "External .hex file",
            PropKey::Radix => "Radix",
            PropKey::Size => "Size",
            PropKey::Delay => "Delay (ticks)",
            PropKey::Digits => "Digits",
            PropKey::Colors => "Colors",
            PropKey::Buffer => "Buffer size",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum Edge {
    #[default]
    Rising,
    Falling,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum ShiftMode {
    /// Logical shift left.
    #[default]
    Lsl,
    /// Logical shift right.
    Lsr,
    /// Arithmetic shift right.
    Asr,
    RotLeft,
    RotRight,
}

impl ShiftMode {
    pub const ALL: [ShiftMode; 5] =
        [ShiftMode::Lsl, ShiftMode::Lsr, ShiftMode::Asr, ShiftMode::RotLeft, ShiftMode::RotRight];

    pub fn name(self) -> &'static str {
        match self {
            ShiftMode::Lsl => "Logical left",
            ShiftMode::Lsr => "Logical right",
            ShiftMode::Asr => "Arithmetic right",
            ShiftMode::RotLeft => "Rotate left",
            ShiftMode::RotRight => "Rotate right",
        }
    }
}

/// Colours of a pixel screen, and the width of its colour input.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum ColorMode {
    /// One bit: off or on.
    #[default]
    Mono,
    /// Eight bits, `RRRGGGBB`.
    Rgb332,
    /// 24 bits, `RRGGBB` in hex.
    Rgb888,
}

impl ColorMode {
    pub const ALL: [ColorMode; 3] = [ColorMode::Mono, ColorMode::Rgb332, ColorMode::Rgb888];

    pub fn name(self) -> &'static str {
        match self {
            ColorMode::Mono => "Mono, 1 bit",
            ColorMode::Rgb332 => "RGB 332, 8 bits",
            ColorMode::Rgb888 => "RGB, 24 bits",
        }
    }

    pub fn bits(self) -> u8 {
        match self {
            ColorMode::Mono => 1,
            ColorMode::Rgb332 => 8,
            ColorMode::Rgb888 => 24,
        }
    }

    /// The 8-bit red, green and blue of a stored pixel value.
    pub fn rgb(self, v: u64) -> (u8, u8, u8) {
        let scale = |x: u64, bits: u32| (x * 255 / ((1 << bits) - 1)) as u8;
        match self {
            ColorMode::Mono => {
                if v & 1 == 1 {
                    (255, 255, 255)
                } else {
                    (0, 0, 0)
                }
            }
            ColorMode::Rgb332 => (scale(v >> 5 & 7, 3), scale(v >> 2 & 7, 3), scale(v & 3, 2)),
            ColorMode::Rgb888 => ((v >> 16) as u8, (v >> 8) as u8, v as u8),
        }
    }
}

/// Bits needed to address `n` positions (at least 1).
pub fn index_width(n: i32) -> u8 {
    let mut bits = 1;
    while (1i64 << bits) < n as i64 {
        bits += 1;
    }
    bits
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// Component properties. A zero/empty field means "the default for this
/// kind"; the accessor methods apply those defaults and clamp to valid
/// ranges. Default fields are omitted when saving to keep files small.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Props {
    #[serde(skip_serializing_if = "is_default")]
    pub width: u8,
    #[serde(skip_serializing_if = "is_default")]
    pub inputs: u8,
    #[serde(skip_serializing_if = "is_default")]
    pub select: u8,
    #[serde(skip_serializing_if = "is_default")]
    pub label: String,
    #[serde(skip_serializing_if = "is_default")]
    pub value: u64,
    #[serde(skip_serializing_if = "is_default")]
    pub period: u32,
    #[serde(skip_serializing_if = "is_default")]
    pub duty: u8,
    #[serde(skip_serializing_if = "is_default")]
    pub edge: Edge,
    #[serde(skip_serializing_if = "is_default")]
    pub signed: bool,
    #[serde(skip_serializing_if = "is_default")]
    pub out_width: u8,
    #[serde(skip_serializing_if = "is_default")]
    pub shift: ShiftMode,
    #[serde(skip_serializing_if = "is_default")]
    pub pieces: Vec<u8>,
    #[serde(skip_serializing_if = "is_default")]
    pub addr_width: u8,
    #[serde(skip_serializing_if = "is_default")]
    pub length: u8,
    #[serde(skip_serializing_if = "is_default")]
    pub data: String,
    #[serde(skip_serializing_if = "is_default")]
    pub data_file: String,
    #[serde(skip_serializing_if = "is_default")]
    pub circuit: String,
    #[serde(skip_serializing_if = "is_default")]
    pub radix: Radix,
    #[serde(skip_serializing_if = "is_default")]
    pub size: (u16, u16),
    #[serde(skip_serializing_if = "is_default")]
    pub delay: u32,
    /// Digits of a number or 7-segment display; 0 = as many as the width
    /// needs (number display) or 1 (7-segment display).
    #[serde(skip_serializing_if = "is_default")]
    pub digits: u8,
    #[serde(skip_serializing_if = "is_default")]
    pub colors: ColorMode,
    /// Characters a keyboard holds; 0 = the default.
    #[serde(skip_serializing_if = "is_default")]
    pub buffer: u16,
}

pub const DEFAULT_CLOCK_PERIOD: u32 = 100;

impl Props {
    pub fn is_default(&self) -> bool {
        *self == Props::default()
    }

    pub fn width(&self) -> u8 {
        self.width.clamp(1, 64)
    }

    pub fn inputs(&self) -> u8 {
        if self.inputs == 0 { 2 } else { self.inputs.clamp(2, 8) }
    }

    pub fn select(&self) -> u8 {
        self.select.clamp(1, 5)
    }

    pub fn period(&self) -> u32 {
        if self.period == 0 { DEFAULT_CLOCK_PERIOD } else { self.period.max(2) }
    }

    pub fn duty(&self) -> u8 {
        if self.duty == 0 { 50 } else { self.duty.clamp(1, 99) }
    }

    /// Ticks the clock stays high and low in each period (both at least 1).
    pub fn clock_phases(&self) -> (u32, u32) {
        let p = self.period();
        let high = ((p as u64 * self.duty() as u64) / 100).clamp(1, p as u64 - 1) as u32;
        (high, p - high)
    }

    pub fn out_width(&self) -> u8 {
        if self.out_width == 0 { 16 } else { self.out_width.clamp(1, 64) }
    }

    pub fn addr_width(&self) -> u8 {
        if self.addr_width == 0 { 8 } else { self.addr_width.clamp(1, 20) }
    }

    pub fn length(&self) -> u8 {
        if self.length == 0 { 4 } else { self.length.clamp(1, 64) }
    }

    /// Splitter/merger piece widths, LSB first. Defaults to one bit each.
    pub fn pieces(&self) -> Vec<u8> {
        let w = self.width();
        if self.pieces.is_empty() || self.pieces.contains(&0) {
            return vec![1; w as usize];
        }
        let mut out = Vec::new();
        let mut used = 0u8;
        for &p in &self.pieces {
            if used >= w {
                break;
            }
            let p = p.min(w - used);
            out.push(p);
            used += p;
        }
        if used < w {
            out.push(w - used);
        }
        out
    }

    pub fn rect_size(&self) -> (i32, i32) {
        self.size_of(Kind::Rect)
    }

    /// The Size property of a `kind` (box, LED matrix, terminal, screen),
    /// with its default and limits applied.
    pub fn size_of(&self, kind: Kind) -> (i32, i32) {
        let (def, lo, hi) = kind.size_limits();
        let pick = |v: u16, d: i32, lo: i32, hi: i32| if v == 0 { d } else { (v as i32).clamp(lo, hi) };
        (pick(self.size.0, def.0, lo.0, hi.0), pick(self.size.1, def.1, lo.1, hi.1))
    }

    /// Digits shown by a number display of `width` bits in `radix`.
    pub fn num_digits(&self) -> u8 {
        if self.digits != 0 {
            return self.digits.clamp(1, MAX_DIGITS);
        }
        let w = self.width() as u32;
        let digits = match self.radix {
            Radix::Bin => w,
            Radix::Hex => w.div_ceil(4),
            Radix::Dec => decimal_digits(u64::MAX >> (64 - w)),
            Radix::Signed => 1 + decimal_digits(1u64 << (w - 1)),
        };
        (digits as u8).clamp(1, MAX_DIGITS)
    }

    /// Characters a keyboard can hold.
    pub fn key_buffer(&self) -> u16 {
        if self.buffer == 0 { 32 } else { self.buffer.clamp(1, MAX_KEY_BUFFER) }
    }

    /// Digits of a 7-segment display.
    pub fn seg_digits(&self) -> u8 {
        self.digits.clamp(1, 8)
    }
}

/// Largest keyboard buffer, in characters.
pub const MAX_KEY_BUFFER: u16 = 4096;

/// Most digits of a number display.
pub const MAX_DIGITS: u8 = 32;

fn decimal_digits(v: u64) -> u32 {
    v.checked_ilog10().unwrap_or(0) + 1
}

impl Kind {
    pub const ALL: [Kind; 47] = [
        Kind::Input,
        Kind::Button,
        Kind::Constant,
        Kind::Clock,
        Kind::Output,
        Kind::Led,
        Kind::Probe,
        Kind::Tunnel,
        Kind::Keyboard,
        Kind::NumDisplay,
        Kind::SevenSeg,
        Kind::LedMatrix,
        Kind::Tty,
        Kind::Screen,
        Kind::And,
        Kind::Or,
        Kind::Nand,
        Kind::Nor,
        Kind::Xor,
        Kind::Xnor,
        Kind::Not,
        Kind::Buffer,
        Kind::TriBuffer,
        Kind::Mux,
        Kind::Demux,
        Kind::Decoder,
        Kind::PriorityEncoder,
        Kind::Adder,
        Kind::Subtractor,
        Kind::Comparator,
        Kind::Shifter,
        Kind::Splitter,
        Kind::Merger,
        Kind::Extend,
        Kind::SrLatch,
        Kind::DLatch,
        Kind::DFlipFlop,
        Kind::TFlipFlop,
        Kind::JkFlipFlop,
        Kind::Register,
        Kind::Counter,
        Kind::ShiftRegister,
        Kind::Rom,
        Kind::Ram,
        Kind::Subcircuit,
        Kind::Text,
        Kind::Rect,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Input => "Input / Switch",
            Kind::Button => "Button",
            Kind::Constant => "Constant",
            Kind::Clock => "Clock",
            Kind::Output => "Output / Indicator",
            Kind::Led => "LED",
            Kind::Probe => "Probe",
            Kind::Tunnel => "Tunnel",
            Kind::Keyboard => "Keyboard",
            Kind::NumDisplay | Kind::HexDisplay => "Number display",
            Kind::SevenSeg => "7-segment display",
            Kind::LedMatrix => "LED matrix",
            Kind::Tty => "Text terminal",
            Kind::Screen => "Pixel screen",
            Kind::And => "AND",
            Kind::Or => "OR",
            Kind::Nand => "NAND",
            Kind::Nor => "NOR",
            Kind::Xor => "XOR",
            Kind::Xnor => "XNOR",
            Kind::Not => "NOT",
            Kind::Buffer => "Buffer",
            Kind::TriBuffer => "Tri-state buffer",
            Kind::Mux => "Multiplexer",
            Kind::Demux => "Demultiplexer",
            Kind::Decoder => "Decoder",
            Kind::PriorityEncoder => "Priority encoder",
            Kind::Adder => "Adder",
            Kind::Subtractor => "Subtractor",
            Kind::Comparator => "Comparator",
            Kind::Shifter => "Shifter",
            Kind::Splitter => "Splitter",
            Kind::Merger => "Merger",
            Kind::Extend => "Bit extender",
            Kind::SrLatch => "SR latch",
            Kind::DLatch => "D latch",
            Kind::DFlipFlop => "D flip-flop",
            Kind::TFlipFlop => "T flip-flop",
            Kind::JkFlipFlop => "JK flip-flop",
            Kind::Register => "Register",
            Kind::Counter => "Counter",
            Kind::ShiftRegister => "Shift register",
            Kind::Rom => "ROM",
            Kind::Ram => "RAM",
            Kind::Subcircuit => "Subcircuit",
            Kind::Text => "Text",
            Kind::Rect => "Box",
        }
    }

    pub fn category(self) -> Category {
        use Kind::*;
        match self {
            Input | Button | Constant | Clock | Output | Led | Probe | Tunnel | Keyboard => Category::Io,
            NumDisplay | HexDisplay | SevenSeg | LedMatrix | Tty | Screen => Category::Displays,
            And | Or | Nand | Nor | Xor | Xnor | Not | Buffer | TriBuffer => Category::Gates,
            Mux | Demux | Decoder | PriorityEncoder | Adder | Subtractor | Comparator | Shifter | Splitter | Merger
            | Extend => Category::Combinational,
            SrLatch | DLatch | DFlipFlop | TFlipFlop | JkFlipFlop | Register | Counter | ShiftRegister => {
                Category::Sequential
            }
            Rom | Ram => Category::Memory,
            Subcircuit | Text | Rect => Category::Annotations,
        }
    }

    /// Annotations are drawn but take no part in simulation.
    pub fn is_annotation(self) -> bool {
        matches!(self, Kind::Text | Kind::Rect)
    }

    pub fn is_gate(self) -> bool {
        matches!(self, Kind::And | Kind::Or | Kind::Nand | Kind::Nor | Kind::Xor | Kind::Xnor)
    }

    /// Components the user can poke during simulation.
    pub fn is_interactive(self) -> bool {
        matches!(self, Kind::Input | Kind::Button | Kind::Keyboard)
    }

    /// Properties shown in the properties panel.
    pub fn prop_keys(self) -> &'static [PropKey] {
        use Kind::*;
        use PropKey as P;
        match self {
            Input => &[P::Label, P::Width, P::Value, P::Radix],
            Button => &[P::Label],
            Constant => &[P::Width, P::Value],
            Clock => &[P::Label, P::Period, P::Duty],
            Output => &[P::Label, P::Width, P::Radix],
            Led => &[P::Label],
            Probe => &[P::Label, P::Width, P::Radix],
            Tunnel => &[P::Label, P::Width],
            Keyboard => &[P::Label, P::Buffer, P::Edge],
            NumDisplay | HexDisplay => &[P::Label, P::Width, P::Radix, P::Digits],
            SevenSeg => &[P::Label, P::Digits],
            LedMatrix => &[P::Label, P::Size],
            Tty => &[P::Label, P::Size, P::Edge],
            Screen => &[P::Label, P::Size, P::Colors, P::Edge],
            And | Or | Nand | Nor | Xor | Xnor => &[P::Inputs, P::Width, P::Delay],
            Not | Buffer | TriBuffer => &[P::Width, P::Delay],
            Mux | Demux => &[P::Select, P::Width, P::Delay],
            Decoder | PriorityEncoder => &[P::Select, P::Delay],
            Adder | Subtractor => &[P::Width, P::Delay],
            Comparator => &[P::Width, P::Signed, P::Delay],
            Shifter => &[P::Width, P::Shift, P::Delay],
            Splitter | Merger => &[P::Width, P::Pieces],
            Extend => &[P::Width, P::OutWidth, P::Signed],
            SrLatch | DLatch => &[P::Label, P::Delay],
            DFlipFlop | TFlipFlop | JkFlipFlop => &[P::Label, P::Edge, P::Delay],
            Register => &[P::Label, P::Width, P::Edge, P::Delay],
            Counter => &[P::Label, P::Width, P::Edge, P::Delay],
            ShiftRegister => &[P::Label, P::Length, P::Edge, P::Delay],
            Rom => &[P::Label, P::AddrWidth, P::Width, P::Data, P::DataFile, P::Delay],
            Ram => &[P::Label, P::AddrWidth, P::Width, P::Edge, P::Data, P::DataFile, P::Delay],
            Subcircuit => &[P::Label],
            Text => &[P::Label],
            Rect => &[P::Label, P::Size],
        }
    }

    /// Size property of a kind: default, smallest and largest (width and
    /// height in grid units for a box, columns and rows for an LED matrix or
    /// a terminal, pixels for a screen).
    pub fn size_limits(self) -> ((i32, i32), (i32, i32), (i32, i32)) {
        match self {
            Kind::LedMatrix => ((8, 8), (1, 1), (64, 64)),
            Kind::Tty => ((32, 8), (4, 1), (120, 50)),
            Kind::Screen => ((32, 32), (2, 2), (128, 128)),
            _ => ((10, 6), (1, 1), (400, 400)),
        }
    }

    /// Properties for a freshly placed component.
    pub fn default_props(self) -> Props {
        use Kind::*;
        let mut p = Props::default();
        match self {
            Input | Output | Probe | Tunnel | Constant | Not | Buffer | TriBuffer => p.width = 1,
            And | Or | Nand | Nor | Xor | Xnor => p.inputs = 2,
            Mux | Demux | Decoder | PriorityEncoder => p.select = 1,
            Adder | Subtractor | Comparator | Shifter | Register | Counter => p.width = 8,
            Splitter | Merger => p.width = 4,
            Extend => {
                p.width = 8;
                p.out_width = 16;
                p.signed = true;
            }
            Rom | Ram => {
                p.addr_width = 4;
                p.width = 8;
            }
            ShiftRegister => p.length = 4,
            NumDisplay => {
                p.width = 8;
                p.radix = Radix::Dec;
            }
            Text => p.label = "Text".into(),
            _ => {}
        }
        if self == Kind::Input || self == Kind::Output {
            p.radix = Radix::Hex;
        }
        p
    }
}

/// Builds the canonical pin name `prefix` + `i`.
pub fn indexed(prefix: &str, i: usize) -> String {
    format!("{prefix}{i}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_kinds_listed_once() {
        let mut v = Kind::ALL.to_vec();
        v.sort();
        v.dedup();
        assert_eq!(v.len(), Kind::ALL.len());
    }

    #[test]
    fn clock_phases() {
        let mut p = Props::default();
        assert_eq!(p.clock_phases(), (50, 50));
        p.period = 10;
        p.duty = 30;
        assert_eq!(p.clock_phases(), (3, 7));
        p.period = 2;
        p.duty = 99;
        assert_eq!(p.clock_phases(), (1, 1));
    }

    #[test]
    fn pieces_cover_width() {
        let mut p = Props { width: 8, ..Props::default() };
        assert_eq!(p.pieces(), vec![1; 8]);
        p.pieces = vec![3, 3];
        assert_eq!(p.pieces(), vec![3, 3, 2]);
        p.pieces = vec![6, 6];
        assert_eq!(p.pieces(), vec![6, 2]);
    }
}
