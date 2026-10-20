//! Circuit-level tests (SPEC §9): latches, flip-flops, counters, adders,
//! shift registers, a small ALU and the error cases, built from primitives
//! and subcircuits and run headless.

use lb_core::circuit::ProjectError;
use lb_core::prelude::*;

/// Small helper to build circuits tersely.
struct Builder {
    c: Circuit,
}

impl Builder {
    fn new(name: &str) -> Self {
        Builder { c: Circuit::new(name) }
    }

    fn comp(&mut self, kind: Kind, label: &str) -> CompId {
        let id = self.c.add(kind, (0, 0));
        self.c.component_mut(id).unwrap().props.label = label.into();
        id
    }

    fn with(&mut self, kind: Kind, label: &str, f: impl FnOnce(&mut Props)) -> CompId {
        let id = self.comp(kind, label);
        f(&mut self.c.component_mut(id).unwrap().props);
        id
    }

    fn sub(&mut self, circuit: &str, label: &str) -> CompId {
        self.with(Kind::Subcircuit, label, |p| p.circuit = circuit.into())
    }

    fn w(&mut self, a: CompId, pa: &str, b: CompId, pb: &str) {
        assert!(self.c.connect((a, pa), (b, pb)), "duplicate wire {a}.{pa} - {b}.{pb}");
    }

    fn done(self) -> Circuit {
        self.c
    }
}

/// A running simulation addressed by hierarchical names.
struct Harness {
    sim: Simulator,
}

impl Harness {
    fn new(project: &Project, root: &str, cfg: SimConfig) -> Self {
        let nl = compile(project, root).expect("compiles");
        assert!(nl.diagnostics.is_empty(), "{:?}", nl.diagnostics);
        let mut h = Harness { sim: Simulator::new(nl, cfg) };
        h.settle();
        h
    }

    fn set(&mut self, input: &str, v: u64) {
        let c = self.sim.netlist().comp_by_path(input).unwrap_or_else(|| panic!("no input {input}"));
        let w = self.sim.input_value(c).width();
        self.sim.set_input(c, Signal::new(w, v));
    }

    fn settle(&mut self) {
        assert!(self.sim.settle(100_000).expect("no oscillation"), "did not settle");
    }

    fn get(&self, net: &str) -> Signal {
        let n = self.sim.netlist().net_by_name(net).unwrap_or_else(|| panic!("no net {net}"));
        self.sim.net_value(n)
    }

    fn val(&self, net: &str) -> u64 {
        self.get(net).to_u64().unwrap_or_else(|| panic!("{net} is {:?}", self.get(net)))
    }
}

fn zero_delay() -> SimConfig {
    SimConfig { gate_delay: 0, ..SimConfig::default() }
}

fn both_modes() -> [SimConfig; 2] {
    [SimConfig::default(), zero_delay()]
}

// ---- SR latch from cross-coupled NOR gates --------------------------------

fn sr_latch() -> Project {
    let mut b = Builder::new("sr");
    let s = b.comp(Kind::Input, "S");
    let r = b.with(Kind::Input, "R", |p| p.value = 1);
    let n1 = b.comp(Kind::Nor, "");
    let n2 = b.comp(Kind::Nor, "");
    let q = b.comp(Kind::Output, "Q");
    let nq = b.comp(Kind::Output, "nQ");
    b.w(r, "out", n1, "in0");
    b.w(n2, "out", n1, "in1");
    b.w(s, "out", n2, "in0");
    b.w(n1, "out", n2, "in1");
    b.w(n1, "out", q, "in");
    b.w(n2, "out", nq, "in");
    Project::new(b.done())
}

#[test]
fn sr_latch_gate_level() {
    for cfg in both_modes() {
        let mut h = Harness::new(&sr_latch(), "sr", cfg);
        assert_eq!((h.val("Q"), h.val("nQ")), (0, 1), "reset");
        let steps = [("R", 0, 0), ("S", 1, 1), ("S", 0, 1), ("R", 1, 0), ("R", 0, 0)];
        for (input, v, q) in steps {
            h.set(input, v);
            h.settle();
            assert_eq!(h.val("Q"), q, "after {input}={v} ({cfg:?})");
            assert_eq!(h.val("nQ"), 1 - q);
        }
    }
}

// ---- master-slave D flip-flop from NAND latches -------------------------------

/// Gated D latch built from four NAND gates and an inverter.
fn d_latch() -> Circuit {
    let mut b = Builder::new("dlatch");
    let d = b.comp(Kind::Input, "D");
    let en = b.comp(Kind::Input, "EN");
    let inv = b.comp(Kind::Not, "");
    let [g1, g2, g3, g4] = [0; 4].map(|_| b.comp(Kind::Nand, ""));
    let q = b.comp(Kind::Output, "Q");
    let nq = b.comp(Kind::Output, "nQ");
    b.w(d, "out", g1, "in0");
    b.w(en, "out", g1, "in1");
    b.w(d, "out", inv, "in");
    b.w(inv, "out", g2, "in0");
    b.w(en, "out", g2, "in1");
    b.w(g1, "out", g3, "in0");
    b.w(g4, "out", g3, "in1");
    b.w(g2, "out", g4, "in0");
    b.w(g3, "out", g4, "in1");
    b.w(g3, "out", q, "in");
    b.w(g4, "out", nq, "in");
    b.done()
}

fn d_flip_flop() -> Project {
    let mut b = Builder::new("dff");
    let d = b.comp(Kind::Input, "D");
    let clk = b.comp(Kind::Input, "CLK");
    let inv = b.comp(Kind::Not, "");
    let master = b.sub("dlatch", "master");
    let slave = b.sub("dlatch", "slave");
    let q = b.comp(Kind::Output, "Q");
    b.w(d, "out", master, "D");
    b.w(clk, "out", inv, "in");
    b.w(inv, "out", master, "EN");
    b.w(master, "Q", slave, "D");
    b.w(clk, "out", slave, "EN");
    b.w(slave, "Q", q, "in");
    Project { circuits: vec![b.done(), d_latch()], libraries: vec![] }
}

#[test]
fn d_flip_flop_master_slave() {
    let mut h = Harness::new(&d_flip_flop(), "dff", SimConfig::default());
    let mut expected = h.val("Q");
    // (D, CLK) sequence; Q follows D only on rising edges of CLK.
    let seq = [(1, 0), (1, 1), (0, 1), (0, 0), (0, 1), (1, 0), (1, 1), (1, 0), (0, 0)];
    let mut clk = 0;
    for (d, c) in seq {
        h.set("D", d);
        h.settle();
        h.set("CLK", c);
        h.settle();
        if clk == 0 && c == 1 {
            expected = d;
        }
        clk = c;
        assert_eq!(h.val("Q"), expected, "D={d} CLK={c}");
    }
    // Hierarchical names of the inner nets.
    assert!(h.sim.netlist().net_by_name("master/Q").is_some());
}

#[test]
fn d_flip_flop_component() {
    let mut b = Builder::new("ff");
    let d = b.comp(Kind::Input, "D");
    let clk = b.with(Kind::Clock, "clk", |p| p.period = 20);
    let ff = b.comp(Kind::DFlipFlop, "ff");
    let q = b.comp(Kind::Output, "Q");
    b.w(d, "out", ff, "d");
    b.w(clk, "out", ff, "clk");
    b.w(ff, "q", q, "in");
    let p = Project::new(b.done());
    let mut h = Harness::new(&p, "ff", SimConfig::default());
    h.set("D", 1);
    h.sim.run_until(5).unwrap();
    assert_eq!(h.val("Q"), 0, "no edge yet");
    h.sim.run_until(12).unwrap(); // rising edge at 10, q at 11
    assert_eq!(h.val("Q"), 1);
    h.set("D", 0);
    h.sim.run_until(25).unwrap();
    assert_eq!(h.val("Q"), 1, "falling edge at 20 ignored");
    h.sim.run_until(31).unwrap();
    assert_eq!(h.val("Q"), 0);
}

// ---- 4-bit synchronous counter from T flip-flops ------------------------------

fn counter4() -> Project {
    let mut b = Builder::new("counter");
    let clk = b.with(Kind::Clock, "clk", |p| p.period = 10);
    let one = b.with(Kind::Constant, "", |p| p.value = 1);
    let ffs: Vec<CompId> = (0..4).map(|i| b.comp(Kind::TFlipFlop, &format!("t{i}"))).collect();
    let and1 = b.comp(Kind::And, "");
    let and2 = b.with(Kind::And, "", |p| p.inputs = 3);
    let merge = b.with(Kind::Merger, "", |p| p.width = 4);
    let out = b.with(Kind::Output, "Q", |p| p.width = 4);
    for &ff in &ffs {
        b.w(clk, "out", ff, "clk");
    }
    b.w(one, "out", ffs[0], "t");
    b.w(ffs[0], "q", ffs[1], "t");
    b.w(ffs[0], "q", and1, "in0");
    b.w(ffs[1], "q", and1, "in1");
    b.w(and1, "out", ffs[2], "t");
    b.w(ffs[0], "q", and2, "in0");
    b.w(ffs[1], "q", and2, "in1");
    b.w(ffs[2], "q", and2, "in2");
    b.w(and2, "out", ffs[3], "t");
    for (i, &ff) in ffs.iter().enumerate() {
        b.w(ff, "q", merge, &format!("in{i}"));
    }
    b.w(merge, "out", out, "in");
    Project::new(b.done())
}

#[test]
fn four_bit_counter() {
    for cfg in both_modes() {
        let mut h = Harness::new(&counter4(), "counter", cfg);
        let mut seen = vec![h.val("Q")];
        for _ in 0..40 {
            h.sim.step_clock().unwrap();
            seen.push(h.val("Q"));
        }
        // Two clock edges per count (rising counts, falling holds).
        let counts: Vec<u64> = seen.iter().step_by(2).copied().collect();
        let expected: Vec<u64> = (0..21).map(|i| i % 16).collect();
        assert_eq!(counts, expected, "{cfg:?}");
    }
}

#[test]
fn counter_component_with_carry() {
    let mut b = Builder::new("cnt");
    let clk = b.with(Kind::Clock, "clk", |p| p.period = 4);
    let cnt = b.with(Kind::Counter, "c", |p| p.width = 3);
    let q = b.with(Kind::Output, "Q", |p| p.width = 3);
    let carry = b.comp(Kind::Output, "C");
    b.w(clk, "out", cnt, "clk");
    b.w(cnt, "q", q, "in");
    b.w(cnt, "carry", carry, "in");
    let mut h = Harness::new(&Project::new(b.done()), "cnt", SimConfig::default());
    for i in 1..=10u64 {
        h.sim.step_clock().unwrap(); // rising
        assert_eq!(h.val("Q"), i % 8);
        assert_eq!(h.val("C"), (i % 8 == 7) as u64);
        h.sim.step_clock().unwrap(); // falling
    }
}

// ---- 8-bit ripple-carry adder from full-adder subcircuits ------------------------

fn full_adder() -> Circuit {
    let mut b = Builder::new("full_adder");
    let a = b.comp(Kind::Input, "a");
    let bb = b.comp(Kind::Input, "b");
    let cin = b.comp(Kind::Input, "cin");
    let x1 = b.comp(Kind::Xor, "");
    let x2 = b.comp(Kind::Xor, "");
    let a1 = b.comp(Kind::And, "");
    let a2 = b.comp(Kind::And, "");
    let o = b.comp(Kind::Or, "");
    let s = b.comp(Kind::Output, "s");
    let cout = b.comp(Kind::Output, "cout");
    b.w(a, "out", x1, "in0");
    b.w(bb, "out", x1, "in1");
    b.w(x1, "out", x2, "in0");
    b.w(cin, "out", x2, "in1");
    b.w(a, "out", a1, "in0");
    b.w(bb, "out", a1, "in1");
    b.w(x1, "out", a2, "in0");
    b.w(cin, "out", a2, "in1");
    b.w(a1, "out", o, "in0");
    b.w(a2, "out", o, "in1");
    b.w(x2, "out", s, "in");
    b.w(o, "out", cout, "in");
    b.done()
}

fn ripple_carry_adder() -> Project {
    let mut b = Builder::new("rca8");
    let a = b.with(Kind::Input, "A", |p| p.width = 8);
    let bb = b.with(Kind::Input, "B", |p| p.width = 8);
    let cin = b.comp(Kind::Input, "Cin");
    let sa = b.with(Kind::Splitter, "", |p| p.width = 8);
    let sb = b.with(Kind::Splitter, "", |p| p.width = 8);
    let m = b.with(Kind::Merger, "", |p| p.width = 8);
    let s = b.with(Kind::Output, "S", |p| p.width = 8);
    let cout = b.comp(Kind::Output, "Cout");
    b.w(a, "out", sa, "in");
    b.w(bb, "out", sb, "in");
    let mut carry = (cin, "out".to_string());
    for i in 0..8 {
        let fa = b.sub("full_adder", &format!("fa{i}"));
        b.w(sa, &format!("out{i}"), fa, "a");
        b.w(sb, &format!("out{i}"), fa, "b");
        b.w(carry.0, &carry.1.clone(), fa, "cin");
        b.w(fa, "s", m, &format!("in{i}"));
        carry = (fa, "cout".into());
    }
    b.w(m, "out", s, "in");
    b.w(carry.0, &carry.1, cout, "in");
    Project { circuits: vec![b.done(), full_adder()], libraries: vec![] }
}

#[test]
fn ripple_carry_adder_8bit() {
    let p = ripple_carry_adder();
    for cfg in both_modes() {
        let mut h = Harness::new(&p, "rca8", cfg);
        let mut rng = 0x1234_5678u32;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng
        };
        let mut cases: Vec<(u64, u64, u64)> = vec![(0, 0, 0), (255, 1, 0), (255, 255, 1), (128, 128, 0)];
        cases.extend((0..300).map(|_| {
            let r = next();
            ((r & 0xff) as u64, (r >> 8 & 0xff) as u64, (r >> 16 & 1) as u64)
        }));
        for (a, b, c) in cases {
            h.set("A", a);
            h.set("B", b);
            h.set("Cin", c);
            h.settle();
            let sum = a + b + c;
            assert_eq!(h.val("S"), sum & 0xff, "{a}+{b}+{c}");
            assert_eq!(h.val("Cout"), sum >> 8, "{a}+{b}+{c}");
        }
        assert!(h.sim.netlist().net_by_name("fa7/cout").is_some() || h.sim.netlist().net_by_name("Cout").is_some());
    }
}

// ---- shift register from D flip-flops ---------------------------------------

#[test]
fn shift_register_from_flip_flops() {
    let mut b = Builder::new("shift");
    let sin = b.comp(Kind::Input, "SIN");
    let clk = b.with(Kind::Clock, "clk", |p| p.period = 10);
    let ffs: Vec<CompId> = (0..4).map(|i| b.comp(Kind::DFlipFlop, &format!("ff{i}"))).collect();
    let m = b.with(Kind::Merger, "", |p| p.width = 4);
    let out = b.with(Kind::Output, "Q", |p| p.width = 4);
    b.w(sin, "out", ffs[0], "d");
    for i in 0..4 {
        b.w(clk, "out", ffs[i], "clk");
        if i > 0 {
            b.w(ffs[i - 1], "q", ffs[i], "d");
        }
        b.w(ffs[i], "q", m, &format!("in{i}"));
    }
    b.w(m, "out", out, "in");
    let p = Project::new(b.done());
    for cfg in both_modes() {
        let mut h = Harness::new(&p, "shift", cfg);
        let bits = [1, 0, 1, 1, 0, 0, 1, 0];
        let mut model = 0u64;
        for bit in bits {
            h.set("SIN", bit);
            h.sim.step_clock().unwrap(); // rising: shift
            model = ((model << 1) | bit) & 0xf;
            h.sim.step_clock().unwrap(); // falling
            // ff0 holds the newest bit, i.e. bit 0 of the model.
            assert_eq!(h.val("Q"), model, "{cfg:?}");
        }
    }
}

#[test]
fn shift_register_component() {
    let mut b = Builder::new("sr");
    let sin = b.comp(Kind::Input, "SIN");
    let clk = b.with(Kind::Clock, "clk", |p| p.period = 4);
    let sr = b.with(Kind::ShiftRegister, "", |p| p.length = 8);
    let q = b.with(Kind::Output, "Q", |p| p.width = 8);
    b.w(sin, "out", sr, "sin");
    b.w(clk, "out", sr, "clk");
    b.w(sr, "q", q, "in");
    let mut h = Harness::new(&Project::new(b.done()), "sr", SimConfig::default());
    for bit in [1, 1, 0, 1] {
        h.set("SIN", bit);
        h.sim.step_clock().unwrap();
        h.sim.step_clock().unwrap();
    }
    assert_eq!(h.val("Q"), 0b1101);
}

// ---- small 4-bit ALU ----------------------------------------------------------

fn alu() -> Project {
    let mut b = Builder::new("alu");
    let w4 = |p: &mut Props| p.width = 4;
    let a = b.with(Kind::Input, "A", w4);
    let bb = b.with(Kind::Input, "B", w4);
    let op = b.with(Kind::Input, "OP", |p| p.width = 2);
    let add = b.with(Kind::Adder, "", w4);
    let sub = b.with(Kind::Subtractor, "", w4);
    let and = b.with(Kind::And, "", w4);
    let or = b.with(Kind::Or, "", w4);
    let mux = b.with(Kind::Mux, "", |p| {
        p.width = 4;
        p.select = 2;
    });
    let zero = b.with(Kind::Constant, "", w4);
    let cmp = b.with(Kind::Comparator, "", w4);
    let y = b.with(Kind::Output, "Y", w4);
    let z = b.comp(Kind::Output, "Z");
    for g in [add, sub] {
        b.w(a, "out", g, "a");
        b.w(bb, "out", g, "b");
    }
    for g in [and, or] {
        b.w(a, "out", g, "in0");
        b.w(bb, "out", g, "in1");
    }
    for (i, g) in [add, sub, and, or].into_iter().enumerate() {
        b.w(g, "out", mux, &format!("in{i}"));
    }
    b.w(op, "out", mux, "sel");
    b.w(mux, "out", y, "in");
    b.w(mux, "out", cmp, "a");
    b.w(zero, "out", cmp, "b");
    b.w(cmp, "eq", z, "in");
    Project::new(b.done())
}

#[test]
fn small_alu_exhaustive() {
    let mut h = Harness::new(&alu(), "alu", zero_delay());
    for op in 0..4u64 {
        for a in 0..16u64 {
            for b in 0..16u64 {
                h.set("A", a);
                h.set("B", b);
                h.set("OP", op);
                h.settle();
                let y = match op {
                    0 => a + b,
                    1 => a.wrapping_sub(b),
                    2 => a & b,
                    _ => a | b,
                } & 0xf;
                assert_eq!(h.val("Y"), y, "op {op} a {a} b {b}");
                assert_eq!(h.val("Z"), (y == 0) as u64);
            }
        }
    }
}

// ---- RAM -------------------------------------------------------------------------

#[test]
fn ram_write_then_read() {
    let mut b = Builder::new("mem");
    let addr = b.with(Kind::Input, "ADDR", |p| p.width = 4);
    let din = b.with(Kind::Input, "DIN", |p| p.width = 8);
    let we = b.comp(Kind::Input, "WE");
    let clk = b.comp(Kind::Input, "CLK");
    let ram = b.with(Kind::Ram, "ram", |p| {
        p.addr_width = 4;
        p.width = 8;
    });
    let dout = b.with(Kind::Output, "DOUT", |p| p.width = 8);
    b.w(addr, "out", ram, "addr");
    b.w(din, "out", ram, "din");
    b.w(we, "out", ram, "we");
    b.w(clk, "out", ram, "clk");
    b.w(ram, "dout", dout, "in");
    let mut h = Harness::new(&Project::new(b.done()), "mem", SimConfig::default());
    h.set("WE", 1);
    for i in 0..16 {
        h.set("ADDR", i);
        h.set("DIN", i * 3 + 1);
        h.settle();
        h.set("CLK", 1);
        h.settle();
        h.set("CLK", 0);
        h.settle();
    }
    h.set("WE", 0);
    for i in 0..16 {
        h.set("ADDR", i);
        h.settle();
        assert_eq!(h.val("DOUT"), i * 3 + 1);
    }
}

// ---- error cases -----------------------------------------------------------------

#[test]
fn not_loop_is_detected() {
    let mut b = Builder::new("osc");
    let en = b.with(Kind::Input, "EN", |p| p.value = 0);
    let nand = b.comp(Kind::Nand, "");
    b.w(en, "out", nand, "in0");
    b.w(nand, "out", nand, "in1");
    let p = Project::new(b.done());
    let nl = compile(&p, "osc").unwrap();
    let en_c = nl.comp_by_path("EN").unwrap();
    let mut sim = Simulator::new(nl, zero_delay());
    assert_eq!(sim.settle(10_000), Ok(true), "stable while disabled");
    sim.set_input(en_c, Signal::bool(true));
    let osc = sim.settle(10_000).unwrap_err();
    assert_eq!(osc.nets.len(), 1);
    assert!(sim.step_tick().is_err(), "stays stopped until reset");
    sim.reset();
    assert_eq!(sim.settle(10_000), Ok(true));
}

#[test]
fn driver_conflict_produces_x() {
    let mut b = Builder::new("c");
    let a = b.with(Kind::Constant, "", |p| p.value = 0);
    let one = b.with(Kind::Constant, "", |p| p.value = 1);
    let o = b.comp(Kind::Output, "O");
    b.w(a, "out", o, "in");
    b.w(one, "out", o, "in");
    let p = Project::new(b.done());
    let nl = compile(&p, "c").unwrap();
    assert!(!nl.diagnostics.is_empty(), "multiple drivers reported");
    let net = nl.net_by_name("O").unwrap();
    let mut sim = Simulator::new(nl, SimConfig::default());
    sim.settle(100).unwrap();
    assert_eq!(sim.net_value(net), Signal::x(1));
}

#[test]
fn tunnels_join_a_tristate_bus() {
    let mut b = Builder::new("bus");
    let byte = |p: &mut Props| p.width = 8;
    let a = b.with(Kind::Input, "A", byte);
    let bb = b.with(Kind::Input, "B", byte);
    let ea = b.comp(Kind::Input, "EA");
    let eb = b.comp(Kind::Input, "EB");
    let ta = b.with(Kind::TriBuffer, "", byte);
    let tb = b.with(Kind::TriBuffer, "", byte);
    let o = b.with(Kind::Output, "O", byte);
    b.w(a, "out", ta, "in");
    b.w(ea, "out", ta, "en");
    b.w(bb, "out", tb, "in");
    b.w(eb, "out", tb, "en");
    // Each driver and the reader reach the bus only through a tunnel.
    for (id, pin) in [(ta, "out"), (tb, "out"), (o, "in")] {
        let t = b.with(Kind::Tunnel, "data", byte);
        b.w(id, pin, t, "in");
    }
    let p = Project::new(b.done());
    for cfg in both_modes() {
        let mut h = Harness::new(&p, "bus", cfg);
        h.set("A", 0x12);
        h.set("B", 0x34);
        h.set("EA", 1);
        h.settle();
        assert_eq!(h.val("O"), 0x12);
        h.set("EA", 0);
        h.set("EB", 1);
        h.settle();
        assert_eq!(h.val("O"), 0x34);
        h.set("EB", 0);
        h.settle();
        assert_eq!(h.get("O"), Signal::z(8));
    }
}

#[test]
fn tunnels_are_local_to_their_instance() {
    // i -> tunnel x, tunnel x -> NOT -> o: an inverter, if the tunnels join.
    let inverter = |b: &mut Builder, input: CompId, pin: &str, output: CompId, label: &str| {
        let (ta, tb) = (b.comp(Kind::Tunnel, label), b.comp(Kind::Tunnel, label));
        let n = b.comp(Kind::Not, "");
        b.w(input, pin, ta, "in");
        b.w(tb, "in", n, "in");
        b.w(n, "out", output, "in");
    };
    let mut inner = Builder::new("inv");
    let (i, o) = (inner.comp(Kind::Input, "i"), inner.comp(Kind::Output, "o"));
    inverter(&mut inner, i, "out", o, "x");

    // Two instances and the top level all use tunnels named "x".
    let mut top = Builder::new("top");
    for (input, inst) in [("a", "s0"), ("b", "s1")] {
        let (inp, out) = (top.comp(Kind::Input, input), top.comp(Kind::Output, &format!("n{input}")));
        let s = top.sub("inv", inst);
        top.w(inp, "out", s, "i");
        top.w(s, "o", out, "in");
    }
    let (c, nc) = (top.comp(Kind::Input, "c"), top.comp(Kind::Output, "nc"));
    inverter(&mut top, c, "out", nc, "x");

    let p = Project { circuits: vec![top.done(), inner.done()], libraries: vec![] };
    let mut h = Harness::new(&p, "top", zero_delay());
    for v in 0..8u64 {
        for (k, n) in ["a", "b", "c"].into_iter().enumerate() {
            h.set(n, v >> k & 1);
        }
        h.settle();
        for (k, n) in ["na", "nb", "nc"].into_iter().enumerate() {
            assert_eq!(h.val(n), !v >> k & 1, "{n} for {v:03b}");
        }
    }
}

#[test]
fn subcircuit_recursion_rejected() {
    let mut a = Builder::new("a");
    a.sub("b", "");
    let mut b = Builder::new("b");
    b.sub("c", "");
    let mut c = Builder::new("c");
    c.sub("a", "");
    let p = Project { circuits: vec![a.done(), b.done(), c.done()], libraries: vec![] };
    match compile(&p, "a") {
        Err(ProjectError::Recursion(cycle)) => assert_eq!(cycle, ["a", "b", "c", "a"]),
        other => panic!("expected recursion error, got {:?}", other.map(|_| ())),
    }
}

// ---- persistence -------------------------------------------------------------------

#[test]
fn round_trip_preserves_behaviour() {
    let p = ripple_carry_adder();
    let text = lb_core::io::to_ron(&p);
    let back = lb_core::io::from_ron(&text, std::path::Path::new("rca.lbc")).unwrap();
    assert_eq!(back, p);
    let mut h = Harness::new(&back, "rca8", zero_delay());
    h.set("A", 200);
    h.set("B", 100);
    h.settle();
    assert_eq!(h.val("S"), 44);
    assert_eq!(h.val("Cout"), 1);
}

#[test]
fn compile_10k_components_fast() {
    // Chain of 10 000 gates (SPEC §5.1: full recompilation under 50 ms in
    // release builds; checked loosely here since tests may run unoptimized).
    let mut b = Builder::new("big");
    let mut prev = (b.comp(Kind::Input, "in"), "out".to_string());
    for i in 0..10_000 {
        let k = if i % 2 == 0 { Kind::Not } else { Kind::Buffer };
        let g = b.comp(k, "");
        b.w(prev.0, &prev.1.clone(), g, "in");
        prev = (g, "out".into());
    }
    let p = Project::new(b.done());
    let t = std::time::Instant::now();
    let nl = compile(&p, "big").unwrap();
    let dt = t.elapsed();
    assert_eq!(nl.comps.len(), 10_001);
    assert!(dt.as_millis() < 2_000, "compile took {dt:?}");
}

#[test]
fn keyboard_types_into_a_terminal() {
    // Keyboard → terminal: `avail` enables the write, both share the clock,
    // so each clock edge moves one character.
    let mut b = Builder::new("typing");
    let kb = b.with(Kind::Keyboard, "kb", |p| p.buffer = 4);
    let tty = b.with(Kind::Tty, "tty", |p| p.size = (8, 2));
    let clk = b.with(Kind::Clock, "clk", |p| p.period = 4);
    b.w(kb, "data", tty, "data");
    b.w(kb, "avail", tty, "write");
    b.w(clk, "out", kb, "clk");
    b.w(clk, "out", tty, "clk");
    let p = Project::new(b.done());
    let nl = compile(&p, "typing").unwrap();
    let mut sim = Simulator::new(nl, SimConfig::default());
    sim.settle(1000).unwrap();
    let (k, t) = (sim.netlist().comp_by_path("kb").unwrap(), sim.netlist().comp_by_path("tty").unwrap());
    assert_eq!(sim.type_keys(k, b"HI!\nXYZ"), 4, "the buffer holds 4 characters");
    for _ in 0..12 {
        sim.step_clock().unwrap();
    }
    let text: String = sim.state(t).mem[..16].iter().map(|&c| if c == 0 { '_' } else { c as u8 as char }).collect();
    assert_eq!(text, "HI!_____________");
    assert_eq!(sim.state(t).mem[16], 8, "cursor on the second row");
    assert_eq!(sim.state(k).mem[0], 0, "buffer emptied");
}
