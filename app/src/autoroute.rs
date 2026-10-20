//! Automatic wire routing (A* on the grid).
//!
//! Each wire is routed from the stub of its `from` pin to the stub of its
//! `to` pin with an A* search over grid points:
//!
//! * component bodies (closed bounding boxes) are obstacles;
//! * running along a segment of another net is forbidden, and so are turning
//!   on, or passing through a corner or end of, another net — anything that
//!   would make two nets look connected;
//! * crossing another net costs a little, bends cost more;
//! * reusing a segment of the *same* net is cheap, so fan-outs share trunks
//!   and get junction dots.
//!
//! The point in front of every pin is reserved for the pin's net, so that no
//! other wire can pass in front of a pin. Wires are routed one at a time,
//! shortest first; when that leaves crossings or wires without a clean
//! route, the wires involved and their neighbours are ripped up and routed
//! again in other orders and with other bend preferences (bundles of
//! parallel wires, e.g. into a bus, then come out as a staircase), and the
//! best layout is kept.
//!
//! Routes are memoized per wire (keyed by its end positions and directions)
//! and kept as long as they remain valid, so editing a large circuit only
//! reroutes the wires that are affected. User waypoints (`Wire::path`) are
//! honoured with straight L segments.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use lb_core::circuit::{Circuit, CompId, Geometry, PinRef, Wire};

use crate::routing::{self, P, Routes, WireRoute};

/// Dense occupancy map of the circuit area.
#[derive(Clone)]
struct Board {
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    /// Inside a component body.
    blocked: Vec<bool>,
    /// Net (+1) using the unit edge to the right / below each point; 0 = free.
    hedge: Vec<u32>,
    vedge: Vec<u32>,
    /// Net (+1) using each point; 0 = free.
    pnet: Vec<u32>,
    /// A corner or an end of a route lies on the point.
    pvertex: Vec<bool>,
    /// Net (+1) whose pin stub is the point; 0 = none.
    stub: Vec<u32>,
}

impl Board {
    fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let n = (w * h) as usize;
        Board {
            x0,
            y0,
            w,
            h,
            blocked: vec![false; n],
            hedge: vec![0; n],
            vedge: vec![0; n],
            pnet: vec![0; n],
            pvertex: vec![false; n],
            stub: vec![0; n],
        }
    }

    fn idx(&self, p: P) -> Option<usize> {
        let (x, y) = (p.0 - self.x0, p.1 - self.y0);
        (x >= 0 && y >= 0 && x < self.w && y < self.h).then(|| (y * self.w + x) as usize)
    }

    fn is_blocked(&self, p: P) -> bool {
        self.idx(p).is_some_and(|i| self.blocked[i])
    }

    /// Slot of the unit edge `a`–`b`: (horizontal?, index of its first point).
    fn edge_slot(&self, a: P, b: P) -> Option<(bool, usize)> {
        let first = if a <= b { a } else { b };
        Some((a.1 == b.1, self.idx(first)?))
    }

    fn edge_net(&self, a: P, b: P) -> Option<u32> {
        let (h, i) = self.edge_slot(a, b)?;
        let n = if h { self.hedge[i] } else { self.vedge[i] };
        (n != 0).then(|| n - 1)
    }

    fn point(&self, p: P) -> Option<(u32, bool)> {
        let i = self.idx(p)?;
        (self.pnet[i] != 0).then(|| (self.pnet[i] - 1, self.pvertex[i]))
    }

    /// Whether `pts` (a simplified polyline of `net`) fits without
    /// conflicting with other nets or crossing component bodies.
    fn fits(&self, net: u32, pts: &[P]) -> bool {
        let last = *pts.last().unwrap_or(&(0, 0));
        for seg in pts.windows(2) {
            let mut ok = true;
            walk(seg[0], seg[1], |a, b| {
                if self.edge_net(a, b).is_some_and(|m| m != net) || (b != last && self.is_blocked(b)) {
                    ok = false;
                }
            });
            if !ok {
                return false;
            }
        }
        pts.iter().all(|&p| self.point(p).is_none_or(|(n, _)| n == net))
    }

    /// Reserves the stub of a pin of `net`: the edge from the pin `a` to the
    /// point `b` in front of it. Other nets may cross `b`, but not end, turn
    /// or run there.
    fn reserve(&mut self, net: u32, a: P, b: P) {
        if let Some((h, i)) = self.edge_slot(a, b) {
            let slot = if h { &mut self.hedge[i] } else { &mut self.vedge[i] };
            if *slot == 0 {
                *slot = net + 1;
            }
        }
        if let Some(i) = self.idx(b)
            && self.stub[i] == 0
        {
            self.stub[i] = net + 1;
        }
    }

    /// Whether `p` is the stub of a pin of another net than `net`.
    fn foreign_stub(&self, net: u32, p: P) -> bool {
        self.idx(p).is_some_and(|i| self.stub[i] != 0 && self.stub[i] != net + 1)
    }

    /// Points of `pts` (a polyline of `net`) that lie on other nets.
    fn crossings(&self, net: u32, pts: &[P]) -> u32 {
        let mut n = 0;
        for seg in pts.windows(2) {
            walk(seg[0], seg[1], |_, q| {
                if self.point(q).is_some_and(|(m, _)| m != net) {
                    n += 1;
                }
            });
        }
        n
    }

    fn insert(&mut self, net: u32, pts: &[P]) {
        for seg in pts.windows(2) {
            walk(seg[0], seg[1], |a, b| {
                if let Some((h, i)) = self.edge_slot(a, b) {
                    let slot = if h { &mut self.hedge[i] } else { &mut self.vedge[i] };
                    if *slot == 0 {
                        *slot = net + 1;
                    }
                }
                for p in [a, b] {
                    if let Some(i) = self.idx(p)
                        && self.pnet[i] == 0
                    {
                        self.pnet[i] = net + 1;
                    }
                }
            });
        }
        // A simplified polyline only has corners and ends as points.
        for &p in pts {
            if let Some(i) = self.idx(p) {
                if self.pnet[i] == 0 {
                    self.pnet[i] = net + 1;
                }
                if self.pnet[i] == net + 1 {
                    self.pvertex[i] = true;
                }
            }
        }
    }
}

/// Calls `f` for every unit step from `a` to `b` (axis-aligned).
fn walk(a: P, b: P, mut f: impl FnMut(P, P)) {
    let (dx, dy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
    let mut p = a;
    while p != b {
        let q = (p.0 + dx, p.1 + dy);
        f(p, q);
        p = q;
    }
}

const DIRS: [P; 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
/// Costs are scaled so that the bend preference (below one step) can break
/// ties between routes of equal length without ever lengthening a route.
const SCALE: u32 = 64;
const STEP: u32 = 10 * SCALE;
const SHARED_STEP: u32 = 3 * SCALE;
const BEND: u32 = 40 * SCALE;
const CROSS: u32 = 25 * SCALE;
/// Largest bend preference penalty, per bend (two bends stay below a step).
const BIAS_MAX: i32 = (STEP as i32 - 1) / 2;

/// Where a route prefers to bend, among routes of equal cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bias {
    None,
    /// Near the start.
    Early,
    /// Near the goal.
    Late,
}
/// Heuristic weight (> 1: faster, slightly less optimal searches).
const GREED: u32 = 3;
const MAX_EXPANSIONS: usize = 200_000;
pub static ALLOC_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static LOOP_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static STATES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static SEARCHES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static FAILED_BUDGET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static FAILED_EXHAUST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static FAILED_EXP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static EXPANSIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A* from `start` (heading `dir`) to `goal`, entered heading `goal_dir`,
/// within `margin` of the two points. Returns the visited points.
#[allow(clippy::too_many_arguments)]
fn search(board: &Board, net: u32, start: P, dir: P, goal: P, goal_dir: P, margin: i32, bias: Bias) -> Option<Vec<P>> {
    let rx0 = start.0.min(goal.0) - margin;
    let ry0 = start.1.min(goal.1) - margin;
    let rw = (start.0.max(goal.0) + margin) - rx0 + 1;
    let rh = (start.1.max(goal.1) + margin) - ry0 + 1;
    let states = (rw * rh * 4) as usize;
    let sidx = |p: P, d: usize| -> usize { (((p.1 - ry0) * rw + (p.0 - rx0)) * 4) as usize + d };
    let h = |p: P| ((p.0 - goal.0).abs() + (p.1 - goal.1).abs()) as u32 * STEP * GREED / 2;
    let dir_index = |d: P| DIRS.iter().position(|x| *x == d).unwrap_or(0);

    let __t = std::time::Instant::now();
    let mut best = vec![u32::MAX; states];
    let mut parent = vec![u32::MAX; states];
    ALLOC_NS.fetch_add(__t.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
    STATES.fetch_add(states as u64, std::sync::atomic::Ordering::Relaxed);
    SEARCHES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let __t2 = std::time::Instant::now();
    struct G(std::time::Instant);
    impl Drop for G {
        fn drop(&mut self) {
            LOOP_NS.fetch_add(self.0.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let _g = G(__t2);
    let mut heap = BinaryHeap::new();
    let mut budget = MAX_EXPANSIONS;
    let s = sidx(start, dir_index(dir));
    best[s] = 0;
    heap.push(Reverse((h(start), 0u32, start, dir_index(dir))));
    while let Some(Reverse((_, g, p, di))) = heap.pop() {
        let si = sidx(p, di);
        if best[si] < g {
            continue;
        }
        if p == goal {
            let mut path = vec![p];
            let mut cur = si;
            while parent[cur] != u32::MAX {
                cur = parent[cur] as usize;
                let cell = (cur / 4) as i32;
                path.push((rx0 + cell % rw, ry0 + cell / rw));
            }
            path.reverse();
            return Some(path);
        }
        budget -= 1;
        EXPANSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if budget == 0 {
            FAILED_BUDGET.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return None;
        }
        let other_here = board.point(p).is_some_and(|(n, _)| n != net) || board.foreign_stub(net, p);
        for (ndi, d) in DIRS.iter().enumerate() {
            if ndi != di && DIRS[di] == (-d.0, -d.1) {
                continue; // no U-turns
            }
            let turning = ndi != di;
            // Never turn where another net passes: it would look joined.
            if turning && other_here {
                continue;
            }
            let q = (p.0 + d.0, p.1 + d.1);
            if q.0 < rx0 || q.1 < ry0 || q.0 >= rx0 + rw || q.1 >= ry0 + rh {
                continue;
            }
            if q != goal && board.is_blocked(q) {
                continue;
            }
            let mut cost = g;
            match board.edge_net(p, q) {
                Some(n) if n != net => continue,
                Some(_) => cost += SHARED_STEP,
                None => cost += STEP,
            }
            if let Some((n, vertex)) = board.point(q)
                && n != net
            {
                // Passing over a corner or end of another net, or stopping
                // on its line, is not allowed; crossing straight is.
                if vertex || q == goal {
                    continue;
                }
                cost += CROSS;
            }
            if board.foreign_stub(net, q) {
                // Crossing in front of another pin: its wire will cross here.
                if q == goal {
                    continue;
                }
                cost += CROSS;
            }
            if turning {
                cost += BEND;
                let from = match bias {
                    Bias::None => None,
                    Bias::Early => Some(start),
                    Bias::Late => Some(goal),
                };
                if let Some(o) = from {
                    cost += ((p.0 - o.0).abs() + (p.1 - o.1).abs()).min(BIAS_MAX) as u32;
                }
            }
            if q == goal && *d != goal_dir {
                cost += BEND;
            }
            let ni = sidx(q, ndi);
            if cost < best[ni] {
                best[ni] = cost;
                parent[ni] = si as u32;
                heap.push(Reverse((cost + h(q), cost, q, ndi)));
            }
        }
    }
    FAILED_EXHAUST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    FAILED_EXP.fetch_add((MAX_EXPANSIONS - budget) as u64, std::sync::atomic::Ordering::Relaxed);
    None
}

/// Memo of routes from the previous computation, with whether each one was
/// a fallback (kept as is: rerouting would not find anything better).
#[derive(Default, Clone)]
pub struct Memo {
    routes: HashMap<Key, (Vec<P>, bool)>,
}

type Key = (Wire, P, P, P, P);

/// Groups wires into nets (union-find over their pins).
fn net_ids(wires: &[Wire]) -> Vec<u32> {
    let mut ids: HashMap<&PinRef, usize> = HashMap::new();
    let mut parent: Vec<usize> = Vec::new();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut first = Vec::with_capacity(wires.len());
    for w in wires {
        let mut idx = [0usize; 2];
        for (k, p) in [&w.from, &w.to].into_iter().enumerate() {
            let n = ids.len();
            let id = *ids.entry(p).or_insert(n);
            if id == parent.len() {
                parent.push(id);
            }
            idx[k] = id;
        }
        let (ra, rb) = (find(&mut parent, idx[0]), find(&mut parent, idx[1]));
        if ra != rb {
            parent[ra.max(rb)] = ra.min(rb);
        }
        first.push(idx[0]);
    }
    first.into_iter().map(|i| find(&mut parent, i) as u32).collect()
}

/// Routes every wire of `circuit`.
pub fn route_all(geom: &Geometry, circuit: &Circuit, memo: &mut Memo) -> Routes {
    let __t = std::time::Instant::now();
    let wires = &circuit.wires;
    let nets = net_ids(wires);
    let no_offset = |_: CompId| (0, 0);
    let ends: Vec<_> = wires
        .iter()
        .map(|w| (routing::pin_info(geom, &w.from, &no_offset), routing::pin_info(geom, &w.to, &no_offset)))
        .collect();

    // Board covering components, pins and waypoints, with a margin.
    let (mut x0, mut y0, mut x1, mut y1) = (0, 0, 0, 0);
    let mut grow = |x: i32, y: i32| {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    };
    for g in geom.items.values() {
        let b = g.bounds();
        grow(b.0, b.1);
        grow(b.2, b.3);
    }
    for w in wires {
        for p in &w.path {
            grow(p.0, p.1);
        }
    }
    let mut empty = Board::new(x0 - MARGIN_MAX, y0 - MARGIN_MAX, x1 + MARGIN_MAX, y1 + MARGIN_MAX);
    for c in &circuit.components {
        if c.kind.is_annotation() {
            continue;
        }
        if let Some(g) = geom.items.get(&c.id) {
            let (bx0, by0, bx1, by1) = g.bounds();
            for y in by0..=by1 {
                for x in bx0..=bx1 {
                    if let Some(i) = empty.idx((x, y)) {
                        empty.blocked[i] = true;
                    }
                }
            }
        }
    }
    // Reserve the pin stubs (the pin and the grid point in front of it) for
    // their net: other wires may cross them, but not end, turn or run there,
    // so a wire routed earlier cannot take the way in of a later one.
    for (i, e) in ends.iter().enumerate() {
        for (p, d, _) in [e.0, e.1].into_iter().flatten() {
            empty.reserve(nets[i], p, (p.0 + d.0, p.1 + d.1));
        }
    }
    let job = Job { wires, nets: &nets, ends: &ends };
    eprintln!("  board setup {:?}", __t.elapsed());

    let mut board = empty.clone();
    let mut points: Vec<Slot> = vec![None; wires.len()];
    // First keep the memoized routes that are still valid.
    for (i, w) in wires.iter().enumerate() {
        let (Some((a, da, _)), Some((b, db, _))) = ends[i] else { continue };
        let key: Key = (w.clone(), a, da, b, db);
        if let Some((pts, fallback)) = memo.routes.get(&key)
            && pts.first() == Some(&a)
            && pts.last() == Some(&b)
            && (*fallback || board.fits(nets[i], pts))
        {
            if !fallback {
                board.insert(nets[i], pts);
            }
            points[i] = Some((pts.clone(), *fallback));
        }
    }
    eprintln!("  memo reuse done {:?}", __t.elapsed());
    // Then route the others, shortest first: short local wires must not be
    // pushed aside by long ones.
    let fresh: Vec<usize> =
        (0..wires.len()).filter(|&i| points[i].is_none() && ends[i].0.is_some() && ends[i].1.is_some()).collect();
    let bad = job.route_set(&mut board, &fresh, Order::Shortest, Bias::None, &mut points);
    eprintln!("  route_set done {:?} (fresh {}, bad {})", __t.elapsed(), fresh.len(), bad.len());
    {
        use std::sync::atomic::Ordering::Relaxed;
        eprintln!(
            "  failed budget {} exhausted {} (exp {})",
            FAILED_BUDGET.swap(0, Relaxed),
            FAILED_EXHAUST.swap(0, Relaxed),
            FAILED_EXP.swap(0, Relaxed)
        );
        eprintln!(
            "  searches {} states {} expansions {} alloc {:?} loop {:?}",
            SEARCHES.swap(0, Relaxed),
            STATES.swap(0, Relaxed),
            EXPANSIONS.swap(0, Relaxed),
            std::time::Duration::from_nanos(ALLOC_NS.swap(0, Relaxed)),
            std::time::Duration::from_nanos(LOOP_NS.swap(0, Relaxed))
        );
    }
    if !bad.is_empty() {
        job.improve(&empty, &bad, &mut points);
    }
    eprintln!("  improve done {:?}", __t.elapsed());

    let mut routes = Routes::default();
    let mut new_memo = HashMap::new();
    for (i, w) in wires.iter().enumerate() {
        let (from, pts) = match ends[i] {
            (Some((a, da, ia)), Some((b, db, ib))) => {
                routes.connected.insert((w.from.0, ia));
                routes.connected.insert((w.to.0, ib));
                let (pts, fallback) = points[i].take().unwrap_or_default();
                new_memo.insert((w.clone(), a, da, b, db), (pts.clone(), fallback));
                (Some((w.from.0, ia)), pts)
            }
            _ => (None, Vec::new()),
        };
        routes.wires.push(WireRoute { points: pts, from });
    }
    memo.routes = new_memo;
    eprintln!("  memo rebuilt {:?}", __t.elapsed());
    routes.dots = routing::junctions(wires, &routes.wires);
    eprintln!("  junctions {:?}", __t.elapsed());
    routes
}

/// Order in which a set of wires is routed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    Shortest,
    Longest,
}

/// Largest set of wires ripped up and routed again to remove crossings.
const RIP_UP_MAX: usize = 64;

/// Quality of a layout, lower is better: wires without a clean route,
/// crossings, bends, length.
type Score = (u32, u32, u32, u32);

/// The wires of a circuit with their nets and ends.
struct Job<'a> {
    wires: &'a [Wire],
    nets: &'a [u32],
    ends: &'a [Ends],
}

/// Route of a wire, if computed, and whether it is a fallback.
type Slot = Option<(Vec<P>, bool)>;

type End = Option<(P, P, usize)>;
type Ends = (End, End);

impl Job<'_> {
    fn length(&self, i: usize) -> i32 {
        match self.ends[i] {
            (Some((a, ..)), Some((b, ..))) => (a.0 - b.0).abs() + (a.1 - b.1).abs(),
            _ => 0,
        }
    }

    /// Routes `set` on `board` in `order`, storing the routes in `points`;
    /// returns the wires that cross another net or have no clean route.
    fn route_set(&self, board: &mut Board, set: &[usize], order: Order, bias: Bias, points: &mut [Slot]) -> Vec<usize> {
        let mut set = set.to_vec();
        match order {
            Order::Shortest => set.sort_by_key(|&i| (self.length(i), i)),
            Order::Longest => set.sort_by_key(|&i| (Reverse(self.length(i)), i)),
        }
        let mut bad = Vec::new();
        for i in set {
            let w = &self.wires[i];
            let (Some((a, da, _)), Some((b, db, _))) = self.ends[i] else { continue };
            let routed = if w.path.is_empty() {
                route_one(board, self.nets[i], a, da, b, db, bias)
            } else {
                (routing::route_through(a, &w.path, b), false)
            };
            // Fallback routes ignore the rules: they must not constrain others.
            if routed.1 {
                bad.push(i);
            } else {
                if board.crossings(self.nets[i], &routed.0) > 0 {
                    bad.push(i);
                }
                board.insert(self.nets[i], &routed.0);
            }
            points[i] = Some(routed);
        }
        bad
    }

    /// Score of the routes of `set`, laid over `board`.
    fn score(&self, board: &Board, set: &[usize], points: &[Slot]) -> Score {
        let mut board = board.clone();
        let mut score = (0, 0, 0, 0);
        for &i in set {
            let Some((pts, fallback)) = &points[i] else { continue };
            if *fallback {
                score.0 += 1;
                continue;
            }
            score.1 += board.crossings(self.nets[i], pts);
            score.2 += pts.len().saturating_sub(2) as u32;
            score.3 += pts.windows(2).map(|s| (s[0].0 - s[1].0).abs() + (s[0].1 - s[1].1).abs()).sum::<i32>() as u32;
            board.insert(self.nets[i], pts);
        }
        score
    }

    /// Rips up the `bad` wires and the wires around them, and routes them
    /// again in every order and bend preference; keeps the best layout.
    fn improve(&self, empty: &Board, bad: &[usize], points: &mut [Slot]) {
        let bbox = |i: usize, points: &[Slot]| -> Option<(P, P)> {
            let mut pts: Vec<P> = points[i].as_ref().map(|r| r.0.clone()).unwrap_or_default();
            for (p, ..) in [self.ends[i].0, self.ends[i].1].into_iter().flatten() {
                pts.push(p);
            }
            let x0 = pts.iter().map(|p| p.0).min()?;
            let y0 = pts.iter().map(|p| p.1).min()?;
            let x1 = pts.iter().map(|p| p.0).max()?;
            let y1 = pts.iter().map(|p| p.1).max()?;
            Some(((x0, y0), (x1, y1)))
        };
        // The wires around the bad ones: everything that meets their area,
        // grown until it covers whole bundles.
        let routable: Vec<(usize, (P, P))> = (0..self.wires.len())
            .filter(|&i| self.ends[i].0.is_some() && self.ends[i].1.is_some())
            .filter_map(|i| Some((i, bbox(i, points)?)))
            .collect();
        let union = |boxes: &mut dyn Iterator<Item = (P, P)>| {
            boxes.reduce(|a, b| ((a.0.0.min(b.0.0), a.0.1.min(b.0.1)), (a.1.0.max(b.1.0), a.1.1.max(b.1.1))))
        };
        let Some(mut area) = union(&mut bad.iter().filter_map(|&i| bbox(i, points))) else { return };
        let set = loop {
            let (lo, hi) = area;
            let set: Vec<(usize, (P, P))> = routable
                .iter()
                .copied()
                .filter(|(_, b)| b.0.0 <= hi.0 + 1 && b.1.0 >= lo.0 - 1 && b.0.1 <= hi.1 + 1 && b.1.1 >= lo.1 - 1)
                .collect();
            if set.len() > RIP_UP_MAX {
                return;
            }
            let grown = union(&mut set.iter().map(|s| s.1)).unwrap_or(area);
            if grown == area {
                break set.into_iter().map(|s| s.0).collect::<Vec<usize>>();
            }
            area = grown;
        };

        // Everything else stays where it is.
        let mut base = empty.clone();
        for (i, r) in points.iter().enumerate() {
            if let Some((pts, false)) = r
                && !set.contains(&i)
            {
                base.insert(self.nets[i], pts);
            }
        }
        let mut best = self.score(&base, &set, points);
        let mut best_points: Option<Vec<Slot>> = None;
        for order in [Order::Shortest, Order::Longest] {
            for bias in [Bias::None, Bias::Early, Bias::Late] {
                let mut trial = points.to_vec();
                let mut board = base.clone();
                self.route_set(&mut board, &set, order, bias, &mut trial);
                let score = self.score(&base, &set, &trial);
                if score < best {
                    best = score;
                    best_points = Some(trial);
                }
                if best.0 == 0 && best.1 == 0 {
                    break;
                }
            }
        }
        if let Some(p) = best_points {
            points.clone_from_slice(&p);
        }
    }
}

const MARGIN_MAX: i32 = 24;

/// Routes one wire; the flag tells whether the plain fallback was used.
fn route_one(board: &Board, net: u32, a: P, da: P, b: P, db: P, bias: Bias) -> (Vec<P>, bool) {
    let a1 = (a.0 + da.0, a.1 + da.1);
    let b1 = (b.0 + db.0, b.1 + db.1);
    if a1 == b1 || a == b {
        return (routing::simplify(vec![a, a1, b]), false);
    }
    let goal_dir = (-db.0, -db.1);
    for margin in [4, 12, MARGIN_MAX] {
        if let Some(mut path) = search(board, net, a1, da, b1, goal_dir, margin, bias) {
            path.insert(0, a);
            path.push(b);
            return (routing::simplify(path), false);
        }
    }
    // No clean route: fall back to the plain geometric one.
    (routing::route(a, da, b, db, None), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lb_core::circuit::Project;
    use lb_core::component::Kind;

    fn setup(build: impl FnOnce(&mut Circuit)) -> (Geometry, Circuit) {
        let mut c = Circuit::new("t");
        build(&mut c);
        let p = Project::new(c.clone());
        (Geometry::new(&p, &c), c)
    }

    fn inside(g: &Geometry, c: &Circuit, p: P, except: &[CompId]) -> bool {
        c.components.iter().filter(|x| !except.contains(&x.id)).any(|x| {
            let (x0, y0, x1, y1) = g.items[&x.id].bounds();
            p.0 >= x0 && p.0 <= x1 && p.1 >= y0 && p.1 <= y1
        })
    }

    #[test]
    fn avoids_components_in_the_way() {
        let (g, c) = setup(|c| {
            let a = c.add(Kind::Input, (0, 0));
            c.add(Kind::And, (6, -1)); // obstacle right on the straight line
            let o = c.add(Kind::Led, (16, 0));
            c.connect((a, "out"), (o, "in"));
        });
        let r = route_all(&g, &c, &mut Memo::default());
        let pts = &r.wires[0].points;
        let mut all = Vec::new();
        for s in pts.windows(2) {
            walk(s[0], s[1], |_, q| all.push(q));
        }
        all.pop(); // the destination pin
        assert!(all.iter().all(|&p| !inside(&g, &c, p, &[])), "{pts:?}");
        for s in pts.windows(2) {
            assert!(s[0].0 == s[1].0 || s[0].1 == s[1].1, "orthogonal");
        }
    }

    #[test]
    fn different_nets_never_share_segments() {
        let (g, c) = setup(|c| {
            let a = c.add(Kind::Input, (0, 0));
            let b = c.add(Kind::Input, (0, 4));
            let o1 = c.add(Kind::Led, (14, 8));
            let o2 = c.add(Kind::Led, (14, 12));
            c.connect((a, "out"), (o1, "in"));
            c.connect((b, "out"), (o2, "in"));
        });
        let r = route_all(&g, &c, &mut Memo::default());
        let edge = |a: P, b: P| if a <= b { (a, b) } else { (b, a) };
        let mut edges: Vec<std::collections::HashSet<(P, P)>> = Vec::new();
        for w in &r.wires {
            let mut s = std::collections::HashSet::new();
            for seg in w.points.windows(2) {
                walk(seg[0], seg[1], |a, b| {
                    s.insert(edge(a, b));
                });
            }
            edges.push(s);
        }
        assert!(edges[0].is_disjoint(&edges[1]));
    }

    #[test]
    fn fan_out_shares_a_trunk() {
        let (g, c) = setup(|c| {
            let a = c.add(Kind::Input, (0, 4));
            for y in [0, 4, 8] {
                let o = c.add(Kind::Led, (12, y));
                c.connect((a, "out"), (o, "in"));
            }
        });
        let r = route_all(&g, &c, &mut Memo::default());
        assert!(!r.dots.is_empty(), "junction expected: {:?}", r.wires);
    }

    /// Points where routes of different wires cross.
    fn crossings(r: &Routes) -> usize {
        let pts: Vec<std::collections::HashSet<P>> = r
            .wires
            .iter()
            .map(|w| {
                let mut s = std::collections::HashSet::new();
                for seg in w.points.windows(2) {
                    walk(seg[0], seg[1], |a, b| {
                        s.insert(a);
                        s.insert(b);
                    });
                }
                s
            })
            .collect();
        let mut n = 0;
        for i in 0..pts.len() {
            for j in i + 1..pts.len() {
                n += pts[i].intersection(&pts[j]).count();
            }
        }
        n
    }

    /// Inputs stacked two units apart, wired to the consecutive inputs of a
    /// merger: the wires can be laid out as a staircase without crossings.
    fn stacked_into_merger(n: i32, gap: i32) -> (Geometry, Circuit) {
        setup(|c| {
            let m = c.add(Kind::Merger, (gap, 0));
            c.component_mut(m).unwrap().props.width = n as u8;
            for i in 0..n {
                let a = c.add(Kind::Input, (0, 2 * i));
                c.connect((a, "out"), (m, &format!("in{i}")));
            }
        })
    }

    #[test]
    fn wires_into_a_bus_do_not_cross() {
        for (n, gap) in [(8, 10), (8, 14), (4, 8), (6, 20)] {
            let (g, c) = stacked_into_merger(n, gap);
            let r = route_all(&g, &c, &mut Memo::default());
            assert_eq!(crossings(&r), 0, "{n} wires, gap {gap}: {:?}", r.wires);
        }
    }

    #[test]
    fn wires_added_one_at_a_time_into_a_bus_do_not_cross() {
        // As drawn by hand: earlier routes come from the memo.
        let (_, full) = stacked_into_merger(8, 10);
        let mut c = full.clone();
        c.wires.clear();
        let mut memo = Memo::default();
        for w in &full.wires {
            c.wires.push(w.clone());
            let p = Project::new(c.clone());
            let r = route_all(&Geometry::new(&p, &c), &c, &mut memo);
            assert_eq!(crossings(&r), 0, "after {} wires: {:?}", c.wires.len(), r.wires);
        }
    }

    #[test]
    fn wires_out_of_a_bus_do_not_cross() {
        for (n, gap) in [(7, 10), (8, 12), (5, 8)] {
            let (g, c) = setup(|c| {
                let s = c.add(Kind::Splitter, (0, 0));
                c.component_mut(s).unwrap().props.width = n as u8;
                for i in 0..n {
                    let o = c.add(Kind::Led, (gap, 2 * i));
                    c.connect((s, &format!("out{i}")), (o, "in"));
                }
            });
            let r = route_all(&g, &c, &mut Memo::default());
            assert_eq!(crossings(&r), 0, "{n} wires, gap {gap}: {:?}", r.wires);
        }
    }

    #[test]
    fn memo_reuses_unaffected_routes() {
        let (g, mut c) = setup(|c| {
            let a = c.add(Kind::Input, (0, 0));
            let o = c.add(Kind::Led, (10, 3));
            c.connect((a, "out"), (o, "in"));
        });
        let mut memo = Memo::default();
        let first = route_all(&g, &c, &mut memo).wires[0].points.clone();
        // Unrelated component far away: same route, from the memo.
        c.add(Kind::Not, (40, 40));
        let p = Project::new(c.clone());
        let g2 = Geometry::new(&p, &c);
        assert_eq!(route_all(&g2, &c, &mut memo).wires[0].points, first);
        assert_eq!(memo.routes.len(), 1);
    }
}

#[cfg(test)]
mod perf {
    use super::*;
    use lb_core::circuit::Project;
    use lb_core::component::Kind;

    /// 5 000 components in 50 chains of 100 gates, plus fan-outs across rows.
    pub fn stress() -> Circuit {
        let mut c = Circuit::new("stress");
        for row in 0..50 {
            let y = row * 6;
            let mut prev = (c.add(Kind::Input, (0, y)), "out");
            for col in 0..99 {
                let kind = if col % 2 == 0 { Kind::Not } else { Kind::Buffer };
                let g = c.add(kind, (5 + col * 6, y));
                c.connect(prev, (g, "in"));
                prev = (g, "out");
            }
        }
        // Long wires across rows and columns (Input of row r to a led far
        // away), forcing real searches around obstacles.
        let mut seed = 12345u32;
        for k in 0..300 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let col = (seed >> 8) % 98;
            let row = (seed >> 20) % 49;
            let led = c.add(Kind::Led, (8 + col as i32 * 6, row as i32 * 6 + 3));
            let src = 1 + (k % 50) * 100;
            c.connect((src, "out"), (led, "in"));
        }
        c
    }

    /// Heavy: runs only in release builds (`cargo test --release`).
    #[test]
    #[cfg_attr(debug_assertions, ignore)]
    fn routes_5000_components_quickly() {
        let c = stress();
        assert_eq!(c.components.len(), 5300);
        let p = Project::new(c.clone());
        let g = Geometry::new(&p, &c);
        let mut memo = Memo::default();
        let t = std::time::Instant::now();
        let r = route_all(&g, &c, &mut memo);
        let first = t.elapsed();
        let t = std::time::Instant::now();
        route_all(&g, &c, &mut memo);
        let again = t.elapsed();
        let fallbacks = memo.routes.values().filter(|r| r.1).count();
        eprintln!("route 5000: first {first:?}, memoized {again:?}, fallbacks {fallbacks}");
        // Re-routing after an edit only touches what changed.
        assert!(again.as_millis() < 300, "memoized pass took {again:?}");
        assert_eq!(r.wires.len(), 5250);
        assert!(r.wires.iter().all(|w| w.points.len() >= 2));
    }
}
