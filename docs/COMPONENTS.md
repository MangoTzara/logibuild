# Component reference

Every component of LogiBuild: what it does and its pins. The **?** button in the properties panel
shows the section of the selected component, read from this file: edit it here. A test checks that
every component has a section, and that its pins match the code.

Pins are listed facing east, as placed: inputs on the left, outputs on the right. **Bits** gives the
width of each pin; *Width* means the *Width (bits)* property. Unconnected control inputs (`en`,
`rst`, `pre`, `load`, carry and borrow in) take their inactive value; unconnected gate inputs are
ignored.

## Inputs & outputs

### Input / Switch

A value set by hand: click a 1-bit input on the canvas to toggle it; wider ones are set in the
properties panel. In a subcircuit, each input is a terminal of its box.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `out` | out | Width | The value set |

### Button

A push button: 1 while held down with the mouse, 0 otherwise.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `out` | out | 1 | 1 while pressed |

### Constant

A fixed value.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `out` | out | Width | The value |

### Clock

A square wave: *Period* ticks per cycle, high for *Duty cycle* percent of them.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `out` | out | 1 | The clock |

### Output / Indicator

Shows the value of its input. In a subcircuit, each output is a terminal of its box.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in` | in | Width | The value shown |

### LED

A light: on when its input is 1.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in` | in | 1 | On at 1 |

### Probe

A small tag showing the value of the net it touches, for debugging.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in` | in | Width | The value shown |

### Tunnel

Connects nets without a wire: every tunnel with the same label in a circuit is joined, as if wired.
The pin is passive, so the net can have one driver or several tri-state ones. Tunnels do not cross
subcircuit boundaries (use Input and Output for that). A tunnel without a label, or the only one
with its label, is reported as a warning.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in` | in | Width | The net joined to the other tunnels |

### Keyboard

Select it on the canvas and type: the keys wait in a buffer of *Buffer size* characters (up to
4096; Enter is line feed 10, Backspace is 8, Del deletes the component).

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `read` | in | 1 | Consume the oldest character on the clock edge (1 when unconnected) |
| `clk` | in | 1 | Clock |
| `clear` | in | 1 | Empty the buffer at once |
| `data` | out | 7 | The oldest character (ASCII), 0 when empty |
| `avail` | out | 1 | 1 while a character is waiting |

## Displays

### Number display

Shows its input on 7-segment digits, in the chosen radix. *Digits* `Auto` fits the whole range of
the width; with fewer digits the lowest ones are shown. Hex and binary are padded with zeros,
decimal with blanks; an unknown value shows dashes.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in` | in | Width | The number shown |

### 7-segment display

Segments driven directly. With one digit each segment has its own pin; with more digits each digit
has an 8-bit input `dig0`, `dig1`, … under it (`dig0` is the rightmost; bit 0 is `a`, bit 6 is `g`,
bit 7 is `dp`).

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `a` … `g` | in | 1 | Segments a (top) to g (middle), clockwise from the top |
| `dp` | in | 1 | Decimal point |

### LED matrix

A grid of LEDs, *Size* columns by rows. Each row has an input, its most significant bit on the
left.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `row0` … `row7` | in | Columns | The LEDs of each row, top to bottom |

### Text terminal

A text screen. On the active clock edge with `write` at 1, writes the character on `data` at the
cursor: printable characters (32-126) advance the cursor, line feed (10) and carriage return (13)
start a new line, backspace (8) erases the previous character, form feed (12) clears the screen.
Text scrolls up past the last row.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `data` | in | 7 | Character to write (ASCII) |
| `write` | in | 1 | Write on the clock edge |
| `clk` | in | 1 | Clock |
| `clear` | in | 1 | Empty the screen at once |

### Pixel screen

A screen of pixels. On the active clock edge with `write` at 1, sets pixel (`x`, `y`) to `color`;
pixels outside the screen are ignored.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `x` | in | log₂ columns | Column, 0 on the left |
| `y` | in | log₂ rows | Row, 0 at the top |
| `color` | in | 1, 8 or 24 | On/off, `RRRGGGBB` or `RRGGBB` (*Colors*) |
| `write` | in | 1 | Write on the clock edge |
| `clk` | in | 1 | Clock |
| `clear` | in | 1 | Blank the screen at once |

## Gates

### AND

1 when all inputs are 1. Wider than 1 bit, works bit by bit.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in0`, `in1`, … | in | Width | Inputs, as many as *Inputs* (2 to 8) |
| `out` | out | Width | AND of the inputs |

### OR

1 when at least one input is 1. Wider than 1 bit, works bit by bit.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in0`, `in1`, … | in | Width | Inputs, as many as *Inputs* (2 to 8) |
| `out` | out | Width | OR of the inputs |

### NAND

0 when all inputs are 1: an inverted AND. Wider than 1 bit, works bit by bit.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in0`, `in1`, … | in | Width | Inputs, as many as *Inputs* (2 to 8) |
| `out` | out | Width | NAND of the inputs |

### NOR

1 when all inputs are 0: an inverted OR. Wider than 1 bit, works bit by bit.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in0`, `in1`, … | in | Width | Inputs, as many as *Inputs* (2 to 8) |
| `out` | out | Width | NOR of the inputs |

### XOR

1 when an odd number of inputs are 1. Wider than 1 bit, works bit by bit.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in0`, `in1`, … | in | Width | Inputs, as many as *Inputs* (2 to 8) |
| `out` | out | Width | XOR of the inputs |

### XNOR

1 when an even number of inputs are 1: an inverted XOR. Wider than 1 bit, works bit by bit.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in0`, `in1`, … | in | Width | Inputs, as many as *Inputs* (2 to 8) |
| `out` | out | Width | XNOR of the inputs |

### NOT

Inverts its input, bit by bit.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in` | in | Width | Input |
| `out` | out | Width | Inverted input |

### Buffer

Copies its input to its output, after its delay.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in` | in | Width | Input |
| `out` | out | Width | The same value |

### Tri-state buffer

Copies its input while enabled, otherwise leaves its output floating (Z), so that several of them
can drive one net.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in` | in | Width | Input |
| `en` | in | 1 | 1: drive the output; 0: float |
| `out` | out | Width | The input, or Z |

## Combinational

### Multiplexer

Copies the input chosen by `sel` to the output.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in0`, `in1`, … | in | Width | Data inputs, 2^*Select bits* of them |
| `sel` | in | Select bits | Number of the input to copy |
| `out` | out | Width | The chosen input |

### Demultiplexer

Copies the input to the output chosen by `sel`; the other outputs are 0.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in` | in | Width | Data input |
| `sel` | in | Select bits | Number of the output |
| `out0`, `out1`, … | out | Width | Outputs, 2^*Select bits* of them |

### Decoder

Sets to 1 the output chosen by `sel`, the others to 0.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `sel` | in | Select bits | Number of the output |
| `out0`, `out1`, … | out | 1 | Outputs, 2^*Select bits* of them |

### Priority encoder

Gives the number of the highest input at 1.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in0`, `in1`, … | in | 1 | Inputs, 2^*Select bits* of them |
| `out` | out | Select bits | Number of the highest input at 1, 0 when none |
| `v` | out | 1 | 1 when some input is 1 |

### Adder

Adds two numbers and a carry.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `a` | in | Width | First number |
| `b` | in | Width | Second number |
| `cin` | in | 1 | Carry in (0 when unconnected) |
| `out` | out | Width | `a` + `b` + `cin` |
| `cout` | out | 1 | Carry out |

### Subtractor

Subtracts two numbers and a borrow.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `a` | in | Width | First number |
| `b` | in | Width | Number subtracted |
| `bin` | in | 1 | Borrow in (0 when unconnected) |
| `out` | out | Width | `a` − `b` − `bin` |
| `bout` | out | 1 | Borrow out: 1 when the result is negative |

### Comparator

Compares two numbers, unsigned or in two's complement (*Signed*).

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `a` | in | Width | First number |
| `b` | in | Width | Second number |
| `lt` | out | 1 | `a` < `b` |
| `eq` | out | 1 | `a` = `b` |
| `gt` | out | 1 | `a` > `b` |

### Shifter

Shifts or rotates its input by `sh` bits: logical left or right, arithmetic right (keeping the
sign), rotate left or right (*Shift type*).

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in` | in | Width | Value |
| `sh` | in | log₂ Width | Number of bits |
| `out` | out | Width | Shifted value |

### Splitter

Splits a bus into pieces of *Split* bits; `out0` gets the lowest bits.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in` | in | Width | The bus |
| `out0`, `out1`, … | out | Split | The pieces, lowest first |

### Merger

Joins pieces of *Split* bits into a bus; `in0` gives the lowest bits.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in0`, `in1`, … | in | Split | The pieces, lowest first |
| `out` | out | Width | The bus |

### Bit extender

Widens (or cuts) a value to *Output width* bits, filling with zeros or, when *Signed*, with copies
of the sign bit.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `in` | in | Width | Value |
| `out` | out | Output width | Extended value |

## Sequential

### SR latch

Set and reset: `s` sets the output to 1, `r` to 0, and with both at 0 it keeps its value. With
both at 1 both outputs are 0.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `s` | in | 1 | Set |
| `r` | in | 1 | Reset |
| `q` | out | 1 | Stored value |
| `nq` | out | 1 | Inverted value |

### D latch

Transparent while enabled: `q` follows `d` while `en` is 1 and keeps its value while it is 0.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `d` | in | 1 | Data |
| `en` | in | 1 | Enable |
| `q` | out | 1 | Stored value |
| `nq` | out | 1 | Inverted value |

### D flip-flop

Stores `d` on the active clock edge.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `d` | in | 1 | Data |
| `clk` | in | 1 | Clock |
| `pre` | in | 1 | Set to 1 at once |
| `rst` | in | 1 | Set to 0 at once (wins over `pre`) |
| `q` | out | 1 | Stored value |
| `nq` | out | 1 | Inverted value |

### T flip-flop

Toggles on the active clock edge while `t` is 1.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `t` | in | 1 | Toggle |
| `clk` | in | 1 | Clock |
| `pre` | in | 1 | Set to 1 at once |
| `rst` | in | 1 | Set to 0 at once (wins over `pre`) |
| `q` | out | 1 | Stored value |
| `nq` | out | 1 | Inverted value |

### JK flip-flop

On the active clock edge: `j` sets, `k` resets, both toggle, neither keeps the value.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `j` | in | 1 | Set |
| `k` | in | 1 | Reset |
| `clk` | in | 1 | Clock |
| `pre` | in | 1 | Set to 1 at once |
| `rst` | in | 1 | Set to 0 at once (wins over `pre`) |
| `q` | out | 1 | Stored value |
| `nq` | out | 1 | Inverted value |

### Register

Stores `d` on the active clock edge while `en` is 1.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `d` | in | Width | Data |
| `en` | in | 1 | Enable (1 when unconnected) |
| `clk` | in | 1 | Clock |
| `rst` | in | 1 | Set to 0 at once |
| `q` | out | Width | Stored value |

### Counter

Counts up by one on the active clock edge while `en` is 1, wrapping around; `load` loads `d`
instead.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `d` | in | Width | Value to load |
| `load` | in | 1 | Load `d` on the clock edge (wins over `en`) |
| `en` | in | 1 | Count (1 when unconnected) |
| `clk` | in | 1 | Clock |
| `rst` | in | 1 | Set to 0 at once |
| `q` | out | Width | Count |
| `carry` | out | 1 | 1 at the highest count |

### Shift register

On the active clock edge while `en` is 1, shifts its bits up by one and takes `sin` as bit 0.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `sin` | in | 1 | Serial input |
| `en` | in | 1 | Shift (1 when unconnected) |
| `clk` | in | 1 | Clock |
| `rst` | in | 1 | Set to 0 at once |
| `q` | out | Stages | All the stages, `sin` entering at bit 0 |
| `sout` | out | 1 | Serial output: the highest bit |

## Memory

### ROM

Read-only memory: `data` is the word at `addr`. The contents are hex words, kept in the project or
in an external .hex file.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `addr` | in | Address bits | Address |
| `data` | out | Width | The word at `addr` |

### RAM

Read-write memory: `dout` is the word at `addr`, and `din` is written there on the active clock
edge while `we` is 1.

| Pin | Dir | Bits | Function |
| --- | --- | --- | --- |
| `addr` | in | Address bits | Address |
| `din` | in | Width | Word to write |
| `we` | in | 1 | Write on the clock edge |
| `clk` | in | 1 | Clock |
| `dout` | out | Width | The word at `addr` |

## Text & annotations

### Subcircuit

Another circuit of the project used as a component. Its pins are the Input (left) and Output
(right) components of that circuit, named after their labels. Double-click it to look inside with
the live values.

### Text

A text note on the canvas; it does not take part in the simulation.

### Box

A dashed frame to group parts of a circuit, with its label in the top left corner; it does not
take part in the simulation.
