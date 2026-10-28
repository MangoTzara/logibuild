//! Netlists as Yosys writes them, imported and simulated: the circuits must
//! behave like the HDL they come from.

use lb_core::circuit::{CompGeom, Project};
use lb_core::prelude::*;
use lb_hdl::import::{ImportError, Imported, import, readable_name};
use lb_hdl::json::Design;
use serde_json::{Value, json};

fn port(dir: &str, bits: Value) -> Value {
    json!({ "direction": dir, "bits": bits })
}

fn cell(ty: &str, params: Value, conns: Value) -> Value {
    json!({ "hide_name": 1, "type": ty, "parameters": params, "attributes": {}, "connections": conns })
}

fn design(modules: Value) -> Design {
    Design::parse(&json!({ "creator": "test", "modules": modules }).to_string()).unwrap()
}

fn bits(nets: std::ops::Range<i64>) -> Value {
    Value::Array(nets.map(|n| json!(n)).collect())
}

fn load(d: &Design, top: &str) -> Imported {
    import(d, &[top.to_string()], "digest").unwrap()
}

fn project(i: &Imported) -> Project {
    Project { circuits: i.circuits.clone(), ..Default::default() }
}

/// A running simulation of a circuit, addressed by input and output labels.
struct Harness {
    sim: Simulator,
}

impl Harness {
    fn new(p: &Project, root: &str) -> Self {
        let nl = compile(p, root).expect("compiles");
        assert!(nl.diagnostics.is_empty(), "{:?}", nl.diagnostics);
        let mut h = Harness { sim: Simulator::new(nl, SimConfig::default()) };
        h.settle();
        h
    }

    fn set(&mut self, input: &str, v: u64) {
        let c = self.sim.netlist().comp_by_path(input).unwrap_or_else(|| panic!("no input {input}"));
        let w = self.sim.input_value(c).width();
        self.sim.set_input(c, Signal::new(w, v));
        self.settle();
    }

    fn settle(&mut self) {
        assert!(self.sim.settle(100_000).expect("no oscillation"), "did not settle");
    }

    fn val(&self, net: &str) -> u64 {
        let n = self.sim.netlist().net_by_name(net).unwrap_or_else(|| panic!("no net {net}"));
        let v = self.sim.net_value(n);
        v.to_u64().unwrap_or_else(|| panic!("{net} is {v:?}"))
    }

    fn clock(&mut self, clk: &str) {
        self.set(clk, 1);
        self.set(clk, 0);
    }
}

fn count(c: &Circuit, kind: Kind) -> usize {
    c.components.iter().filter(|x| x.kind == kind).count()
}

/// Every component inside the circuit's bounding boxes overlaps no other.
fn assert_no_overlap(p: &Project, c: &Circuit) {
    let boxes: Vec<(u32, (i32, i32, i32, i32))> =
        c.components.iter().map(|x| (x.id, CompGeom::new(p, x).bounds())).collect();
    for (i, (ia, a)) in boxes.iter().enumerate() {
        for (ib, b) in &boxes[i + 1..] {
            let overlap = a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3;
            assert!(!overlap, "{} components {ia} {a:?} and {ib} {b:?} overlap", c.name);
        }
    }
}

/// 4-bit counter with enable and an asynchronous reset to 3:
/// `if rst then q <= 3 elsif rising_edge(clk) then if en then q <= q + 1`.
fn counter() -> Design {
    design(json!({
        "counter": {
            "ports": {
                "clk": port("input", json!([2])),
                "rst": port("input", json!([3])),
                "en": port("input", json!([4])),
                "q": port("output", bits(5..9)),
            },
            "cells": {
                "$procdff$1": cell("$adff",
                    json!({"WIDTH": 4, "CLK_POLARITY": 1, "ARST_POLARITY": 1, "ARST_VALUE": "0011"}),
                    json!({"CLK": [2], "ARST": [3], "D": bits(9..13), "Q": bits(5..9)})),
                "$add$1": cell("$add",
                    json!({"A_SIGNED": 0, "B_SIGNED": 0, "A_WIDTH": 4, "B_WIDTH": 1, "Y_WIDTH": 4}),
                    json!({"A": bits(5..9), "B": ["1"], "Y": bits(13..17)})),
                "$mux$1": cell("$mux", json!({"WIDTH": 4}),
                    json!({"A": bits(5..9), "B": bits(13..17), "S": [4], "Y": bits(9..13)})),
            },
            "netnames": {
                "q": {"hide_name": 0, "bits": bits(5..9)},
            }
        }
    }))
}

#[test]
fn counter_counts_and_resets_to_its_value() {
    let i = load(&counter(), "counter");
    let c = &i.circuits[0];
    assert_eq!(c.name, "counter");
    assert_eq!(c.hdl.as_ref().unwrap().digest, "digest");
    assert_eq!((count(c, Kind::Register), count(c, Kind::Adder), count(c, Kind::Mux)), (1, 1, 1));
    let reg = c.components.iter().find(|x| x.kind == Kind::Register).unwrap();
    assert_eq!((reg.props.width(), reg.props.label.as_str()), (4, "q"), "named after its output");
    let p = project(&i);
    assert_no_overlap(&p, c);

    let mut h = Harness::new(&p, "counter");
    h.set("rst", 1);
    assert_eq!(h.val("q"), 3, "asynchronous reset to a non-zero value");
    h.set("rst", 0);
    h.set("en", 1);
    for expect in [4, 5, 6] {
        h.clock("clk");
        assert_eq!(h.val("q"), expect);
    }
    h.set("en", 0);
    h.clock("clk");
    assert_eq!(h.val("q"), 6, "holds while disabled");
    h.set("en", 1);
    for _ in 0..10 {
        h.clock("clk");
    }
    assert_eq!(h.val("q"), 0, "wraps around");
}

/// `y = op==0 ? a+b : op==1 ? a-b : op==2 ? a&b : a^b`, `zero = y == 0`,
/// `low_ones = a[3:0] == 4'hF`, `sh = a << b[3:0]`, `sar = $signed(a) >>> 2`.
fn alu() -> Design {
    design(json!({
        "alu": {
            "ports": {
                "a": port("input", bits(2..10)),
                "b": port("input", bits(10..18)),
                "op": port("input", json!([18, 19])),
                "y": port("output", bits(20..28)),
                "zero": port("output", json!([28])),
                "low_ones": port("output", json!([29])),
                "sh": port("output", bits(70..78)),
                "sar": port("output", bits(80..88)),
            },
            "cells": {
                "add": cell("$add", json!({"A_WIDTH": 8, "B_WIDTH": 8, "Y_WIDTH": 8}),
                    json!({"A": bits(2..10), "B": bits(10..18), "Y": bits(30..38)})),
                "sub": cell("$sub", json!({"A_WIDTH": 8, "B_WIDTH": 8, "Y_WIDTH": 8}),
                    json!({"A": bits(2..10), "B": bits(10..18), "Y": bits(40..48)})),
                "and": cell("$and", json!({"A_WIDTH": 8, "B_WIDTH": 8, "Y_WIDTH": 8}),
                    json!({"A": bits(2..10), "B": bits(10..18), "Y": bits(50..58)})),
                "xor": cell("$xor", json!({"A_WIDTH": 8, "B_WIDTH": 8, "Y_WIDTH": 8}),
                    json!({"A": bits(2..10), "B": bits(10..18), "Y": bits(60..68)})),
                "sel": cell("$bmux", json!({"WIDTH": 8, "S_WIDTH": 2}),
                    json!({"A": Value::Array([bits(30..38), bits(40..48), bits(50..58), bits(60..68)].iter()
                        .flat_map(|v| v.as_array().unwrap().clone()).collect()),
                        "S": [18, 19], "Y": bits(20..28)})),
                "z": cell("$logic_not", json!({"A_WIDTH": 8, "Y_WIDTH": 1}),
                    json!({"A": bits(20..28), "Y": [28]})),
                "eq": cell("$eq", json!({"A_WIDTH": 4, "B_WIDTH": 4, "Y_WIDTH": 1}),
                    json!({"A": bits(2..6), "B": ["1", "1", "1", "1"], "Y": [29]})),
                "shl": cell("$shl", json!({"A_WIDTH": 8, "B_WIDTH": 4, "Y_WIDTH": 8}),
                    json!({"A": bits(2..10), "B": bits(10..14), "Y": bits(70..78)})),
                "sshr": cell("$sshr", json!({"A_SIGNED": 1, "A_WIDTH": 8, "B_WIDTH": 2, "Y_WIDTH": 8}),
                    json!({"A": bits(2..10), "B": ["0", "1"], "Y": bits(80..88)})),
            },
            "netnames": {}
        }
    }))
}

#[test]
fn alu_computes() {
    let i = load(&alu(), "alu");
    let c = &i.circuits[0];
    assert_eq!(count(c, Kind::Mux), 2, "one 4-way multiplexer, one for shifts out of range");
    assert!(count(c, Kind::Splitter) >= 1, "a[3:0] and b[3:0] are slices");
    let p = project(&i);
    assert_no_overlap(&p, c);
    let mut h = Harness::new(&p, "alu");
    let cases = [(0, 200, 100, 44), (1, 5, 7, 254), (2, 0b1100, 0b1010, 0b1000), (3, 0b1100, 0b1010, 0b0110)];
    for (op, a, b, y) in cases {
        h.set("op", op);
        h.set("a", a);
        h.set("b", b);
        assert_eq!(h.val("y"), y, "op {op}: {a}, {b}");
    }
    h.set("op", 3);
    h.set("a", 9);
    h.set("b", 9);
    assert_eq!((h.val("y"), h.val("zero")), (0, 1));
    h.set("a", 0x3f);
    assert_eq!(h.val("low_ones"), 1);
    h.set("a", 0x3e);
    assert_eq!(h.val("low_ones"), 0);
    h.set("a", 1);
    for (shift, out) in [(3, 8), (7, 128), (8, 0), (15, 0)] {
        h.set("b", shift);
        assert_eq!(h.val("sh"), out, "1 << {shift}");
    }
    h.set("a", 0x80);
    assert_eq!(h.val("sar"), 0xe0, "arithmetic shift keeps the sign");
}

/// `inv` instantiated seven times, every instance reading `x`.
fn hierarchy() -> Design {
    let mut cells = serde_json::Map::new();
    for k in 0..7 {
        let mut c = cell("inv", json!({}), json!({"a": [2], "y": [10 + k]}));
        c["hide_name"] = json!(0);
        cells.insert(format!("u{k}"), c);
    }
    design(json!({
        "inv": {
            "ports": {"a": port("input", json!([2])), "y": port("output", json!([3]))},
            "cells": {"$not$1": cell("$not", json!({"A_WIDTH": 1, "Y_WIDTH": 1}), json!({"A": [2], "Y": [3]}))},
            "netnames": {}
        },
        "top": {
            "ports": {"x": port("input", json!([2])), "y": port("output", bits(10..17))},
            "cells": Value::Object(cells),
            "netnames": {}
        }
    }))
}

#[test]
fn hierarchy_becomes_subcircuits() {
    let i = load(&hierarchy(), "top");
    let names: Vec<&str> = i.circuits.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["inv", "top"], "children first");
    let top = &i.circuits[1];
    assert_eq!(count(top, Kind::Subcircuit), 7);
    assert!(top.components.iter().any(|c| c.kind == Kind::Subcircuit && c.props.label == "u3"));
    assert_eq!(count(top, Kind::Tunnel), 8, "x reaches seven readers through tunnels");
    let p = project(&i);
    assert_no_overlap(&p, top);
    let mut h = Harness::new(&p, "top");
    h.set("x", 0);
    assert_eq!(h.val("y"), 0x7f);
    h.set("x", 1);
    assert_eq!(h.val("y"), 0);
}

fn init(words: &[u64], width: usize) -> String {
    words.iter().rev().map(|w| format!("{w:0width$b}")).collect()
}

/// A ROM (constant table) and a RAM written on the rising edge.
fn memories() -> Design {
    design(json!({
        "mem": {
            "ports": {
                "addr": port("input", bits(2..4)),
                "din": port("input", bits(4..12)),
                "we": port("input", json!([12])),
                "clk": port("input", json!([13])),
                "rom": port("output", bits(20..28)),
                "ram": port("output", bits(30..38)),
            },
            "cells": {
                "table": cell("$mem_v2", json!({
                    "MEMID": "\\table", "SIZE": 4, "ABITS": 2, "WIDTH": 8, "OFFSET": 0,
                    "INIT": init(&[0x11, 0x22, 0x33, 0x44], 8),
                    "RD_PORTS": 1, "WR_PORTS": 0, "RD_CLK_ENABLE": "0", "RD_CLK_POLARITY": "1",
                    "RD_WIDE_CONTINUATION": "0", "WR_WIDE_CONTINUATION": "0"}),
                    json!({"RD_ADDR": bits(2..4), "RD_DATA": bits(20..28), "RD_CLK": ["x"], "RD_EN": ["1"],
                        "RD_ARST": ["0"], "RD_SRST": ["0"]})),
                "store": cell("$mem_v2", json!({
                    "MEMID": "\\store", "SIZE": 4, "ABITS": 2, "WIDTH": 8, "OFFSET": 0, "INIT": "x",
                    "RD_PORTS": 1, "WR_PORTS": 1, "RD_CLK_ENABLE": "0", "RD_CLK_POLARITY": "1",
                    "WR_CLK_ENABLE": "1", "WR_CLK_POLARITY": "1",
                    "RD_WIDE_CONTINUATION": "0", "WR_WIDE_CONTINUATION": "0"}),
                    json!({"RD_ADDR": bits(2..4), "RD_DATA": bits(30..38), "RD_CLK": ["x"], "RD_EN": ["1"],
                        "RD_ARST": ["0"], "RD_SRST": ["0"],
                        "WR_ADDR": bits(2..4), "WR_DATA": bits(4..12), "WR_EN": [12, 12, 12, 12, 12, 12, 12, 12],
                        "WR_CLK": [13]})),
            },
            "netnames": {}
        }
    }))
}

#[test]
fn memories_become_rom_and_ram() {
    let i = load(&memories(), "mem");
    let c = &i.circuits[0];
    let rom = c.components.iter().find(|x| x.kind == Kind::Rom).unwrap();
    assert_eq!((rom.props.addr_width(), rom.props.width(), rom.props.label.as_str()), (2, 8, "table"));
    assert_eq!(count(c, Kind::Ram), 1);
    let p = project(&i);
    let mut h = Harness::new(&p, "mem");
    for (a, v) in [(0, 0x11), (3, 0x44)] {
        h.set("addr", a);
        assert_eq!(h.val("rom"), v);
    }
    h.set("addr", 2);
    h.set("din", 0xab);
    h.set("we", 1);
    h.clock("clk");
    h.set("we", 0);
    h.set("din", 0);
    h.clock("clk");
    assert_eq!(h.val("ram"), 0xab);
    h.set("addr", 1);
    assert_eq!(h.val("ram"), 0);
}

/// The netlist Yosys writes for `examples/hdl/ram16x8.v` (with `memory -nomap`).
#[test]
fn yosys_ram_becomes_one_ram_component() {
    let d = Design::parse(include_str!("fixtures/ram16x8.json")).unwrap();
    let i = load(&d, "ram16x8");
    let c = &i.circuits[0];
    assert_eq!(count(c, Kind::Ram), 1);
    assert_eq!(count(c, Kind::Mux), 0);
    let p = project(&i);
    let mut h = Harness::new(&p, "ram16x8");
    h.set("addr", 5);
    h.set("din", 0x5a);
    h.set("we", 1);
    h.clock("clk");
    h.set("we", 0);
    h.set("din", 0);
    h.clock("clk");
    assert_eq!(h.val("dout"), 0x5a);
    h.set("addr", 4);
    assert_eq!(h.val("dout"), 0);
}

#[test]
fn register_files_need_mapping() {
    let d = design(json!({
        "rf": {
            "ports": {"ra": port("input", json!([2])), "rb": port("input", json!([3]))},
            "cells": {
                "regs": cell("$mem_v2", json!({"MEMID": "\\regs", "SIZE": 2, "ABITS": 1, "WIDTH": 1,
                    "RD_PORTS": 2, "WR_PORTS": 1}), json!({"RD_ADDR": [2, 3], "RD_DATA": [4, 5]})),
            },
            "netnames": {}
        }
    }));
    match import(&d, &[], "") {
        Err(ImportError::NeedsMemoryMap(why)) => assert!(why.contains("regs") && why.contains("2 read"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn unsupported_constructs_are_named() {
    let d = design(json!({
        "m": {
            "ports": {"a": port("input", bits(2..70)), "io": port("inout", json!([80]))},
            "cells": {"x": cell("$dffsr", json!({"WIDTH": 1}), json!({}))},
            "netnames": {}
        }
    }));
    let Err(ImportError::Unsupported(msg)) = import(&d, &[], "") else { panic!() };
    for part in ["$dffsr", "wider than 64", "bidirectional port “io”"] {
        assert!(msg.contains(part), "{msg}");
    }
    let d = design(json!({"m": {"ports": {}, "cells": {}, "netnames": {}}}));
    assert!(matches!(import(&d, &["nope".into()], ""), Err(ImportError::Unsupported(m)) if m.contains("nope")));
}

#[test]
fn layout_keeps_port_order() {
    let i = load(&counter(), "counter");
    let c = &i.circuits[0];
    let p = project(&i);
    let ys = |kind: Kind| -> Vec<(String, i32)> {
        let mut v: Vec<(String, i32)> =
            c.components.iter().filter(|x| x.kind == kind).map(|x| (x.props.label.clone(), x.pos.1)).collect();
        v.sort_by_key(|x| x.1);
        v
    };
    let ins: Vec<String> = ys(Kind::Input).into_iter().map(|x| x.0).collect();
    assert_eq!(ins, ["clk", "rst", "en"]);
    let x_in = c.components.iter().filter(|x| x.kind == Kind::Input).map(|x| x.pos.0).max().unwrap();
    let x_out = c.components.iter().filter(|x| x.kind == Kind::Output).map(|x| x.pos.0).min().unwrap();
    assert!(x_in < x_out, "inputs on the left, outputs on the right");
    // Used as a subcircuit, the pins follow the ports.
    let (pins_in, _) = lb_core::component::layout::terminals(c);
    assert_eq!(pins_in.iter().map(|t| t.0.as_str()).collect::<Vec<_>>(), ["clk", "rst", "en"]);
    assert_no_overlap(&p, c);
}

#[test]
fn parameterized_names_are_readable() {
    assert_eq!(readable_name("$paramod\\counter\\WIDTH=s32'00000000000000000000000000001000"), "counter WIDTH=8");
    assert_eq!(readable_name("$paramod\\fifo\\DEPTH=16\\NAME=\"x\""), "fifo DEPTH=16 NAME=x");
    assert_eq!(readable_name("\\top"), "top");
}
