//! Schematic model: components, wires and the project holding several
//! circuits (the main one plus subcircuits).
//!
//! Wires connect two pins (`(component id, pin name)`); all pins joined by
//! wires form a net. Drawing a wire is a purely visual matter, computed by
//! the editor from the pin positions and the optional waypoints in `path`.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::component::{Kind, Layout, PinDir, Props, Transform, layout::layout};

pub type CompId = u32;

fn is_zero(v: &u8) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Component {
    pub id: CompId,
    pub kind: Kind,
    pub pos: (i32, i32),
    /// Clockwise quarter turns (0..4).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rot: u8,
    #[serde(default, skip_serializing_if = "is_false")]
    pub mirror: bool,
    #[serde(default, skip_serializing_if = "Props::is_default")]
    pub props: Props,
}

/// A pin of a component: `(component id, pin name)`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PinRef(pub CompId, pub String);

impl PinRef {
    pub fn new(id: CompId, pin: impl Into<String>) -> Self {
        PinRef(id, pin.into())
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Wire {
    pub from: PinRef,
    pub to: PinRef,
    /// Optional user waypoints (grid coordinates) between the two ends.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<(i32, i32)>,
}

impl Wire {
    pub fn new(from: PinRef, to: PinRef) -> Self {
        Wire { from, to, path: Vec::new() }
    }

    pub fn touches(&self, id: CompId) -> bool {
        self.from.0 == id || self.to.0 == id
    }

    /// Same endpoints regardless of direction.
    pub fn same_ends(&self, other: &Wire) -> bool {
        (self.from == other.from && self.to == other.to) || (self.from == other.to && self.to == other.from)
    }
}

/// Where a circuit comes from: embedded in the project file or loaded from a
/// library folder (in which case it is not saved inline).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Origin {
    #[default]
    Embedded,
    Library(std::path::PathBuf),
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Circuit {
    pub name: String,
    #[serde(default)]
    pub components: Vec<Component>,
    #[serde(default)]
    pub wires: Vec<Wire>,
    #[serde(skip)]
    pub origin: Origin,
}

impl Circuit {
    pub fn new(name: impl Into<String>) -> Self {
        Circuit { name: name.into(), ..Default::default() }
    }

    pub fn next_id(&self) -> CompId {
        self.components.iter().map(|c| c.id).max().unwrap_or(0) + 1
    }

    pub fn component(&self, id: CompId) -> Option<&Component> {
        self.index_of(id).map(|i| &self.components[i])
    }

    pub fn component_mut(&mut self, id: CompId) -> Option<&mut Component> {
        self.index_of(id).map(|i| &mut self.components[i])
    }

    fn index_of(&self, id: CompId) -> Option<usize> {
        // Components are kept sorted by id; fall back to a scan if a caller
        // broke the invariant.
        match self.components.binary_search_by_key(&id, |c| c.id) {
            Ok(i) => Some(i),
            Err(_) => self.components.iter().position(|c| c.id == id),
        }
    }

    /// Inserts keeping components sorted by id.
    pub fn insert_component(&mut self, c: Component) {
        let i = self.components.partition_point(|x| x.id < c.id);
        if self.components.get(i).is_some_and(|x| x.id == c.id) {
            self.components[i] = c;
        } else {
            self.components.insert(i, c);
        }
    }

    pub fn remove_component(&mut self, id: CompId) -> Option<Component> {
        self.index_of(id).map(|i| self.components.remove(i))
    }

    /// Inserts keeping wires sorted; duplicates are ignored.
    pub fn insert_wire(&mut self, w: Wire) -> bool {
        if self.wires.iter().any(|x| x.same_ends(&w)) {
            return false;
        }
        let i = self.wires.partition_point(|x| *x < w);
        self.wires.insert(i, w);
        true
    }

    pub fn remove_wire(&mut self, w: &Wire) -> bool {
        if let Some(i) = self.wires.iter().position(|x| x == w) {
            self.wires.remove(i);
            true
        } else {
            false
        }
    }

    /// Restores the canonical order (components by id, wires sorted) and
    /// converts components of older files.
    pub fn normalize(&mut self) {
        // The hex display of older files is a one-digit number display.
        for c in &mut self.components {
            if c.kind == Kind::HexDisplay {
                c.kind = Kind::NumDisplay;
                c.props.width = 4;
                c.props.radix = crate::value::Radix::Hex;
                c.props.digits = 1;
            }
        }
        self.components.sort_by_key(|c| c.id);
        self.wires.sort();
        self.wires.dedup_by(|a, b| a.same_ends(b));
    }

    /// Adds a component with default properties; returns its id.
    pub fn add(&mut self, kind: Kind, pos: (i32, i32)) -> CompId {
        let id = self.next_id();
        let props = kind.default_props();
        self.insert_component(Component { id, kind, pos, rot: 0, mirror: false, props });
        id
    }

    /// Adds a wire between two pins.
    pub fn connect(&mut self, from: (CompId, &str), to: (CompId, &str)) -> bool {
        self.insert_wire(Wire::new(PinRef::new(from.0, from.1), PinRef::new(to.0, to.1)))
    }

    pub fn subcircuit_refs(&self) -> impl Iterator<Item = &str> {
        self.components.iter().filter(|c| c.kind == Kind::Subcircuit).map(|c| c.props.circuit.as_str())
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum ProjectError {
    MissingCircuit { name: String, from: String },
    Recursion(Vec<String>),
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectError::MissingCircuit { name, from } => {
                write!(f, "circuit \"{from}\" uses subcircuit \"{name}\", which does not exist")
            }
            ProjectError::Recursion(cycle) => {
                write!(f, "recursive subcircuit reference: {}", cycle.join(" → "))
            }
        }
    }
}

impl std::error::Error for ProjectError {}

/// A set of circuits. `circuits[0]` is the main circuit.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Project {
    pub circuits: Vec<Circuit>,
    /// Library folders, relative to the project file.
    pub libraries: Vec<String>,
}

impl Project {
    pub fn new(main: Circuit) -> Self {
        Project { circuits: vec![main], libraries: Vec::new() }
    }

    pub fn main(&self) -> &Circuit {
        &self.circuits[0]
    }

    pub fn circuit(&self, name: &str) -> Option<&Circuit> {
        self.circuits.iter().find(|c| c.name == name)
    }

    pub fn circuit_mut(&mut self, name: &str) -> Option<&mut Circuit> {
        self.circuits.iter_mut().find(|c| c.name == name)
    }

    pub fn circuit_index(&self, name: &str) -> Option<usize> {
        self.circuits.iter().position(|c| c.name == name)
    }

    /// The first of `base 1`, `base 2`, … not yet used in the project.
    pub fn unique_name(&self, base: &str) -> String {
        (1..).map(|i| format!("{base} {i}")).find(|n| self.circuit(n).is_none()).unwrap()
    }

    /// Layout of a component of any circuit of this project.
    pub fn layout(&self, c: &Component) -> Layout {
        let sub = (c.kind == Kind::Subcircuit).then(|| self.circuit(&c.props.circuit)).flatten();
        layout(c.kind, &c.props, sub)
    }

    /// Checks that every subcircuit reference reachable from `root` exists
    /// and that there are no reference cycles.
    pub fn check_hierarchy(&self, root: &str) -> Result<(), ProjectError> {
        fn visit<'a>(
            p: &'a Project,
            name: &'a str,
            stack: &mut Vec<&'a str>,
            done: &mut HashSet<&'a str>,
        ) -> Result<(), ProjectError> {
            if let Some(i) = stack.iter().position(|n| *n == name) {
                let mut cycle: Vec<String> = stack[i..].iter().map(|s| s.to_string()).collect();
                cycle.push(name.to_string());
                return Err(ProjectError::Recursion(cycle));
            }
            if done.contains(name) {
                return Ok(());
            }
            let c = p.circuit(name).ok_or_else(|| ProjectError::MissingCircuit {
                name: name.to_string(),
                from: stack.last().map(|s| s.to_string()).unwrap_or_default(),
            })?;
            stack.push(name);
            for r in c.subcircuit_refs() {
                visit(p, r, stack, done)?;
            }
            stack.pop();
            done.insert(name);
            Ok(())
        }
        visit(self, root, &mut Vec::new(), &mut HashSet::new())
    }

    /// Whether placing an instance of `child` inside `parent` would create a
    /// reference cycle.
    pub fn would_recurse(&self, parent: &str, child: &str) -> bool {
        if parent == child {
            return true;
        }
        // Does `child` (transitively) use `parent`?
        let mut stack = vec![child];
        let mut seen = HashSet::new();
        while let Some(n) = stack.pop() {
            if n == parent {
                return true;
            }
            if !seen.insert(n) {
                continue;
            }
            if let Some(c) = self.circuit(n) {
                stack.extend(c.subcircuit_refs());
            }
        }
        false
    }
}

/// Geometry of the components of one circuit, resolved against a project:
/// layouts, transforms and pin positions in world grid coordinates.
#[derive(Clone, Debug, Default)]
pub struct Geometry {
    pub items: HashMap<CompId, CompGeom>,
}

#[derive(Clone, Debug)]
pub struct CompGeom {
    pub layout: Layout,
    pub transform: Transform,
    /// Top-left corner of the bounding box.
    pub pos: (i32, i32),
    /// Pin positions in world coordinates, parallel to `layout.pins`.
    pub pins: Vec<(i32, i32)>,
}

impl CompGeom {
    pub fn new(project: &Project, c: &Component) -> Self {
        let layout = project.layout(c);
        let transform = Transform::new(c.rot, c.mirror, &layout);
        let pins = layout
            .pins
            .iter()
            .map(|p| {
                let (x, y) = transform.apply(p.pos);
                (c.pos.0 + x, c.pos.1 + y)
            })
            .collect();
        CompGeom { layout, transform, pos: c.pos, pins }
    }

    pub fn size(&self) -> (i32, i32) {
        self.transform.size()
    }

    /// Bounding box `(x0, y0, x1, y1)`.
    pub fn bounds(&self) -> (i32, i32, i32, i32) {
        let (w, h) = self.size();
        (self.pos.0, self.pos.1, self.pos.0 + w, self.pos.1 + h)
    }

    pub fn pin_pos(&self, name: &str) -> Option<(i32, i32)> {
        self.layout.pin(name).map(|(i, _)| self.pins[i])
    }

    pub fn pin_dir(&self, name: &str) -> Option<PinDir> {
        self.layout.pin(name).map(|(_, p)| p.dir)
    }
}

impl Geometry {
    pub fn new(project: &Project, circuit: &Circuit) -> Self {
        let items = circuit.components.iter().map(|c| (c.id, CompGeom::new(project, c))).collect();
        Geometry { items }
    }

    pub fn pin_pos(&self, p: &PinRef) -> Option<(i32, i32)> {
        self.items.get(&p.0)?.pin_pos(&p.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp(id: CompId, kind: Kind) -> Component {
        Component { id, kind, pos: (0, 0), rot: 0, mirror: false, props: kind.default_props() }
    }

    #[test]
    fn sorted_insertions() {
        let mut c = Circuit::new("t");
        c.insert_component(comp(3, Kind::And));
        c.insert_component(comp(1, Kind::Or));
        c.insert_component(comp(2, Kind::Not));
        assert_eq!(c.components.iter().map(|c| c.id).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(c.next_id(), 4);
        let w = Wire::new(PinRef::new(1, "out"), PinRef::new(3, "in0"));
        assert!(c.insert_wire(w.clone()));
        let rev = Wire::new(PinRef::new(3, "in0"), PinRef::new(1, "out"));
        assert!(!c.insert_wire(rev), "duplicate in reverse direction");
        assert!(c.remove_wire(&w));
    }

    #[test]
    fn recursion_detected() {
        let mut a = Circuit::new("a");
        let mut sub = comp(1, Kind::Subcircuit);
        sub.props.circuit = "b".into();
        a.components.push(sub);
        let mut b = Circuit::new("b");
        let mut sub = comp(1, Kind::Subcircuit);
        sub.props.circuit = "a".into();
        b.components.push(sub);
        let p = Project { circuits: vec![a, b], libraries: vec![] };
        assert_eq!(p.check_hierarchy("a"), Err(ProjectError::Recursion(vec!["a".into(), "b".into(), "a".into()])));
        assert!(p.would_recurse("a", "b"));
    }

    #[test]
    fn missing_subcircuit() {
        let mut a = Circuit::new("a");
        let mut sub = comp(1, Kind::Subcircuit);
        sub.props.circuit = "ghost".into();
        a.components.push(sub);
        let p = Project::new(a);
        assert!(matches!(p.check_hierarchy("a"), Err(ProjectError::MissingCircuit { .. })));
    }
}
