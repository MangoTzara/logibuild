//! Orthogonal wire routing and automatic junction dots.
//!
//! A wire joins two pins. Its route leaves each pin in the pin's outward
//! direction and connects the two stubs with an L or Z shape. Wires that
//! start from the same pin share their vertical (or horizontal) trunk, which
//! makes fan-outs look like a bus with branches. Junction dots are placed
//! wherever three or more wire segments of the same net meet.

use std::collections::{HashMap, HashSet};

use lb_core::circuit::{CompGeom, CompId, Geometry, PinRef, Wire};

pub type P = (i32, i32);

#[derive(Clone, Debug)]
pub struct WireRoute {
    /// Polyline in grid coordinates (empty if an end is missing).
    pub points: Vec<P>,
    /// Component and layout pin index of the `from` end, to look up the net.
    pub from: Option<(CompId, usize)>,
}

#[derive(Clone, Debug, Default)]
pub struct Routes {
    pub wires: Vec<WireRoute>,
    /// Junction dots, with the index of a wire of the same net (for colour).
    pub dots: Vec<(P, usize)>,
    /// Pins with at least one wire: `(component, pin index)`.
    pub connected: HashSet<(CompId, usize)>,
}

/// Outward direction of pin `i` of a component, in world coordinates.
pub fn pin_direction(g: &CompGeom, i: usize) -> P {
    let (x, y) = g.layout.pins[i].pos;
    let (w, h) = (g.layout.w, g.layout.h);
    let d = if x == 0 {
        (-1, 0)
    } else if x == w {
        (1, 0)
    } else if y == 0 {
        (0, -1)
    } else if y == h {
        (0, 1)
    } else {
        (-1, 0)
    };
    let a = g.transform.apply((x, y));
    let b = g.transform.apply((x + d.0, y + d.1));
    (b.0 - a.0, b.1 - a.1)
}

fn add(a: P, b: P) -> P {
    (a.0 + b.0, a.1 + b.1)
}

/// Removes repeated points and collinear middle points.
pub fn simplify(points: Vec<P>) -> Vec<P> {
    let mut out: Vec<P> = Vec::with_capacity(points.len());
    for p in points {
        if out.last() == Some(&p) {
            continue;
        }
        if out.len() >= 2 {
            let a = out[out.len() - 2];
            let b = out[out.len() - 1];
            let collinear = (a.0 == b.0 && b.0 == p.0) || (a.1 == b.1 && b.1 == p.1);
            // Only drop `b` if it lies between `a` and `p` (no U-turn).
            let between = (a.0.min(p.0)..=a.0.max(p.0)).contains(&b.0) && (a.1.min(p.1)..=a.1.max(p.1)).contains(&b.1);
            if collinear && between {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

/// Route between pin `a` (outward direction `da`) and pin `b` (`db`).
/// `trunk` fixes the coordinate of the middle segment for Z shapes.
pub fn route(a: P, da: P, b: P, db: P, trunk: Option<i32>) -> Vec<P> {
    let a1 = add(a, da);
    let b1 = add(b, db);
    let horiz_a = da.1 == 0;
    let horiz_b = db.1 == 0;
    let pts = match (horiz_a, horiz_b) {
        (true, true) => {
            // Facing each other (or at least b1 ahead of a1): Z shape.
            let ahead = (b1.0 - a1.0) * da.0 >= 0;
            if ahead || a.1 == b.1 && (b.0 - a.0) * da.0 > 0 {
                let xm = trunk.unwrap_or((a1.0 + b1.0).div_euclid(2));
                vec![a, (xm, a.1), (xm, b.1), b]
            } else {
                // Loop around: leave, go to the middle row, come back.
                let ym = if a.1 == b.1 { a.1 + 2 } else { (a.1 + b.1).div_euclid(2) };
                vec![a, a1, (a1.0, ym), (b1.0, ym), b1, b]
            }
        }
        (false, false) => {
            let ahead = (b1.1 - a1.1) * da.1 >= 0;
            if ahead {
                let ym = trunk.unwrap_or((a1.1 + b1.1).div_euclid(2));
                vec![a, (a.0, ym), (b.0, ym), b]
            } else {
                let xm = if a.0 == b.0 { a.0 + 2 } else { (a.0 + b.0).div_euclid(2) };
                vec![a, a1, (xm, a1.1), (xm, b1.1), b1, b]
            }
        }
        (true, false) => {
            // Horizontal from a, vertical into b.
            if (b.0 - a.0) * da.0 >= 0 && (a.1 - b.1) * db.1 >= 0 {
                vec![a, (b.0, a.1), b]
            } else {
                vec![a, a1, (a1.0, b1.1), b1, b]
            }
        }
        (false, true) => {
            if (b.1 - a.1) * da.1 >= 0 && (a.0 - b.0) * db.0 >= 0 {
                vec![a, (a.0, b.1), b]
            } else {
                vec![a, a1, (b1.0, a1.1), b1, b]
            }
        }
    };
    simplify(pts)
}

/// Route through user waypoints: L segments, horizontal first.
pub fn route_through(a: P, path: &[P], b: P) -> Vec<P> {
    let mut pts = vec![a];
    let mut cur = a;
    for &p in path.iter().chain(std::iter::once(&b)) {
        if cur.0 != p.0 && cur.1 != p.1 {
            pts.push((p.0, cur.1));
        }
        pts.push(p);
        cur = p;
    }
    simplify(pts)
}

/// Removes the waypoints that do not shape the route from pin `a` to pin
/// `b` through `path` (see [`route_through`]): those on a straight run, which
/// leave the route unchanged, and those where the wire turns back on itself,
/// which drops the useless spur.
pub fn tidy_path(a: P, path: &[P], b: P) -> Vec<P> {
    // The corner of the L from `f` to `t`, if it is not straight.
    let elbow = |f: P, t: P, straight: P| if f.0 != t.0 && f.1 != t.1 { (t.0, f.1) } else { straight };
    let mut path = path.to_vec();
    while let Some(i) = (0..path.len()).find(|&i| {
        let w = path[i];
        let prev = if i == 0 { a } else { path[i - 1] };
        let next = path.get(i + 1).copied().unwrap_or(b);
        // The route points just before and just after `w`.
        let (before, after) = (elbow(prev, w, prev), elbow(w, next, next));
        (before.0 == w.0 && w.0 == after.0) || (before.1 == w.1 && w.1 == after.1)
    }) {
        path.remove(i);
    }
    path
}

/// `path` with waypoint `i` moved to `p`, for a wire from pin `a` to pin `b`
/// (each `(position, outward direction)`). The neighbouring waypoints joined
/// to it by a straight segment follow it on the other axis, so that dragging
/// a corner moves its segments instead of bending them (the elbows of
/// [`route_through`] would otherwise let only one end of a segment move it).
/// A pin cannot follow: it gets a new bend instead (see [`jog`]).
pub fn move_waypoint(a: (P, P), path: &[P], i: usize, p: P, b: (P, P)) -> Vec<P> {
    let old = path[i];
    let mut out = path.to_vec();
    for j in [i.checked_sub(1), Some(i + 1)].into_iter().flatten() {
        if let Some(n) = out.get_mut(j) {
            if n.0 == old.0 {
                n.0 = p.0;
            }
            if n.1 == old.1 {
                n.1 = p.1;
            }
        }
    }
    out[i] = p;
    if i + 1 == path.len() {
        out.extend(jog(b, old, p).into_iter().rev());
    }
    if i == 0 {
        out.splice(0..0, jog(a, old, p));
    }
    out
}

/// Bends joining `pin` to the segment it shared with waypoint `old`, moved
/// with it to `p`, in order from the pin. One corner if the wire still leaves
/// the pin outwards, else a Z stepping halfway along the old segment.
fn jog((pin, dir): (P, P), old: P, p: P) -> Vec<P> {
    let (corner, z) = if pin.0 == old.0 && p.0 != pin.0 {
        let m = (pin.1 + old.1).div_euclid(2);
        ((p.0, pin.1), [(pin.0, m), (p.0, m)])
    } else if pin.1 == old.1 && p.1 != pin.1 {
        let m = (pin.0 + old.0).div_euclid(2);
        ((pin.0, p.1), [(m, pin.1), (m, p.1)])
    } else {
        return Vec::new();
    };
    let leaves = ((corner.0 - pin.0).signum(), (corner.1 - pin.1).signum());
    if leaves == dir { vec![corner] } else { z.to_vec() }
}

/// Waypoints at every corner of the route from pin `a` to pin `b` through
/// `path`, so that the corners the L segments add can be dragged too. Loops
/// and spurs are cut out of the route (see [`erase_loops`]).
pub fn pin_corners(a: P, path: &[P], b: P) -> Vec<P> {
    let pts = simplify(erase_loops(&route_through(a, path, b)));
    pts.get(1..pts.len().saturating_sub(1)).map(<[P]>::to_vec).unwrap_or_default()
}

/// The grid points along the orthogonal polyline `pts`, with the part
/// between two passes through the same point dropped: the wire neither
/// crosses nor runs back over itself.
fn erase_loops(pts: &[P]) -> Vec<P> {
    let mut walk: Vec<P> = Vec::new();
    // Index of each point of `walk`.
    let mut seen: HashMap<P, usize> = HashMap::new();
    let mut visit = |p: P| match seen.get(&p) {
        Some(&i) => {
            for q in walk.drain(i + 1..) {
                seen.remove(&q);
            }
        }
        None => {
            seen.insert(p, walk.len());
            walk.push(p);
        }
    };
    if let Some(&first) = pts.first() {
        visit(first);
    }
    for s in pts.windows(2) {
        let step = ((s[1].0 - s[0].0).signum(), (s[1].1 - s[0].1).signum());
        let mut p = s[0];
        while p != s[1] {
            p = add(p, step);
            visit(p);
        }
    }
    walk
}

/// Shortest straight run with which a wire with waypoints leaves a pin.
pub const MIN_STUB: i32 = 1;

/// Like [`pin_corners`], for a wire from pin `a` to pin `b` (each
/// `(position, outward direction)`), first making the route leave each pin
/// outwards and straight for at least [`MIN_STUB`] where there is room
/// (otherwise the added point is the tip of a spur, and is dropped again).
pub fn with_stubs(a: (P, P), path: &[P], b: (P, P)) -> Vec<P> {
    if path.is_empty() {
        return Vec::new();
    }
    let pts = route_through(a.0, path, b.0);
    // Whether the route leaves `pin` through `next` as it should.
    let straight = |(pin, dir): (P, P), next: P| {
        let d = (next.0 - pin.0, next.1 - pin.1);
        (d.0.signum(), d.1.signum()) == dir && d.0.abs() + d.1.abs() >= MIN_STUB
    };
    let stub = |(pin, dir): (P, P)| (pin.0 + dir.0 * MIN_STUB, pin.1 + dir.1 * MIN_STUB);
    let mut path = path.to_vec();
    if pts.len() >= 2 && !straight(a, pts[1]) {
        path.insert(0, stub(a));
    }
    if pts.len() >= 2 && !straight(b, pts[pts.len() - 2]) {
        path.push(stub(b));
    }
    pin_corners(a.0, &path, b.0)
}

/// Resolves a pin to `(position, direction, index)`, with an optional offset
/// applied to moved components.
pub(crate) fn pin_info(geom: &Geometry, p: &PinRef, offset: &dyn Fn(CompId) -> P) -> Option<(P, P, usize)> {
    let g = geom.items.get(&p.0)?;
    let (i, _) = g.layout.pin(&p.1)?;
    let o = offset(p.0);
    Some((add(g.pins[i], o), pin_direction(g, i), i))
}

/// Computes the routes of `wires`. Components in `moved` are displaced by
/// `delta` (used while dragging).
pub fn compute(geom: &Geometry, wires: &[Wire], moved: &HashSet<CompId>, delta: P) -> Routes {
    let offset = |id: CompId| if moved.contains(&id) { delta } else { (0, 0) };

    // Trunk per source pin: the Z-shape middle segment closest to the source
    // among its wires, so that fan-outs share it.
    struct Trunk {
        /// Vertical trunk (horizontal pins) or horizontal trunk.
        vertical: bool,
        pos: i32,
        /// Allowed positions: between the source stub and the nearest target.
        range: (i32, i32),
        /// Extent of the trunk along its own axis.
        span: (i32, i32),
    }
    let mut trunks: HashMap<&PinRef, Trunk> = HashMap::new();
    let mut ends = Vec::with_capacity(wires.len());
    for w in wires {
        let e = (pin_info(geom, &w.from, &offset), pin_info(geom, &w.to, &offset));
        if let (Some((a, da, _)), Some((b, db, _))) = e
            && w.path.is_empty()
        {
            let (a1, b1) = (add(a, da), add(b, db));
            let vertical = da.1 == 0 && db.1 == 0 && (b1.0 - a1.0) * da.0 >= 0;
            let horizontal = da.0 == 0 && db.0 == 0 && (b1.1 - a1.1) * da.1 >= 0;
            if vertical || horizontal {
                let (main_a, main_b, cross_a, cross_b) =
                    if vertical { (a1.0, b1.0, a.1, b.1) } else { (a1.1, b1.1, a.0, b.0) };
                let c = (main_a + main_b).div_euclid(2);
                let range = (main_a.min(main_b), main_a.max(main_b));
                let t = trunks.entry(&w.from).or_insert(Trunk {
                    vertical,
                    pos: c,
                    range,
                    span: (cross_a.min(cross_b), cross_a.max(cross_b)),
                });
                if (c - main_a).abs() < (t.pos - main_a).abs() {
                    t.pos = c;
                }
                // The allowed range shrinks to the nearest target.
                t.range = if main_b >= main_a {
                    (t.range.0, t.range.1.min(main_b))
                } else {
                    (t.range.0.max(main_b), t.range.1)
                };
                t.span = (t.span.0.min(cross_b), t.span.1.max(cross_b));
            }
        }
        ends.push(e);
    }
    // Shift trunks of different nets that would overlap on the same line.
    let mut order: Vec<&PinRef> = trunks.keys().copied().collect();
    order.sort();
    let mut placed: Vec<(bool, i32, (i32, i32))> = Vec::new();
    for key in order {
        let t = trunks.get_mut(key).unwrap();
        let clash = |pos: i32| {
            placed.iter().any(|&(v, p, (s0, s1))| v == t.vertical && p == pos && s0 <= t.span.1 && t.span.0 <= s1)
        };
        let base = t.pos;
        for k in [0, 1, -1, 2, -2, 3, -3, 4, -4] {
            let pos = base + k;
            if pos >= t.range.0 && pos <= t.range.1 && !clash(pos) {
                t.pos = pos;
                break;
            }
        }
        placed.push((t.vertical, t.pos, t.span));
    }
    let trunks: HashMap<&PinRef, i32> = trunks.into_iter().map(|(k, t)| (k, t.pos)).collect();

    let mut routes = Routes::default();
    for (w, e) in wires.iter().zip(ends) {
        let (points, from) = match e {
            (Some((a, da, ia)), Some((b, db, ib))) => {
                routes.connected.insert((w.from.0, ia));
                routes.connected.insert((w.to.0, ib));
                let pts = if w.path.is_empty() {
                    let trunk = trunks.get(&w.from).copied();
                    // Only use the shared trunk if it lies between the ends.
                    let trunk = trunk.filter(|&t| {
                        if da.1 == 0 && db.1 == 0 {
                            (a.0.min(b.0)..=a.0.max(b.0)).contains(&t)
                        } else if da.0 == 0 && db.0 == 0 {
                            (a.1.min(b.1)..=a.1.max(b.1)).contains(&t)
                        } else {
                            false
                        }
                    });
                    route(a, da, b, db, trunk)
                } else {
                    // As the move will leave it (see `App::tidy_wires`): the
                    // waypoints follow when both ends move, else the route
                    // loses its spurs.
                    let path: Vec<P> = match (moved.contains(&w.from.0), moved.contains(&w.to.0)) {
                        (true, true) => w.path.iter().map(|&p| add(p, delta)).collect(),
                        (false, false) => w.path.clone(),
                        _ => with_stubs((a, da), &w.path, (b, db)),
                    };
                    route_through(a, &path, b)
                };
                (pts, Some((w.from.0, ia)))
            }
            _ => (Vec::new(), None),
        };
        routes.wires.push(WireRoute { points, from });
    }
    routes.dots = junctions(wires, &routes.wires);
    routes
}

/// Points where three or more unit segments of the same net meet, each with
/// the index of one wire of that net.
pub(crate) fn junctions(wires: &[Wire], routes: &[WireRoute]) -> Vec<(P, usize)> {
    // Group wires into nets with a union-find over their pins.
    let mut ids: HashMap<&PinRef, usize> = HashMap::new();
    let mut parent: Vec<usize> = Vec::new();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut wire_pins = Vec::with_capacity(wires.len());
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
            parent[ra] = rb;
        }
        wire_pins.push(idx[0]);
    }

    let mut edges: HashMap<usize, (usize, HashSet<(P, P)>)> = HashMap::new();
    for (i, r) in routes.iter().enumerate() {
        let net = find(&mut parent, wire_pins[i]);
        let (_, set) = edges.entry(net).or_insert_with(|| (i, HashSet::new()));
        for seg in r.points.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            if a.1 == b.1 {
                for x in a.0.min(b.0)..a.0.max(b.0) {
                    set.insert(((x, a.1), (x + 1, a.1)));
                }
            } else {
                for y in a.1.min(b.1)..a.1.max(b.1) {
                    set.insert(((a.0, y), (a.0, y + 1)));
                }
            }
        }
    }
    let mut dots = Vec::new();
    for (wire, set) in edges.values() {
        let mut degree: HashMap<P, u32> = HashMap::new();
        for (a, b) in set {
            *degree.entry(*a).or_default() += 1;
            *degree.entry(*b).or_default() += 1;
        }
        dots.extend(degree.into_iter().filter(|(_, d)| *d >= 3).map(|(p, _)| (p, *wire)));
    }
    dots.sort_unstable();
    dots
}

/// Position of `p` along the polyline `pts`, as arc length to its projection.
fn arc_position(pts: &[P], p: P) -> f32 {
    let q = (p.0 as f32, p.1 as f32);
    let mut best = (f32::MAX, 0.0);
    let mut acc = 0.0;
    for s in pts.windows(2) {
        let d = dist_to_segment(q, s[0], s[1]);
        let len = ((s[1].0 - s[0].0).abs() + (s[1].1 - s[0].1).abs()) as f32;
        if d < best.0 {
            let along = ((q.0 - s[0].0 as f32).abs() + (q.1 - s[0].1 as f32).abs()).min(len);
            best = (d, acc + along);
        }
        acc += len;
    }
    best.1
}

/// Inserts waypoint `p` in `path` keeping the order along the wire's
/// current route `route`.
pub fn insert_waypoint(route: &[P], path: &[P], p: P) -> Vec<P> {
    let mut all: Vec<(f32, P)> = path.iter().map(|&w| (arc_position(route, w), w)).collect();
    if path.contains(&p) {
        return path.to_vec();
    }
    let s = arc_position(route, p);
    let i = all.partition_point(|(t, _)| *t <= s);
    all.insert(i, (s, p));
    all.into_iter().map(|(_, w)| w).collect()
}

/// Distance from point `p` to segment `a`–`b` (grid units).
pub fn dist_to_segment(p: (f32, f32), a: P, b: P) -> f32 {
    let (ax, ay, bx, by) = (a.0 as f32, a.1 as f32, b.0 as f32, b.1 as f32);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - ax) * dx + (p.1 - ay) * dy) / len2).clamp(0.0, 1.0) };
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn z_route_between_facing_pins() {
        let r = route((0, 0), (1, 0), (10, 4), (-1, 0), None);
        assert_eq!(r, vec![(0, 0), (5, 0), (5, 4), (10, 4)]);
        let straight = route((0, 2), (1, 0), (6, 2), (-1, 0), None);
        assert_eq!(straight, vec![(0, 2), (6, 2)]);
    }

    #[test]
    fn loop_back_route() {
        let r = route((10, 0), (1, 0), (0, 0), (-1, 0), None);
        assert_eq!(r.first(), Some(&(10, 0)));
        assert_eq!(r.last(), Some(&(0, 0)));
        for s in r.windows(2) {
            assert!(s[0].0 == s[1].0 || s[0].1 == s[1].1, "orthogonal");
        }
    }

    #[test]
    fn simplify_keeps_u_turns() {
        assert_eq!(simplify(vec![(0, 0), (1, 0), (2, 0), (2, 0), (2, 3)]), vec![(0, 0), (2, 0), (2, 3)]);
        assert_eq!(simplify(vec![(0, 0), (3, 0), (1, 0)]), vec![(0, 0), (3, 0), (1, 0)]);
    }

    #[test]
    fn trunks_of_different_nets_do_not_overlap() {
        // Two sources at the same x, targets at the same x: the natural
        // trunks coincide and must be separated.
        let mut c = lb_core::circuit::Circuit::new("t");
        let a = c.add(lb_core::component::Kind::Input, (0, 0));
        let b = c.add(lb_core::component::Kind::Input, (0, 2));
        let o1 = c.add(lb_core::component::Kind::Led, (10, 6));
        let o2 = c.add(lb_core::component::Kind::Led, (10, 8));
        c.connect((a, "out"), (o1, "in"));
        c.connect((b, "out"), (o2, "in"));
        let p = lb_core::circuit::Project::new(c.clone());
        let g = Geometry::new(&p, &c);
        let r = compute(&g, &c.wires, &HashSet::new(), (0, 0));
        let xs: Vec<i32> = r.wires.iter().map(|w| w.points[1].0).collect();
        assert_ne!(xs[0], xs[1], "{:?}", r.wires);
    }

    #[test]
    fn waypoints_inserted_in_route_order() {
        let route = vec![(0, 0), (10, 0), (10, 10)];
        let path = insert_waypoint(&route, &[], (4, 0));
        let path = insert_waypoint(&route, &path, (10, 6));
        let path = insert_waypoint(&route, &path, (2, 0));
        assert_eq!(path, vec![(2, 0), (4, 0), (10, 6)]);
    }

    #[test]
    fn tidy_path_drops_straight_points_and_spurs() {
        // On a straight run: removed, the route is the same.
        let (a, b) = ((0, 0), (10, 6));
        let path = [(4, 0), (7, 0), (7, 3), (7, 6)];
        let tidy = tidy_path(a, &path, b);
        assert_eq!(tidy, [(7, 0), (7, 6)]);
        assert_eq!(route_through(a, &tidy, b), route_through(a, &path, b));
        // Corners stay.
        let corners = [(5, 0), (5, 6)];
        assert_eq!(tidy_path(a, &corners, b), corners);
        // A spur past the pin it ends at (the wire goes on and comes back).
        let (a, b) = ((0, 10), (12, 0));
        let path = [(0, 4), (8, 4), (8, 0), (4, 0)];
        assert_eq!(tidy_path(a, &path, b), [(0, 4), (8, 4), (8, 0)]);
        // A spur in the middle, and duplicate points.
        let path = [(0, 4), (0, 2), (0, 2), (0, 6), (12, 6)];
        assert_eq!(tidy_path(a, &path, b), [(0, 6), (12, 6)]);
    }

    #[test]
    fn pin_corners_adds_the_elbows_of_moved_waypoints() {
        // A frozen Z route; dragging its first corner off the run adds two elbows.
        let (a, b) = ((0, 0), (10, 6));
        let path = [(3, 2), (5, 6)];
        let corners = pin_corners(a, &path, b);
        assert_eq!(corners, [(3, 0), (3, 2), (5, 2), (5, 6)]);
        assert_eq!(route_through(a, &corners, b), route_through(a, &path, b));
        // Already one waypoint per corner: unchanged.
        assert_eq!(pin_corners(a, &[(5, 0), (5, 6)], b), [(5, 0), (5, 6)]);
        assert!(pin_corners(a, &[], (10, 0)).is_empty());
    }

    #[test]
    fn moving_either_corner_moves_the_segment_between() {
        // A Z route from (0, 0) to (10, -4) with its vertical run at x = 5.
        let (a, b) = ((0, 0), (10, -4));
        let path = [(5, 0), (5, -4)];
        let moved = |i, p| pin_corners(a, &move_waypoint((a, (1, 0)), &path, i, p, (b, (-1, 0))), b);
        assert_eq!(moved(0, (7, 0)), [(7, 0), (7, -4)]);
        assert_eq!(moved(1, (7, -4)), [(7, 0), (7, -4)]);
        assert_eq!(moved(0, (3, 0)), [(3, 0), (3, -4)]);
        // Off the pin's row: the pin cannot follow, and the wire must leave
        // it sideways, so it steps halfway.
        assert_eq!(moved(1, (5, -6)), [(5, 0), (5, -6), (7, -6), (7, -4)]);
    }

    #[test]
    fn moving_the_corner_next_to_a_pin_bends_into_it() {
        // An L from (0, 0) down into a pin at (6, 8) facing left.
        let (a, b) = (((0, 0), (1, 0)), ((6, 8), (-1, 0)));
        let moved = |p| pin_corners(a.0, &move_waypoint(a, &[(6, 0)], 0, p, b), b.0);
        // To the left the wire still enters the pin from its side: one bend.
        assert_eq!(moved((4, 0)), [(4, 0), (4, 8)]);
        // To the right it would enter from behind: a step halfway instead.
        assert_eq!(moved((8, 0)), [(8, 0), (8, 4), (6, 4)]);
    }

    #[test]
    fn dragging_one_end_drops_the_spur_left_behind() {
        let mut c = lb_core::circuit::Circuit::new("t");
        let n = c.add(lb_core::component::Kind::Not, (0, 0));
        let led = c.add(lb_core::component::Kind::Led, (10, 0));
        c.connect((n, "out"), (led, "in"));
        let p = lb_core::circuit::Project::new(c.clone());
        let g = Geometry::new(&p, &c);
        let w = &mut c.wires[0];
        let (a, b) = (g.pin_pos(&w.from).unwrap(), g.pin_pos(&w.to).unwrap());
        // Down, across to the LED's column and up into it.
        w.path = vec![(a.0 + 2, a.1), (a.0 + 2, a.1 + 6), (b.0, a.1 + 6)];
        // Dragging the LED left: the run back to its new column is not drawn.
        let r = compute(&g, &c.wires, &[led].into(), (-3, 0));
        let right = r.wires[0].points.iter().map(|p| p.0).max().unwrap();
        assert_eq!(right, b.0 - 3, "{:?}", r.wires[0].points);
    }

    #[test]
    fn wires_leave_pins_straight_where_there_is_room() {
        // Into a pin at (20, 6) facing left, from straight above it.
        let (a, b) = (((0, 0), (1, 0)), ((20, 6), (-1, 0)));
        let path = [(8, 0), (8, 8), (14, 8), (14, 2), (20, 2)];
        assert_eq!(with_stubs(a, &path, b), [(8, 0), (8, 8), (14, 8), (14, 2), (19, 2), (19, 6)]);
        // Already straight: unchanged.
        let ok = [(8, 0), (8, 6)];
        assert_eq!(with_stubs(a, &ok, b), ok);
        // No room: the wire runs straight on into a pin facing away, and the
        // stub past it would be a spur. Nothing is added.
        let (a, b) = (((0, 0), (1, 0)), ((10, 0), (1, 0)));
        assert_eq!(with_stubs(a, &[(5, 0), (7, 0)], b), pin_corners(a.0, &[(5, 0), (7, 0)], b.0));
    }

    #[test]
    fn pin_corners_cuts_out_loops() {
        // Right past the corner (3, 0), round a square and back down through
        // it: the wire would cross itself there.
        let (a, b) = ((0, 0), (12, 5));
        let path = [(6, 0), (6, -3), (3, -3), (3, 5)];
        assert_eq!(pin_corners(a, &path, b), [(3, 0), (3, 5)]);
        // Coming back onto its first run and along it counts too.
        assert_eq!(pin_corners(a, &[(6, 0), (6, -2), (3, -2), (3, 0)], (8, 4)), [(8, 0)]);
    }

    #[test]
    fn fan_out_has_junction() {
        let wires = vec![
            Wire::new(PinRef::new(1, "out"), PinRef::new(2, "in")),
            Wire::new(PinRef::new(1, "out"), PinRef::new(3, "in")),
            Wire::new(PinRef::new(1, "out"), PinRef::new(4, "in")),
        ];
        let routes = vec![
            WireRoute { points: vec![(0, 0), (4, 0), (4, -3), (8, -3)], from: None },
            WireRoute { points: vec![(0, 0), (8, 0)], from: None },
            WireRoute { points: vec![(0, 0), (4, 0), (4, 3), (8, 3)], from: None },
        ];
        assert_eq!(junctions(&wires, &routes), vec![((4, 0), 0)]);
    }
}
