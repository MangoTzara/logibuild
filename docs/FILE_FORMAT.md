# The `.lbc` file format

LogiBuild projects are UTF-8 text files in [RON](https://github.com/ron-rs/ron)
(Rusty Object Notation) with the `.lbc` extension. The format is designed for version control:

* components are sorted by `id`, wires are sorted, one per line;
* ids are small integers assigned deterministically (the next id is the largest plus one);
* default properties are omitted;
* nothing volatile is stored (no timestamps, no absolute paths, no simulation state).

## Top level

```ron
(
    version: 1,
    name: "half_adder",
    components: [
        (id: 1, kind: Input, pos: (0, 0), props: (width: 1, label: "A")),
        (id: 2, kind: Input, pos: (0, 4), props: (width: 1, label: "B")),
        (id: 3, kind: Xor, pos: (6, 0), props: (inputs: 2)),
        (id: 4, kind: And, pos: (6, 4), props: (inputs: 2)),
        (id: 5, kind: Output, pos: (12, 1), props: (width: 1, label: "SUM")),
        (id: 6, kind: Output, pos: (12, 5), props: (width: 1, label: "CARRY")),
    ],
    wires: [
        (from: (1, "out"), to: (3, "in0")),
        (from: (1, "out"), to: (4, "in0")),
        (from: (2, "out"), to: (3, "in1")),
        (from: (2, "out"), to: (4, "in1")),
        (from: (3, "out"), to: (5, "in")),
        (from: (4, "out"), to: (6, "in")),
    ],
    subcircuits: [
        (
            name: "full_adder",
            components: [ ... ],
            wires: [ ... ],
        ),
    ],
    libraries: ["lib"],
)
```

| Field | Required | Meaning |
| --- | --- | --- |
| `version` | yes | Format version, currently `1`. Newer versions are refused with an explicit error. |
| `name` | yes | Name of the main circuit. |
| `components`, `wires` | no | The main circuit. |
| `subcircuits` | no | Circuits embedded in the file, sorted by name. |
| `libraries` | no | Folders (relative to the file) searched for missing subcircuits: a circuit named `x` is loaded from `<folder>/x.lbc`, recursively. Circuits loaded this way are not embedded when saving. |
| `hdl` | no | HDL sources and the entities synthesized from them, see below. |

## HDL

```ron
    hdl: (sources: ["hdl/alu.vhd", "hdl/uart.v"], tops: ["cpu"], vhdl_std: V93),
```

| Field | Meaning |
| --- | --- |
| `sources` | VHDL (`.vhd`, `.vhdl`) and Verilog (`.v`, `.sv`) files, relative to the project file (absolute when outside its folder). |
| `tops` | Entities or modules synthesized into circuits; the entities they instantiate become circuits too. |
| `vhdl_std` | `V93` or `V08` (default). |

A circuit synthesized from the sources (an entry of `subcircuits`) carries
`hdl: (digest: "…")`, the fingerprint of the sources it was made from: the editor keeps it
read-only and shows it out of date when the sources no longer match. Removing the field makes
it an ordinary circuit.

## Components

```ron
(id: 7, kind: DFlipFlop, pos: (20, 4), rot: 1, mirror: true, props: (label: "ff0", edge: Falling))
```

| Field | Meaning |
| --- | --- |
| `id` | Unique within the circuit. |
| `kind` | Component type, see [COMPONENTS.md](COMPONENTS.md). |
| `pos` | Top-left corner of the (rotated) bounding box, in grid units. |
| `rot` | Clockwise quarter turns, 0–3 (omitted when 0). |
| `mirror` | Flip top/bottom before rotating (omitted when false). |
| `props` | Properties; omitted fields take the default for the kind. |

Properties:

| Field | Type | Used by | Default |
| --- | --- | --- | --- |
| `width` | 1–64 | data width of most components | 1 |
| `inputs` | 2–8 | gates | 2 |
| `select` | 1–5 | multiplexer, demultiplexer, decoder, priority encoder | 1 |
| `label` | string | any (also names nets and subcircuit terminals) | `""` |
| `value` | integer | constant value, initial value of an input | 0 |
| `period` | ticks ≥ 2 | clock | 100 |
| `duty` | 1–99 % | clock | 50 |
| `edge` | `Rising`, `Falling` | flip-flops, registers, counters, RAM, keyboard, text terminal, pixel screen | `Rising` |
| `signed` | bool | comparator, bit extender | false |
| `out_width` | 1–64 | bit extender | 16 |
| `shift` | `Lsl`, `Lsr`, `Asr`, `RotLeft`, `RotRight` | shifter | `Lsl` |
| `pieces` | list of widths | splitter, merger (LSB first) | one bit each |
| `addr_width` | 1–20 | ROM, RAM | 8 |
| `length` | 1–64 | shift register stages | 4 |
| `buffer` | 1–4096 | keyboard: characters it holds | 32 |
| `data` | string | ROM/RAM contents (hex words, see below) | empty |
| `data_file` | path | ROM/RAM contents in a separate `.hex` file, relative to the project | — |
| `circuit` | string | subcircuit: name of the instantiated circuit | — |
| `radix` | `Bin`, `Hex`, `Dec`, `Signed` | display of inputs, outputs, probes, number displays | `Hex` |
| `size` | `(w, h)` | box annotation (grid units), LED matrix and text terminal (columns, rows), pixel screen (pixels) | `(10, 6)`, `(8, 8)`, `(32, 8)`, `(32, 32)` |
| `digits` | 0–32 | number display (0 = as many as the width needs), 7-segment display (1–8) | 0 |
| `colors` | `Mono`, `Rgb332`, `Rgb888` | pixel screen: 1, 8 or 24-bit colours | `Mono` |
| `delay` | ticks | propagation delay override (0 = simulator default) | 0 |

## Wires

```ron
(from: (3, "out"), to: (5, "in"), path: [(10, 4), (10, 8)])
```

`from` and `to` are `(component id, pin name)`. The optional `path` lists waypoints in grid
units; without it the editor routes the wire automatically. All pins joined by wires form a
net. A wire to a pin that does not exist is reported as a warning and ignored.

## Memory images (`.hex`)

Whitespace-separated hexadecimal words. `N*value` repeats a word `N` times, `#` starts a
comment, and an optional first line `v2.0 raw` makes the files compatible with Logisim
images. When saving, trailing zero words are dropped and runs of four or more equal words
are compressed. The same syntax is used inline in the `data` property.

```text
v2.0 raw
3f 6 5b 4f 66 6d 7d 7
7f 6f 77 7c 39 5e 79 71
4*0 ff
```

## Saving

Files are written to `<name>.lbc.tmp`, flushed to disk and renamed over the original, so a
crash never leaves a half-written project; the previous version is kept as
`<name>.lbc.bak`. Memory contents with a `data_file` are written to that file.
