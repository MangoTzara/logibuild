//! Automatic layout of imported circuits, signals flowing left to right.
//!
//! Components are assigned to columns by their longest distance from the
//! inputs (feedback through registers is ignored), ordered within each
//! column to reduce crossings, and stacked so that each component lines up
//! with the pins feeding it. Inputs are the first column and outputs the
//! last, both in port order, which makes it the pin order of the circuit
//! used as a subcircuit. Constants and tunnels with a single connection sit
//! right beside the pin they serve instead of taking a column. Wires are
//! left to the router.

use std::collections::{HashMap, HashSet};

use lb_core::circuit::{Circuit, Project};
use lb_core::component::{Kind, Layout, PinDir, Transform};

/// Free rows between components stacked in a column.
const ROW_GAP: i32 = 2;
/// Narrowest and widest space between columns, for the wires.
const MIN_GAP: i32 = 4;
const MAX_GAP: i32 = 40;
/// Length of the wire between an attached constant or tunnel and its pin.
const STUB: i32 = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    West,
    East,
}

/// A connection from an output (or a tunnel acting as source) to an input.
#[derive(Clone, Copy)]
struct Link {
    from: usize,
    from_pin: usize,
    to: usize,
    to_pin: usize,
}

/// Sets the position (and the rotation of tunnels) of every component of
/// `c`; `project` holds the circuits its subcircuits refer to.
pub fn place(project: &Project, c: &mut Circuit) {
    let n = c.components.len();
    if n == 0 {
        return;
    }
    let index: HashMap<u32, usize> = c.components.iter().enumerate().map(|(i, x)| (x.id, i)).collect();
    let layouts: Vec<Layout> = c.components.iter().map(|x| project.layout(x)).collect();
    let kinds: Vec<Kind> = c.components.iter().map(|x| x.kind).collect();
    let kind = |i: usize| kinds[i];

    let mut links = Vec::new();
    for w in &c.wires {
        let (Some(&a), Some(&b)) = (index.get(&w.from.0), index.get(&w.to.0)) else { continue };
        let (Some((pa, da)), Some((pb, db))) = (layouts[a].pin(&w.from.1), layouts[b].pin(&w.to.1)) else {
            continue;
        };
        let (pa, pb) = ((pa, da.dir), (pb, db.dir));
        let link = match (pa.1, pb.1) {
            (PinDir::Out, PinDir::In) => Link { from: a, from_pin: pa.0, to: b, to_pin: pb.0 },
            (PinDir::In, PinDir::Out) => Link { from: b, from_pin: pb.0, to: a, to_pin: pa.0 },
            // A tunnel at a reader drives it.
            (PinDir::In, PinDir::In) if kind(a) == Kind::Tunnel => {
                Link { from: a, from_pin: pa.0, to: b, to_pin: pb.0 }
            }
            (PinDir::In, PinDir::In) if kind(b) == Kind::Tunnel => {
                Link { from: b, from_pin: pb.0, to: a, to_pin: pa.0 }
            }
            _ => continue,
        };
        if link.from != link.to {
            links.push(link);
        }
    }

    // Tunnels driving a reader face it: pin on the right.
    for l in &links {
        if kind(l.from) == Kind::Tunnel {
            c.components[l.from].rot = 2;
        }
    }
    let transform = |c: &Circuit, i: usize| Transform::new(c.components[i].rot, false, &layouts[i]);
    let pin_at = |c: &Circuit, i: usize, pin: usize| transform(c, i).apply(layouts[i].pins[pin].pos);

    // Constants and tunnels with one connection to a pin on the left (or a
    // tunnel on an output on the right) sit beside that pin.
    let mut degree = vec![0; n];
    for l in &links {
        degree[l.from] += 1;
        degree[l.to] += 1;
    }
    let small = |i: usize| matches!(kind(i), Kind::Constant | Kind::Tunnel);
    let mut attached: Vec<Option<(usize, usize, Side)>> = vec![None; n];
    // Rows of the pins already given an attachment, by host and side.
    let mut taken: HashMap<(usize, bool), Vec<i32>> = HashMap::new();
    for l in &links {
        let (att, host, pin, side) = if small(l.from) && degree[l.from] == 1 && !small(l.to) {
            (l.from, l.to, l.to_pin, Side::West)
        } else if kind(l.to) == Kind::Tunnel && degree[l.to] == 1 && !small(l.from) {
            (l.to, l.from, l.from_pin, Side::East)
        } else {
            continue;
        };
        let p = layouts[host].pins[pin].pos;
        let facing = if side == Side::West { p.0 == 0 } else { p.0 == layouts[host].w };
        let rows = taken.entry((host, side == Side::West)).or_default();
        if facing && rows.iter().all(|y| (y - p.1).abs() >= 2) {
            rows.push(p.1);
            attached[att] = Some((host, pin, side));
        }
    }
    let is_node = |i: usize| attached[i].is_none();

    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for l in links.iter().filter(|l| is_node(l.from) && is_node(l.to)) {
        if !succs[l.from].contains(&l.to) {
            succs[l.from].push(l.to);
            preds[l.to].push(l.from);
        }
    }

    // Feedback edges, found by a depth-first search from the inputs.
    let mut back: HashSet<(usize, usize)> = HashSet::new();
    let mut state = vec![0u8; n];
    let mut starts: Vec<usize> = (0..n).filter(|&i| kind(i) == Kind::Input).collect();
    starts.extend((0..n).filter(|&i| kind(i) != Kind::Input));
    for s in starts {
        if state[s] != 0 || !is_node(s) {
            continue;
        }
        let mut stack = vec![(s, 0usize)];
        state[s] = 1;
        while let Some(top) = stack.last_mut() {
            let (v, k) = *top;
            if k < succs[v].len() {
                top.1 += 1;
                let u = succs[v][k];
                match state[u] {
                    0 => {
                        state[u] = 1;
                        stack.push((u, 0));
                    }
                    1 => {
                        back.insert((v, u));
                    }
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    let forward = |v: usize, u: usize| !back.contains(&(v, u));

    // Longest path from the sources, in topological order.
    let mut layer = vec![0usize; n];
    let mut indeg: Vec<usize> = (0..n).map(|v| preds[v].iter().filter(|&&u| forward(u, v)).count()).collect();
    let mut queue: Vec<usize> = (0..n).filter(|&v| is_node(v) && indeg[v] == 0).collect();
    let mut qi = 0;
    while qi < queue.len() {
        let v = queue[qi];
        qi += 1;
        for &u in &succs[v] {
            if forward(v, u) {
                layer[u] = layer[u].max(layer[v] + 1);
                indeg[u] -= 1;
                if indeg[u] == 0 {
                    queue.push(u);
                }
            }
        }
    }
    // Other sources (constants, tunnels, ...) go just before their readers.
    for v in (0..n).filter(|&v| is_node(v) && kind(v) != Kind::Input) {
        if preds[v].iter().all(|&u| !forward(u, v)) && !succs[v].is_empty() {
            let first = succs[v].iter().map(|&u| layer[u]).min().unwrap();
            layer[v] = first.saturating_sub(1);
        }
    }
    let last = (0..n).filter(|&v| is_node(v) && kind(v) != Kind::Output).map(|v| layer[v]).max().unwrap_or(0) + 1;
    for v in (0..n).filter(|&v| kind(v) == Kind::Output) {
        layer[v] = last;
    }
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); last + 1];
    for v in (0..n).filter(|&v| is_node(v)) {
        layers[layer[v]].push(v);
    }

    // Barycentre ordering; the inputs and outputs keep the port order.
    let mut rank = vec![0.0f64; n];
    let set_rank = |layers: &[Vec<usize>], rank: &mut Vec<f64>| {
        for l in layers {
            for (k, &v) in l.iter().enumerate() {
                rank[v] = (k as f64 + 0.5) / l.len() as f64;
            }
        }
    };
    set_rank(&layers, &mut rank);
    for _ in 0..4 {
        for downward in [true, false] {
            let order: Vec<usize> = if downward { (1..last).collect() } else { (1..last).rev().collect() };
            for l in order {
                let keys: HashMap<usize, f64> = layers[l]
                    .iter()
                    .map(|&v| {
                        let near = if downward { &preds[v] } else { &succs[v] };
                        let key = if near.is_empty() {
                            rank[v]
                        } else {
                            near.iter().map(|&u| rank[u]).sum::<f64>() / near.len() as f64
                        };
                        (v, key)
                    })
                    .collect();
                layers[l].sort_by(|a, b| keys[a].total_cmp(&keys[b]));
                set_rank(&layers, &mut rank);
            }
        }
    }

    // Columns: wide enough for the components, the attachments on both
    // sides and the wires running between them.
    let size = |c: &Circuit, i: usize| transform(c, i).size();
    let mut col_w = vec![0; last + 1];
    let mut att_west = vec![0; last + 1];
    let mut att_east = vec![0; last + 1];
    for v in 0..n {
        match attached[v] {
            None => col_w[layer[v]] = col_w[layer[v]].max(size(c, v).0),
            Some((host, _, side)) => {
                let w = size(c, v).0 + STUB;
                let slot = if side == Side::West { &mut att_west[layer[host]] } else { &mut att_east[layer[host]] };
                *slot = (*slot).max(w);
            }
        }
    }
    let mut tracks = vec![HashSet::new(); last + 1];
    for l in links.iter().filter(|l| is_node(l.from) && is_node(l.to)) {
        let (a, b) = (layer[l.from].min(layer[l.to]), layer[l.from].max(layer[l.to]));
        for t in &mut tracks[a..b] {
            t.insert((l.from, l.from_pin));
        }
    }
    let mut x = vec![att_west[0]; last + 1];
    for l in 0..last {
        let room = att_east[l] + att_west[l + 1] + 2;
        let gap = (room + tracks[l].len() as i32 / 2).clamp(MIN_GAP.max(room), MAX_GAP.max(room));
        x[l + 1] = x[l] + col_w[l] + gap;
    }

    // Rows: each component lines up with the pins feeding it, below the
    // previous one of its column.
    let mut incoming: Vec<Vec<Link>> = vec![Vec::new(); n];
    for l in &links {
        incoming[l.to].push(*l);
    }
    let mut y = vec![0i32; n];
    let mut placed = vec![false; n];
    for column in &layers {
        let mut cursor = 0;
        for &v in column {
            let wants: Vec<i32> = incoming[v]
                .iter()
                .filter(|k| placed[k.from] && is_node(k.from))
                .map(|k| y[k.from] + pin_at(c, k.from, k.from_pin).1 - pin_at(c, v, k.to_pin).1)
                .collect();
            let want = if wants.is_empty() { cursor } else { wants.iter().sum::<i32>() / wants.len() as i32 };
            y[v] = want.max(cursor);
            placed[v] = true;
            cursor = y[v] + size(c, v).1 + ROW_GAP;
        }
    }
    for v in (0..n).filter(|&v| is_node(v)) {
        c.components[v].pos = (x[layer[v]], y[v]);
    }
    for (v, att) in attached.iter().enumerate() {
        let Some((host, pin, side)) = *att else { continue };
        let hp = pin_at(c, host, pin);
        let (px, py) = (c.components[host].pos.0 + hp.0, c.components[host].pos.1 + hp.1);
        let own = pin_at(c, v, 0);
        c.components[v].pos = match side {
            Side::West => (px - STUB - own.0, py - own.1),
            Side::East => (px + STUB - own.0, py - own.1),
        };
    }
}
