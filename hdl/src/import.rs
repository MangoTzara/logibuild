//! Conversion of a Yosys netlist into LogiBuild circuits, one per module.
//!
//! Every cell becomes one or a few components: `$add` an adder, `$dff` a
//! register, `$mem_v2` a RAM or ROM, an instance of another module a
//! subcircuit. Cells read and drive vectors of bits; once all cells are
//! converted, each vector read is traced back to the output pins driving its
//! bits. A vector driven by one pin is a plain wire, a slice of a pin goes
//! through a splitter, and a vector gathered from several pins or constants
//! through a merger. Signals keep the widths of the source, so a 32-bit
//! adder stays one adder.
//!
//! Components are created without positions: [`crate::place`] lays them out.

use std::collections::{BTreeSet, HashMap, HashSet};

use lb_core::circuit::{Circuit, CompId, Component, HdlInfo, PinRef, Project, Wire};
use lb_core::component::layout::shift_amount_width;
use lb_core::component::{Edge, Kind, Props, ShiftMode};
use lb_core::io::format_hex;

use crate::json::{Bit, Cell, Design, Dir, Module};
use crate::place::place;

/// Widest signal a component can carry.
const MAX_WIDTH: usize = 64;
/// Largest memory, in address bits.
const MAX_ADDR_BITS: u64 = 20;
/// Inputs of the widest gate.
const MAX_GATE_INPUTS: usize = 8;
/// Select bits of the widest multiplexer.
const MAX_SELECT: usize = 5;
/// Readers from which a net is drawn with tunnels instead of wires.
const TUNNEL_FANOUT: usize = 6;

#[derive(Clone, PartialEq, Debug)]
pub enum ImportError {
    /// The design uses something LogiBuild cannot represent.
    Unsupported(String),
    /// A memory does not fit a RAM or ROM component; it can still be
    /// imported once Yosys maps it to flip-flops (`memory_map`).
    NeedsMemoryMap(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Unsupported(s) => f.write_str(s),
            ImportError::NeedsMemoryMap(s) => write!(f, "{s}: it does not fit a RAM or ROM component"),
        }
    }
}

impl std::error::Error for ImportError {}

/// Circuits converted from a design, children before the circuits using them.
#[derive(Clone, Debug, Default)]
pub struct Imported {
    pub circuits: Vec<Circuit>,
    pub warnings: Vec<String>,
}

/// Converts the modules reachable from `tops` (every module when `tops` is
/// empty) into placed circuits marked as synthesized from sources with
/// `digest`.
pub fn import(design: &Design, tops: &[String], digest: &str) -> Result<Imported, ImportError> {
    let order = module_order(design, tops)?;
    let names = circuit_names(&order);
    let mut placed = Project::default();
    let mut out = Imported::default();
    for m in order {
        let mut b = Builder::new(design, m, &names);
        for cell in &m.cells {
            b.cell(cell)?;
        }
        let (mut circuit, warnings) = b.finish()?;
        circuit.hdl = Some(HdlInfo { digest: digest.to_string(), stale: false });
        place(&placed, &mut circuit);
        placed.circuits.push(circuit.clone());
        out.warnings.extend(warnings);
        out.circuits.push(circuit);
    }
    Ok(out)
}

/// Modules reachable from `tops`, children first.
fn module_order<'a>(design: &'a Design, tops: &[String]) -> Result<Vec<&'a Module>, ImportError> {
    fn visit<'a>(
        design: &'a Design,
        m: &'a Module,
        seen: &mut HashSet<&'a str>,
        out: &mut Vec<&'a Module>,
    ) -> Result<(), ImportError> {
        if !seen.insert(&m.name) {
            return Ok(());
        }
        if m.blackbox {
            return Err(ImportError::Unsupported(format!("module “{}” has no definition", m.name)));
        }
        for c in &m.cells {
            if let Some(child) = design.module(&c.ty) {
                visit(design, child, seen, out)?;
            }
        }
        out.push(m);
        Ok(())
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    if tops.is_empty() {
        for m in design.modules.iter().filter(|m| !m.blackbox) {
            visit(design, m, &mut seen, &mut out)?;
        }
    } else {
        for t in tops {
            let m = design
                .modules
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case(t))
                .ok_or_else(|| ImportError::Unsupported(format!("the netlist has no module “{t}”")))?;
            visit(design, m, &mut seen, &mut out)?;
        }
    }
    Ok(out)
}

/// Readable, unique circuit names for the modules.
fn circuit_names(modules: &[&Module]) -> HashMap<String, String> {
    let mut used = HashSet::new();
    let mut out = HashMap::new();
    for m in modules {
        let base = readable_name(&m.name);
        let mut name = base.clone();
        let mut k = 2;
        while !used.insert(name.clone()) {
            name = format!("{base} {k}");
            k += 1;
        }
        out.insert(m.name.clone(), name);
    }
    out
}

/// A module name without Yosys decorations: `$paramod\counter\WIDTH=s32'00…01000`
/// becomes `counter WIDTH=8`.
pub fn readable_name(name: &str) -> String {
    let name = name.strip_prefix('\\').unwrap_or(name);
    if !name.starts_with("$paramod") {
        return name.to_string();
    }
    let mut parts = name.split('\\').skip(1);
    let Some(base) = parts.next() else { return name.to_string() };
    let mut out = base.to_string();
    for p in parts {
        let Some((key, value)) = p.split_once('=') else { continue };
        let value = match value.rsplit_once('\'') {
            Some((_, digits)) if digits.bytes().all(|b| b == b'0' || b == b'1') && digits.len() <= 64 => {
                u64::from_str_radix(digits, 2).map(|v| v.to_string()).unwrap_or_else(|_| value.to_string())
            }
            _ => value.trim_matches('"').to_string(),
        };
        out.push_str(&format!(" {key}={value}"));
    }
    out
}

fn ext(bits: &[Bit], w: usize, signed: bool) -> Vec<Bit> {
    let fill = if signed { bits.last().copied().unwrap_or(Bit::Zero) } else { Bit::Zero };
    let mut v: Vec<Bit> = bits.iter().take(w).copied().collect();
    v.resize(w, fill);
    v
}

/// A constant as a number, with unknown and floating bits read as 0.
fn const_value(bits: &[Bit]) -> u64 {
    bits.iter().enumerate().fold(0, |acc, (i, b)| acc | ((*b == Bit::One) as u64) << i)
}

/// Address bits needed for `size` words.
fn addr_bits(size: u64) -> u64 {
    (64 - size.saturating_sub(1).leading_zeros() as u64).max(1)
}

/// Where the bits of a vector read come from: a slice of an output pin
/// (`outs` index, first bit, length) or constants.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Part {
    Pin(usize, usize, usize),
    Const(Vec<Bit>),
}

/// A piece of a vector read once splitters exist: a whole pin, or constants.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Seg {
    Pin(PinRef, usize),
    Const(Vec<Bit>),
}

struct Builder<'a> {
    design: &'a Design,
    module: &'a Module,
    names: &'a HashMap<String, String>,
    circuit: Circuit,
    next_id: CompId,
    /// Output pins and the bits they drive.
    outs: Vec<(PinRef, Vec<Bit>)>,
    /// Input pins and the bits they read.
    reads: Vec<(PinRef, Vec<Bit>)>,
    /// Bits that are another bit (`assign a = b`).
    alias: HashMap<u64, Bit>,
    next_net: u64,
    /// Public net names by their bits.
    net_names: HashMap<Vec<Bit>, String>,
    unsupported: BTreeSet<String>,
}

impl<'a> Builder<'a> {
    fn new(design: &'a Design, module: &'a Module, names: &'a HashMap<String, String>) -> Self {
        let mut max = 1;
        let all_bits = module
            .ports
            .iter()
            .flat_map(|p| &p.bits)
            .chain(module.cells.iter().flat_map(|c| c.connections.iter().flat_map(|(_, b)| b)))
            .chain(module.netnames.iter().flat_map(|n| &n.bits));
        for b in all_bits {
            if let Bit::Net(n) = b {
                max = max.max(*n);
            }
        }
        let mut net_names = HashMap::new();
        for n in module.netnames.iter().filter(|n| !n.hidden) {
            net_names.entry(n.bits.clone()).or_insert_with(|| n.name.clone());
        }
        let mut b = Builder {
            design,
            module,
            names,
            circuit: Circuit::new(names[&module.name].clone()),
            next_id: 1,
            outs: Vec::new(),
            reads: Vec::new(),
            alias: HashMap::new(),
            next_net: max + 1,
            net_names,
            unsupported: BTreeSet::new(),
        };
        // Terminals first, in declaration order: they are placed top to
        // bottom in this order, which is the pin order of the subcircuit.
        for p in &module.ports {
            match p.dir {
                Dir::Input => {
                    let id = b.add(Kind::Input, |q| {
                        q.width = p.bits.len().min(MAX_WIDTH) as u8;
                        q.label = p.name.clone();
                    });
                    b.drive(id, "out", &p.bits);
                }
                Dir::Output => {
                    let id = b.add(Kind::Output, |q| {
                        q.width = p.bits.len().min(MAX_WIDTH) as u8;
                        q.label = p.name.clone();
                    });
                    b.read(id, "in", p.bits.clone());
                }
                Dir::Inout => {
                    b.unsupported.insert(format!("bidirectional port “{}”", p.name));
                }
            }
        }
        b
    }

    // ---- building blocks -------------------------------------------------------

    fn add(&mut self, kind: Kind, f: impl FnOnce(&mut Props)) -> CompId {
        let mut props = kind.default_props();
        f(&mut props);
        let id = self.next_id;
        self.next_id += 1;
        self.circuit.components.push(Component { id, kind, pos: (0, 0), rot: 0, mirror: false, props });
        id
    }

    fn check_width(&mut self, n: usize) {
        if n > MAX_WIDTH {
            self.unsupported.insert(format!("signals wider than {MAX_WIDTH} bits ({n} bits)"));
        }
    }

    fn drive(&mut self, id: CompId, pin: &str, bits: &[Bit]) {
        self.check_width(bits.len());
        self.outs.push((PinRef::new(id, pin), bits.to_vec()));
    }

    fn read(&mut self, id: CompId, pin: &str, bits: Vec<Bit>) {
        self.check_width(bits.len());
        if !bits.is_empty() {
            self.reads.push((PinRef::new(id, pin), bits));
        }
    }

    fn fresh(&mut self, n: usize) -> Vec<Bit> {
        let start = self.next_net;
        self.next_net += n as u64;
        (start..start + n as u64).map(Bit::Net).collect()
    }

    /// Makes the net bits of `dst` the bits of `src`.
    fn assign(&mut self, dst: &[Bit], src: &[Bit]) {
        for (d, s) in dst.iter().zip(src) {
            if let Bit::Net(n) = d
                && Bit::Net(*n) != *s
            {
                self.alias.insert(*n, *s);
            }
        }
    }

    /// Adds a component reading `ins` and returns fresh bits for its outputs.
    fn unit(
        &mut self,
        kind: Kind,
        props: impl FnOnce(&mut Props),
        ins: Vec<(&str, Vec<Bit>)>,
        outs: &[(&str, usize)],
    ) -> Vec<Vec<Bit>> {
        let id = self.add(kind, props);
        for (pin, bits) in ins {
            self.read(id, pin, bits);
        }
        outs.iter()
            .map(|&(pin, w)| {
                let bits = self.fresh(w);
                self.drive(id, pin, &bits);
                bits
            })
            .collect()
    }

    fn not(&mut self, b: Bit) -> Bit {
        match b {
            Bit::Zero => Bit::One,
            Bit::One => Bit::Zero,
            _ => self.unit(Kind::Not, |_| {}, vec![("in", vec![b])], &[("out", 1)])[0][0],
        }
    }

    /// A gate of `kind` over `ins`, all of the same width.
    fn gate(&mut self, kind: Kind, ins: Vec<Vec<Bit>>) -> Vec<Bit> {
        let w = ins[0].len();
        let n = ins.len() as u8;
        let names: Vec<String> = (0..ins.len()).map(|i| format!("in{i}")).collect();
        let ins = names.iter().map(String::as_str).zip(ins).collect();
        self.unit(
            kind,
            |p| {
                p.width = w as u8;
                p.inputs = n;
            },
            ins,
            &[("out", w)],
        )
        .remove(0)
    }

    /// `sel ? b : a`.
    fn mux2(&mut self, a: Vec<Bit>, b: Vec<Bit>, sel: Bit) -> Vec<Bit> {
        let w = a.len();
        self.unit(
            Kind::Mux,
            |p| {
                p.width = w as u8;
                p.select = 1;
            },
            vec![("in0", a), ("in1", b), ("sel", vec![sel])],
            &[("out", w)],
        )
        .remove(0)
    }

    /// Selects `inputs[sel]`, with a tree of multiplexers when `sel` is wider
    /// than one multiplexer.
    fn mux_tree(&mut self, inputs: Vec<Vec<Bit>>, sel: &[Bit]) -> Vec<Bit> {
        let w = inputs[0].len();
        if sel.len() <= MAX_SELECT {
            let s = sel.len();
            let names: Vec<String> = (0..1usize << s).map(|i| format!("in{i}")).collect();
            let mut ins: Vec<(&str, Vec<Bit>)> = names.iter().map(String::as_str).zip(inputs).collect();
            ins.push(("sel", sel.to_vec()));
            return self
                .unit(
                    Kind::Mux,
                    |p| {
                        p.width = w as u8;
                        p.select = s as u8;
                    },
                    ins,
                    &[("out", w)],
                )
                .remove(0);
        }
        let (lo, hi) = sel.split_at(MAX_SELECT);
        let groups = inputs.chunks(1 << MAX_SELECT).map(|g| self.mux_tree(g.to_vec(), lo)).collect();
        self.mux_tree(groups, hi)
    }

    /// Bit equal to 1 when `bits` are all zero.
    fn is_zero(&mut self, bits: &[Bit]) -> Bit {
        let w = bits.len();
        if w == 1 {
            return self.not(bits[0]);
        }
        self.compare_eq(bits, &vec![Bit::Zero; w])
    }

    /// Bit equal to 1 when any of `bits` is 1.
    fn nonzero(&mut self, bits: &[Bit]) -> Bit {
        let live: Vec<Bit> = bits.iter().copied().filter(|b| *b != Bit::Zero).collect();
        match live.as_slice() {
            [] => Bit::Zero,
            [b] => *b,
            _ => {
                let z = self.is_zero(&live);
                self.not(z)
            }
        }
    }

    fn compare_eq(&mut self, a: &[Bit], b: &[Bit]) -> Bit {
        let w = a.len();
        let outs = self.unit(
            Kind::Comparator,
            |p| p.width = w as u8,
            vec![("a", a.to_vec()), ("b", b.to_vec())],
            &[("lt", 1), ("eq", 1), ("gt", 1)],
        );
        outs[1][0]
    }

    /// XOR of all `bits`, with a tree of gates of up to eight inputs.
    fn parity(&mut self, bits: &[Bit]) -> Bit {
        if bits.len() == 1 {
            return bits[0];
        }
        let next: Vec<Bit> = bits
            .chunks(MAX_GATE_INPUTS)
            .map(|g| if g.len() == 1 { g[0] } else { self.gate(Kind::Xor, g.iter().map(|b| vec![*b]).collect())[0] })
            .collect();
        self.parity(&next)
    }

    /// A control input of the given polarity, made active high.
    fn active_high(&mut self, b: Bit, polarity: bool) -> Bit {
        if polarity { b } else { self.not(b) }
    }

    /// The public name of the net made of `bits`, if any.
    fn name_of(&self, bits: &[Bit]) -> String {
        self.net_names.get(bits).cloned().unwrap_or_default()
    }

    // ---- cells -----------------------------------------------------------------

    fn cell(&mut self, cell: &Cell) -> Result<(), ImportError> {
        let t = cell.ty.as_str();
        let (a, b, y) = (cell.port("A"), cell.port("B"), cell.port("Y"));
        let (sa, sb) = (cell.flag("A_SIGNED"), cell.flag("B_SIGNED"));
        let yw = y.len();
        match t {
            "$pos" | "$buf" | "$_BUF_" => self.assign(y, &ext(a, yw, sa)),
            "$not" | "$_NOT_" => {
                let out = self.gate1(Kind::Not, ext(a, yw, sa));
                self.assign(y, &out);
            }
            "$neg" => {
                let out = self.unit(
                    Kind::Subtractor,
                    |p| p.width = yw as u8,
                    vec![("a", vec![Bit::Zero; yw]), ("b", ext(a, yw, sa))],
                    &[("out", yw), ("bout", 1)],
                );
                self.assign(y, &out[0]);
            }
            "$and" | "$or" | "$xor" | "$xnor" | "$_AND_" | "$_OR_" | "$_XOR_" | "$_XNOR_" | "$_NAND_" | "$_NOR_" => {
                let kind = match t.trim_matches(|c| c == '$' || c == '_') {
                    "and" | "AND" => Kind::And,
                    "or" | "OR" => Kind::Or,
                    "xor" | "XOR" => Kind::Xor,
                    "NAND" => Kind::Nand,
                    "NOR" => Kind::Nor,
                    _ => Kind::Xnor,
                };
                let s = sa && sb;
                let out = self.gate(kind, vec![ext(a, yw, s), ext(b, yw, s)]);
                self.assign(y, &out);
            }
            "$_ANDNOT_" | "$_ORNOT_" => {
                let nb = self.not(b[0]);
                let kind = if t == "$_ANDNOT_" { Kind::And } else { Kind::Or };
                let out = self.gate(kind, vec![a.to_vec(), vec![nb]]);
                self.assign(y, &out);
            }
            "$_MUX_" | "$_NMUX_" => {
                let mut out = self.mux2(a.to_vec(), b.to_vec(), cell.port("S")[0]);
                if t == "$_NMUX_" {
                    out = vec![self.not(out[0])];
                }
                self.assign(y, &out);
            }
            "$reduce_and" | "$reduce_or" | "$reduce_bool" | "$reduce_xor" | "$reduce_xnor" | "$logic_not" => {
                let r = match t {
                    "$reduce_and" if a.len() == 1 => a[0],
                    "$reduce_and" => self.compare_eq(a, &vec![Bit::One; a.len()]),
                    "$reduce_or" | "$reduce_bool" => self.nonzero(a),
                    "$reduce_xor" => self.parity(a),
                    "$reduce_xnor" => {
                        let p = self.parity(a);
                        self.not(p)
                    }
                    _ => self.is_zero(a),
                };
                self.assign(y, &ext(&[r], yw, false));
            }
            "$logic_and" | "$logic_or" => {
                let (x, z) = (self.nonzero(a), self.nonzero(b));
                let kind = if t == "$logic_and" { Kind::And } else { Kind::Or };
                let r = self.gate(kind, vec![vec![x], vec![z]])[0];
                self.assign(y, &ext(&[r], yw, false));
            }
            "$eq" | "$eqx" | "$ne" | "$nex" | "$lt" | "$le" | "$gt" | "$ge" => {
                let w = a.len().max(b.len());
                let s = sa && sb;
                let outs = self.unit(
                    Kind::Comparator,
                    |p| {
                        p.width = w as u8;
                        p.signed = s;
                    },
                    vec![("a", ext(a, w, s)), ("b", ext(b, w, s))],
                    &[("lt", 1), ("eq", 1), ("gt", 1)],
                );
                let (pin, invert) = match t {
                    "$eq" | "$eqx" => (1, false),
                    "$ne" | "$nex" => (1, true),
                    "$lt" => (0, false),
                    "$ge" => (0, true),
                    "$gt" => (2, false),
                    _ => (2, true),
                };
                let r = outs[pin][0];
                let r = if invert { self.not(r) } else { r };
                self.assign(y, &ext(&[r], yw, false));
            }
            "$add" | "$sub" => {
                let s = sa && sb;
                let (kind, carry) = if t == "$add" { (Kind::Adder, "cout") } else { (Kind::Subtractor, "bout") };
                let out = self.unit(
                    kind,
                    |p| p.width = yw as u8,
                    vec![("a", ext(a, yw, s)), ("b", ext(b, yw, s))],
                    &[("out", yw), (carry, 1)],
                );
                self.assign(y, &out[0]);
            }
            "$shl" | "$sshl" | "$shr" | "$sshr" | "$shift" | "$shiftx" => self.shift(cell),
            "$mux" => {
                let out = self.mux2(a.to_vec(), b.to_vec(), cell.port("S")[0]);
                self.assign(y, &out);
            }
            "$bmux" => {
                let w = cell.uint("WIDTH") as usize;
                let inputs = a.chunks(w.max(1)).map(<[Bit]>::to_vec).collect();
                let out = self.mux_tree(inputs, cell.port("S"));
                self.assign(y, &out);
            }
            "$demux" => {
                let s = cell.port("S");
                if s.len() > MAX_SELECT {
                    self.unsupported.insert(format!("demultiplexers with more than {MAX_SELECT} select bits"));
                    return Ok(());
                }
                let w = a.len();
                let names: Vec<String> = (0..1usize << s.len()).map(|i| format!("out{i}")).collect();
                let outs: Vec<(&str, usize)> = names.iter().map(|n| (n.as_str(), w)).collect();
                let sel = s.len() as u8;
                let outs = self.unit(
                    Kind::Demux,
                    |p| {
                        p.width = w as u8;
                        p.select = sel;
                    },
                    vec![("in", a.to_vec()), ("sel", s.to_vec())],
                    &outs,
                );
                self.assign(y, &outs.concat());
            }
            "$tribuf" | "$_TBUF_" => {
                let w = a.len();
                let id = self.add(Kind::TriBuffer, |p| p.width = w as u8);
                self.read(id, "in", a.to_vec());
                self.read(id, "en", cell.port("EN").to_vec());
                self.drive(id, "out", y);
            }
            "$dff" | "$dffe" | "$adff" | "$adffe" | "$sdff" | "$sdffe" | "$sdffce" => self.flip_flop(cell),
            "$_DFF_P_" | "$_DFF_N_" => {
                let edge = if t == "$_DFF_P_" { Edge::Rising } else { Edge::Falling };
                let q = cell.port("Q");
                let label = self.name_of(q);
                let id = self.add(Kind::DFlipFlop, |p| {
                    p.edge = edge;
                    p.label = label;
                });
                self.read(id, "d", cell.port("D").to_vec());
                self.read(id, "clk", cell.port("C").to_vec());
                self.drive(id, "q", q);
            }
            "$dlatch" | "$_DLATCH_P_" | "$_DLATCH_N_" => {
                let pol = match t {
                    "$dlatch" => cell.flag("EN_POLARITY"),
                    _ => t == "$_DLATCH_P_",
                };
                let en = cell.port("EN").first().or(cell.port("E").first()).copied().unwrap_or(Bit::Zero);
                let en = self.active_high(en, pol);
                let (d, q) = (cell.port("D").to_vec(), cell.port("Q").to_vec());
                let label = if q.len() == 1 { self.name_of(&q) } else { String::new() };
                for (d, q) in d.into_iter().zip(q) {
                    let label = label.clone();
                    let id = self.add(Kind::DLatch, |p| p.label = label);
                    self.read(id, "d", vec![d]);
                    self.read(id, "en", vec![en]);
                    self.drive(id, "q", &[q]);
                }
            }
            "$mem_v2" => self.memory(cell)?,
            "$concat" => self.assign(y, &[a, b].concat()),
            "$slice" => {
                let off = cell.uint("OFFSET") as usize;
                let src: Vec<Bit> = a.iter().skip(off).copied().collect();
                self.assign(y, &ext(&src, yw, false));
            }
            // Verification and debug cells have no hardware.
            "$assert" | "$assume" | "$cover" | "$live" | "$fair" | "$check" | "$print" | "$scopeinfo" => {}
            _ => match self.design.module(t) {
                Some(child) => self.instance(cell, child),
                None => {
                    self.unsupported.insert(format!("cell type {t}"));
                }
            },
        }
        Ok(())
    }

    fn gate1(&mut self, kind: Kind, a: Vec<Bit>) -> Vec<Bit> {
        let w = a.len();
        self.unit(kind, |p| p.width = w as u8, vec![("in", a)], &[("out", w)]).remove(0)
    }

    fn instance(&mut self, cell: &Cell, child: &Module) {
        let circuit = self.names.get(&child.name).cloned().unwrap_or_else(|| child.name.clone());
        let label = if cell.hidden { String::new() } else { cell.name.clone() };
        let id = self.add(Kind::Subcircuit, |p| {
            p.circuit = circuit;
            p.label = label;
        });
        for port in &child.ports {
            let bits = cell.port(&port.name).to_vec();
            match port.dir {
                Dir::Input => self.read(id, &port.name, bits),
                Dir::Output => self.drive(id, &port.name, &bits),
                Dir::Inout => {
                    self.unsupported.insert(format!("bidirectional port “{}”", port.name));
                }
            }
        }
    }

    fn shift(&mut self, cell: &Cell) {
        let t = cell.ty.as_str();
        let (a, b, y) = (cell.port("A"), cell.port("B"), cell.port("Y"));
        let sa = cell.flag("A_SIGNED");
        if matches!(t, "$shift" | "$shiftx") && cell.flag("B_SIGNED") {
            self.unsupported.insert("shifts by a signed amount".into());
            return;
        }
        let w = a.len().max(y.len());
        let mode = match t {
            "$shl" | "$sshl" => ShiftMode::Lsl,
            "$sshr" if sa => ShiftMode::Asr,
            _ => ShiftMode::Lsr,
        };
        let value = ext(a, w, sa);
        let k = shift_amount_width(w.min(MAX_WIDTH) as u8) as usize;
        let (low, high) = if b.len() > k { (b[..k].to_vec(), b[k..].to_vec()) } else { (ext(b, k, false), Vec::new()) };
        let shifted = self
            .unit(
                Kind::Shifter,
                |p| {
                    p.width = w as u8;
                    p.shift = mode;
                },
                vec![("in", value.clone()), ("sh", low)],
                &[("out", w)],
            )
            .remove(0);
        // Amounts beyond the shifter input shift everything out.
        let over = self.nonzero(&high);
        let out = if over == Bit::Zero {
            shifted
        } else {
            let fill = if mode == ShiftMode::Asr { vec![value[w - 1]; w] } else { vec![Bit::Zero; w] };
            self.mux2(shifted, fill, over)
        };
        self.assign(y, &out[..y.len()]);
    }

    fn flip_flop(&mut self, cell: &Cell) {
        let t = cell.ty.as_str();
        let (d, q) = (cell.port("D").to_vec(), cell.port("Q").to_vec());
        let w = q.len();
        let edge = if cell.flag("CLK_POLARITY") { Edge::Rising } else { Edge::Falling };
        let mut d = d;
        let mut en = None;
        if matches!(t, "$dffe" | "$adffe" | "$sdffe" | "$sdffce") {
            en = Some(self.active_high(cell.port("EN")[0], cell.flag("EN_POLARITY")));
        }
        // Synchronous reset: load the reset value through a multiplexer.
        if matches!(t, "$sdff" | "$sdffe" | "$sdffce") {
            let s = self.active_high(cell.port("SRST")[0], cell.flag("SRST_POLARITY"));
            let value = ext(cell.param("SRST_VALUE").map_or(&[], |p| p.bits()), w, false);
            let value: Vec<Bit> = value.into_iter().map(|b| if b == Bit::One { b } else { Bit::Zero }).collect();
            d = self.mux2(d, value, s);
            // `$sdffe` resets even when disabled.
            if t == "$sdffe"
                && let Some(e) = en
            {
                en = Some(self.gate(Kind::Or, vec![vec![e], vec![s]])[0]);
            }
        }
        let mut rst = None;
        let mut flip = Vec::new();
        if matches!(t, "$adff" | "$adffe") {
            rst = Some(self.active_high(cell.port("ARST")[0], cell.flag("ARST_POLARITY")));
            let value = ext(cell.param("ARST_VALUE").map_or(&[], |p| p.bits()), w, false);
            // The register resets to 0: store the bits of a non-zero reset
            // value inverted, and invert them back on the way out.
            if value.contains(&Bit::One) {
                flip = value.into_iter().map(|b| if b == Bit::One { b } else { Bit::Zero }).collect();
            }
        }
        if !flip.is_empty() {
            d = self.gate(Kind::Xor, vec![d, flip.clone()]);
        }
        let label = self.name_of(&q);
        let mut ins = vec![("d", d), ("clk", cell.port("CLK").to_vec())];
        if let Some(e) = en {
            ins.push(("en", vec![e]));
        }
        if let Some(r) = rst {
            ins.push(("rst", vec![r]));
        }
        let out = self
            .unit(
                Kind::Register,
                |p| {
                    p.width = w as u8;
                    p.edge = edge;
                    p.label = label;
                },
                ins,
                &[("q", w)],
            )
            .remove(0);
        let out = if flip.is_empty() { out } else { self.gate(Kind::Xor, vec![out, flip]) };
        self.assign(&q, &out);
    }

    fn memory(&mut self, cell: &Cell) -> Result<(), ImportError> {
        let name = match cell.param("MEMID") {
            Some(crate::json::Param::Str(s)) => s.trim_start_matches('\\').to_string(),
            _ => cell.name.clone(),
        };
        let needs_map = |why: &str| Err(ImportError::NeedsMemoryMap(format!("memory “{name}” ({why})")));
        let size = cell.uint("SIZE");
        let abits = cell.uint("ABITS") as usize;
        let width = cell.uint("WIDTH") as usize;
        let (rd, wr) = (cell.uint("RD_PORTS") as usize, cell.uint("WR_PORTS") as usize);
        let set = |p: &str, i: usize| cell.param(p).and_then(|v| v.bits().get(i).copied()) == Some(Bit::One);
        let wide = |p: &str| cell.param(p).is_some_and(|v| v.bits().contains(&Bit::One));
        if cell.uint("OFFSET") != 0 {
            return needs_map("address offset");
        }
        if wide("RD_WIDE_CONTINUATION") || wide("WR_WIDE_CONTINUATION") {
            return needs_map("wide ports");
        }
        let aw = (abits as u64).min(addr_bits(size));
        if aw > MAX_ADDR_BITS {
            self.unsupported.insert(format!("memory “{name}”: more than 2^{MAX_ADDR_BITS} words"));
            return Ok(());
        }
        let aw = aw as usize;
        let init = cell.param("INIT").map_or(&[][..], |p| p.bits());
        let words: Vec<u64> =
            (0..size as usize).map(|i| const_value(init.get(i * width..(i + 1) * width).unwrap_or(&[]))).collect();
        let data = format_hex(&words, false);
        let port = |p: &str, i: usize, w: usize| cell.port(p).get(i * w..(i + 1) * w).unwrap_or(&[]).to_vec();
        let resets_unused = |p: &str| cell.port(p).iter().all(|b| *b == Bit::Zero) || cell.port(p).is_empty();
        if !resets_unused("RD_ARST") || !resets_unused("RD_SRST") {
            return needs_map("read port reset");
        }
        let read_data = |b: &mut Self, i: usize, out: Vec<Bit>| {
            let data_bits = port("RD_DATA", i, width);
            let out = if set("RD_CLK_ENABLE", i) {
                let edge = if set("RD_CLK_POLARITY", i) { Edge::Rising } else { Edge::Falling };
                let mut ins = vec![("d", out), ("clk", port("RD_CLK", i, 1))];
                let en = port("RD_EN", i, 1);
                if en != [Bit::One] {
                    ins.push(("en", en));
                }
                b.unit(
                    Kind::Register,
                    |p| {
                        p.width = width as u8;
                        p.edge = edge;
                    },
                    ins,
                    &[("q", width)],
                )
                .remove(0)
            } else {
                out
            };
            b.assign(&data_bits, &out);
        };
        if wr == 0 {
            for i in 0..rd {
                let addr = port("RD_ADDR", i, abits)[..aw].to_vec();
                let out = self
                    .unit(
                        Kind::Rom,
                        |p| {
                            p.addr_width = aw as u8;
                            p.width = width as u8;
                            p.data = data.clone();
                            p.label = name.clone();
                        },
                        vec![("addr", addr)],
                        &[("data", width)],
                    )
                    .remove(0);
                read_data(self, i, out);
            }
            return Ok(());
        }
        if wr != 1 || rd != 1 {
            return needs_map(&format!("{rd} read and {wr} write ports"));
        }
        let raddr = port("RD_ADDR", 0, abits);
        if port("WR_ADDR", 0, abits) != raddr {
            return needs_map("separate read and write addresses");
        }
        let we = port("WR_EN", 0, width);
        if we.iter().any(|b| *b != we[0]) {
            return needs_map("write enable per bit");
        }
        if !set("WR_CLK_ENABLE", 0) {
            return needs_map("asynchronous write");
        }
        let edge = if set("WR_CLK_POLARITY", 0) { Edge::Rising } else { Edge::Falling };
        let out = self
            .unit(
                Kind::Ram,
                |p| {
                    p.addr_width = aw as u8;
                    p.width = width as u8;
                    p.edge = edge;
                    p.data = data.clone();
                    p.label = name.clone();
                },
                vec![
                    ("addr", raddr[..aw].to_vec()),
                    ("din", port("WR_DATA", 0, width)),
                    ("we", vec![we[0]]),
                    ("clk", port("WR_CLK", 0, 1)),
                ],
                &[("dout", width)],
            )
            .remove(0);
        read_data(self, 0, out);
        Ok(())
    }

    // ---- wiring ----------------------------------------------------------------

    fn resolve(&self, mut b: Bit) -> Bit {
        // Bounded: aliases never form cycles, but a malformed netlist must
        // not hang the import.
        for _ in 0..1_000_000 {
            match b {
                Bit::Net(n) => match self.alias.get(&n) {
                    Some(&t) => b = t,
                    None => return b,
                },
                _ => return b,
            }
        }
        b
    }

    /// Connects every vector read to the pins driving it, then returns the
    /// circuit and the warnings.
    fn finish(mut self) -> Result<(Circuit, Vec<String>), ImportError> {
        if !self.unsupported.is_empty() {
            let list: Vec<&str> = self.unsupported.iter().map(String::as_str).collect();
            return Err(ImportError::Unsupported(format!(
                "module “{}” uses what LogiBuild cannot represent: {}",
                self.module.name,
                list.join(", ")
            )));
        }
        let mut warnings = Vec::new();
        let mut wires: Vec<Wire> = Vec::new();

        // The output pin and bit driving each net bit. Pins driving exactly
        // the same bits (tri-state buffers) are wired together.
        let mut driver: HashMap<u64, (usize, usize)> = HashMap::new();
        for i in 0..self.outs.len() {
            let (pin, bits) = &self.outs[i];
            if let Some(Bit::Net(first)) = bits.first()
                && let Some(&(j, 0)) = driver.get(first)
                && self.outs[j].1 == *bits
            {
                wires.push(Wire::new(self.outs[j].0.clone(), pin.clone()));
                continue;
            }
            for (k, b) in bits.iter().enumerate() {
                if let Bit::Net(n) = b
                    && driver.insert(*n, (i, k)).is_some()
                {
                    return Err(ImportError::Unsupported(format!(
                        "module “{}”: a signal has several drivers",
                        self.module.name
                    )));
                }
            }
        }

        // Each read as parts: slices of driving pins and constants.
        let mut undriven = 0;
        let mut reads: Vec<(PinRef, Vec<Part>)> = Vec::new();
        for (pin, bits) in std::mem::take(&mut self.reads) {
            let mut parts: Vec<Part> = Vec::new();
            for b in bits {
                let mut b = self.resolve(b);
                if let Bit::Net(n) = b
                    && !driver.contains_key(&n)
                {
                    undriven += 1;
                    b = Bit::X;
                }
                match (b, parts.last_mut()) {
                    (Bit::Net(n), Some(Part::Pin(i, lo, len))) if driver[&n] == (*i, *lo + *len) => *len += 1,
                    (Bit::Net(n), _) => {
                        let (i, k) = driver[&n];
                        parts.push(Part::Pin(i, k, 1));
                    }
                    (c, Some(Part::Const(v))) => v.push(c),
                    (c, _) => parts.push(Part::Const(vec![c])),
                }
            }
            reads.push((pin, parts));
        }
        if undriven > 0 {
            warnings.push(format!("{}: {undriven} undriven bits read as 0", self.circuit.name));
        }

        // One splitter per pin read in slices, cut where the slices start
        // and end.
        let mut cuts: HashMap<usize, BTreeSet<usize>> = HashMap::new();
        for (_, parts) in &reads {
            for p in parts {
                if let Part::Pin(i, lo, len) = *p
                    && len < self.outs[i].1.len()
                {
                    cuts.entry(i).or_default().extend([lo, lo + len]);
                }
            }
        }
        // Pieces of each split pin: (first bit, pin of the splitter).
        let mut pieces: HashMap<usize, Vec<(usize, PinRef)>> = HashMap::new();
        let mut split: Vec<usize> = cuts.keys().copied().collect();
        split.sort();
        for i in split {
            let w = self.outs[i].1.len();
            let mut points = cuts.remove(&i).unwrap();
            points.extend([0, w]);
            let points: Vec<usize> = points.into_iter().collect();
            let widths: Vec<u8> = points.windows(2).map(|p| (p[1] - p[0]) as u8).collect();
            let id = self.add(Kind::Splitter, |p| {
                p.width = w as u8;
                p.pieces = widths;
            });
            wires.push(Wire::new(self.outs[i].0.clone(), PinRef::new(id, "in")));
            pieces.insert(
                i,
                points[..points.len() - 1]
                    .iter()
                    .enumerate()
                    .map(|(k, &lo)| (lo, PinRef::new(id, format!("out{k}"))))
                    .collect(),
            );
        }

        // Connect each read, through a merger when it has several pieces;
        // mergers gathering the same pieces are shared.
        let mut mergers: HashMap<Vec<Seg>, PinRef> = HashMap::new();
        for (reader, parts) in reads {
            let mut segs: Vec<Seg> = Vec::new();
            for p in parts {
                match p {
                    Part::Const(v) => segs.push(Seg::Const(v)),
                    Part::Pin(i, lo, len) => {
                        let w = self.outs[i].1.len();
                        if len == w {
                            segs.push(Seg::Pin(self.outs[i].0.clone(), w));
                            continue;
                        }
                        let list = &pieces[&i];
                        for (k, (start, pin)) in list.iter().enumerate() {
                            let end = list.get(k + 1).map_or(w, |n| n.0);
                            if *start >= lo && end <= lo + len {
                                segs.push(Seg::Pin(pin.clone(), end - start));
                            }
                        }
                    }
                }
            }
            let source = match segs.as_slice() {
                [one] => self.seg_pin(one),
                _ => match mergers.get(&segs) {
                    Some(p) => p.clone(),
                    None => {
                        let total: usize = segs.iter().map(Seg::width).sum();
                        let widths: Vec<u8> = segs.iter().map(|s| s.width() as u8).collect();
                        let id = self.add(Kind::Merger, |p| {
                            p.width = total as u8;
                            p.pieces = widths;
                        });
                        for (k, s) in segs.iter().enumerate() {
                            let from = self.seg_pin(s);
                            wires.push(Wire::new(from, PinRef::new(id, format!("in{k}"))));
                        }
                        let out = PinRef::new(id, "out");
                        mergers.insert(segs, out.clone());
                        out
                    }
                },
            };
            wires.push(Wire::new(source, reader));
        }

        self.tunnel_fanouts(&mut wires);
        self.circuit.wires = wires;
        self.circuit.normalize();
        Ok((self.circuit, warnings))
    }

    /// The pin carrying a piece of a read, adding a constant for constants.
    fn seg_pin(&mut self, s: &Seg) -> PinRef {
        match s {
            Seg::Pin(p, _) => p.clone(),
            Seg::Const(v) => {
                let (w, value) = (v.len() as u8, const_value(v));
                let id = self.add(Kind::Constant, |p| {
                    p.width = w;
                    p.value = value;
                });
                PinRef::new(id, "out")
            }
        }
    }

    /// Replaces the wires of nets read in many places by tunnels: one at the
    /// driver and one at each reader, labelled with the name of the net.
    fn tunnel_fanouts(&mut self, wires: &mut Vec<Wire>) {
        let mut by_source: HashMap<PinRef, Vec<usize>> = HashMap::new();
        for (i, w) in wires.iter().enumerate() {
            by_source.entry(w.from.clone()).or_default().push(i);
        }
        let mut sources: Vec<(PinRef, Vec<usize>)> =
            by_source.into_iter().filter(|(_, v)| v.len() >= TUNNEL_FANOUT).collect();
        sources.sort();
        let mut labels: HashSet<String> = HashSet::new();
        let mut removed = HashSet::new();
        let mut added = Vec::new();
        for (from, list) in sources {
            let Some((_, bits)) = self.outs.iter().find(|(p, _)| *p == from) else { continue };
            let w = bits.len() as u8;
            let mut label = self.name_of(bits);
            if label.is_empty() {
                label = self.circuit.component(from.0).map(|c| c.props.label.clone()).unwrap_or_default();
            }
            if label.is_empty() {
                label = format!("n{}", labels.len() + 1);
            }
            let base = label.clone();
            let mut k = 2;
            while !labels.insert(label.clone()) {
                label = format!("{base}_{k}");
                k += 1;
            }
            let tunnel = |b: &mut Self| {
                let label = label.clone();
                b.add(Kind::Tunnel, |p| {
                    p.width = w;
                    p.label = label;
                })
            };
            let t = tunnel(self);
            added.push(Wire::new(from.clone(), PinRef::new(t, "in")));
            for i in list {
                removed.insert(i);
                let t = tunnel(self);
                added.push(Wire::new(PinRef::new(t, "in"), wires[i].to.clone()));
            }
        }
        let mut i = 0;
        wires.retain(|_| {
            i += 1;
            !removed.contains(&(i - 1))
        });
        wires.extend(added);
    }
}

impl Seg {
    fn width(&self) -> usize {
        match self {
            Seg::Pin(_, w) => *w,
            Seg::Const(v) => v.len(),
        }
    }
}
