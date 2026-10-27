//! Editing commands and unlimited undo/redo.
//!
//! A [`Command`] is a list of invertible [`Op`]s (differential snapshots:
//! only the touched components and wires are stored, before and after).
//! Multi-element operations — moving a selection, deleting, pasting — are a
//! single command and undo in one step.

use std::collections::HashSet;

use crate::circuit::{Circuit, CompId, Component, HdlConfig, PinRef, Project, Wire};

#[derive(Clone, PartialEq, Debug)]
pub enum Op {
    AddComp(Component),
    RemoveComp(Component),
    UpdateComp {
        before: Box<Component>,
        after: Box<Component>,
    },
    AddWire(Wire),
    RemoveWire(Wire),
    /// Inserts a circuit at an index of the project.
    AddCircuit(usize, Box<Circuit>),
    RemoveCircuit(usize, Box<Circuit>),
    /// Renames a circuit and every reference to it.
    RenameCircuit {
        from: String,
        to: String,
    },
    /// Replaces the HDL settings of the project.
    SetHdl {
        before: Box<HdlConfig>,
        after: Box<HdlConfig>,
    },
}

impl Op {
    pub fn inverse(&self) -> Op {
        match self {
            Op::AddComp(c) => Op::RemoveComp(c.clone()),
            Op::RemoveComp(c) => Op::AddComp(c.clone()),
            Op::UpdateComp { before, after } => Op::UpdateComp { before: after.clone(), after: before.clone() },
            Op::AddWire(w) => Op::RemoveWire(w.clone()),
            Op::RemoveWire(w) => Op::AddWire(w.clone()),
            Op::AddCircuit(i, c) => Op::RemoveCircuit(*i, c.clone()),
            Op::RemoveCircuit(i, c) => Op::AddCircuit(*i, c.clone()),
            Op::RenameCircuit { from, to } => Op::RenameCircuit { from: to.clone(), to: from.clone() },
            Op::SetHdl { before, after } => Op::SetHdl { before: after.clone(), after: before.clone() },
        }
    }

    fn apply(&self, project: &mut Project, circuit: &str) {
        match self {
            Op::AddCircuit(i, c) => {
                let i = (*i).min(project.circuits.len());
                project.circuits.insert(i, (**c).clone());
            }
            Op::RemoveCircuit(_, c) => {
                if let Some(i) = project.circuit_index(&c.name) {
                    project.circuits.remove(i);
                }
            }
            Op::RenameCircuit { from, to } => {
                for c in &mut project.circuits {
                    if c.name == *from {
                        c.name = to.clone();
                    }
                    for comp in &mut c.components {
                        if comp.kind == crate::component::Kind::Subcircuit && comp.props.circuit == *from {
                            comp.props.circuit = to.clone();
                        }
                    }
                }
            }
            Op::SetHdl { after, .. } => project.hdl = (**after).clone(),
            _ => {
                let Some(c) = project.circuit_mut(circuit) else { return };
                match self {
                    Op::AddComp(x) => c.insert_component(x.clone()),
                    Op::RemoveComp(x) => {
                        c.remove_component(x.id);
                    }
                    Op::UpdateComp { after, .. } => c.insert_component((**after).clone()),
                    Op::AddWire(w) => {
                        c.insert_wire(w.clone());
                    }
                    Op::RemoveWire(w) => {
                        c.remove_wire(w);
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Command {
    pub label: String,
    /// Circuit the component and wire operations apply to.
    pub circuit: String,
    pub ops: Vec<Op>,
    /// Consecutive commands with the same key are merged into one undo step
    /// (e.g. typing a label one character at a time).
    pub merge: Option<String>,
}

impl Command {
    pub fn new(label: impl Into<String>, circuit: impl Into<String>) -> Self {
        Command { label: label.into(), circuit: circuit.into(), ops: Vec::new(), merge: None }
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn push(&mut self, op: Op) -> &mut Self {
        self.ops.push(op);
        self
    }

    pub fn apply(&self, project: &mut Project) {
        for op in &self.ops {
            op.apply(project, &self.circuit);
        }
    }

    /// The circuit name after this command (it changes when renaming).
    pub fn circuit_after(&self) -> &str {
        self.ops
            .iter()
            .rev()
            .find_map(|op| match op {
                Op::RenameCircuit { from, to } if *from == self.circuit => Some(to.as_str()),
                _ => None,
            })
            .unwrap_or(&self.circuit)
    }

    pub fn revert(&self, project: &mut Project) {
        let circuit = self.circuit_after().to_string();
        for op in self.ops.iter().rev() {
            op.inverse().apply(project, &circuit);
        }
    }

    // ---- builders ------------------------------------------------------------

    /// Adds components and wires.
    pub fn add(circuit: &Circuit, comps: Vec<Component>, wires: Vec<Wire>) -> Command {
        let mut cmd = Command::new("Add", &circuit.name);
        cmd.ops.extend(comps.into_iter().map(Op::AddComp));
        for w in wires {
            if !circuit.wires.iter().any(|x| x.same_ends(&w)) {
                cmd.ops.push(Op::AddWire(w));
            }
        }
        cmd
    }

    /// Deletes components (with every wire attached to them) and wires.
    pub fn delete(circuit: &Circuit, ids: &[CompId], wires: &[Wire]) -> Command {
        let ids: HashSet<CompId> = ids.iter().copied().collect();
        let mut cmd = Command::new("Delete", &circuit.name);
        for w in &circuit.wires {
            if ids.contains(&w.from.0) || ids.contains(&w.to.0) || wires.contains(w) {
                cmd.ops.push(Op::RemoveWire(w.clone()));
            }
        }
        for c in &circuit.components {
            if ids.contains(&c.id) {
                cmd.ops.push(Op::RemoveComp(c.clone()));
            }
        }
        cmd
    }

    /// Moves components by `(dx, dy)`. Waypoints of wires between two moved
    /// components move along.
    pub fn translate(circuit: &Circuit, ids: &[CompId], dx: i32, dy: i32) -> Command {
        let mut cmd = Command::new("Move", &circuit.name);
        if dx == 0 && dy == 0 {
            return cmd;
        }
        let set: HashSet<CompId> = ids.iter().copied().collect();
        for c in &circuit.components {
            if set.contains(&c.id) {
                let mut after = c.clone();
                after.pos = (c.pos.0 + dx, c.pos.1 + dy);
                cmd.ops.push(Op::UpdateComp { before: Box::new(c.clone()), after: Box::new(after) });
            }
        }
        for w in &circuit.wires {
            if !w.path.is_empty() && set.contains(&w.from.0) && set.contains(&w.to.0) {
                let mut moved = w.clone();
                for p in &mut moved.path {
                    *p = (p.0 + dx, p.1 + dy);
                }
                cmd.ops.push(Op::RemoveWire(w.clone()));
                cmd.ops.push(Op::AddWire(moved));
            }
        }
        cmd
    }

    /// Replaces components with edited versions, removing wires attached to
    /// pins that no longer exist.
    pub fn update(project: &Project, circuit: &Circuit, edited: Vec<Component>) -> Command {
        let mut cmd = Command::new("Edit", &circuit.name);
        let mut removed = Vec::new();
        for after in edited {
            let Some(before) = circuit.component(after.id) else { continue };
            if *before == after {
                continue;
            }
            let layout = project.layout(&after);
            for w in &circuit.wires {
                let dangling = [&w.from, &w.to].iter().any(|p| p.0 == after.id && layout.pin(&p.1).is_none());
                if dangling && !removed.contains(w) {
                    removed.push(w.clone());
                }
            }
            cmd.ops.push(Op::UpdateComp { before: Box::new(before.clone()), after: Box::new(after) });
        }
        cmd.ops.extend(removed.into_iter().map(Op::RemoveWire));
        cmd
    }
}

/// Components and wires copied to the clipboard.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Clip {
    pub components: Vec<Component>,
    pub wires: Vec<Wire>,
}

impl Clip {
    /// Copies the selected components and the wires among them.
    pub fn copy(circuit: &Circuit, ids: &[CompId]) -> Clip {
        let set: HashSet<CompId> = ids.iter().copied().collect();
        Clip {
            components: circuit.components.iter().filter(|c| set.contains(&c.id)).cloned().collect(),
            wires: circuit.wires.iter().filter(|w| set.contains(&w.from.0) && set.contains(&w.to.0)).cloned().collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    /// Builds the command that pastes the clip shifted by `offset`, with
    /// fresh ids. Returns the command and the new ids.
    pub fn paste(&self, circuit: &Circuit, offset: (i32, i32)) -> (Command, Vec<CompId>) {
        let mut map = std::collections::HashMap::new();
        let mut comps = Vec::new();
        for (next, c) in (circuit.next_id()..).zip(&self.components) {
            let mut n = c.clone();
            n.id = next;
            n.pos = (c.pos.0 + offset.0, c.pos.1 + offset.1);
            map.insert(c.id, next);
            comps.push(n);
        }
        let wires = self
            .wires
            .iter()
            .map(|w| Wire {
                from: PinRef(map[&w.from.0], w.from.1.clone()),
                to: PinRef(map[&w.to.0], w.to.1.clone()),
                path: w.path.iter().map(|p| (p.0 + offset.0, p.1 + offset.1)).collect(),
            })
            .collect();
        let ids = comps.iter().map(|c| c.id).collect();
        let mut cmd = Command::add(circuit, comps, wires);
        cmd.label = "Paste".into();
        (cmd, ids)
    }
}

/// Unlimited undo/redo stacks for a project.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<Command>,
    redo: Vec<Command>,
    /// Length of the undo stack when last saved (`None`: unreachable state).
    saved: Option<usize>,
}

impl History {
    pub fn new() -> Self {
        History { saved: Some(0), ..Default::default() }
    }

    /// Applies a command and records it. Empty commands are ignored.
    pub fn execute(&mut self, project: &mut Project, cmd: Command) -> bool {
        if cmd.is_empty() {
            return false;
        }
        cmd.apply(project);
        if self.saved.is_some_and(|s| s > self.undo.len()) {
            self.saved = None;
        }
        let at_save_point = self.saved == Some(self.undo.len());
        if let Some(last) = self.undo.last_mut()
            && cmd.merge.is_some()
            && last.merge == cmd.merge
            && last.circuit == cmd.circuit
            && self.redo.is_empty()
            && !at_save_point
        {
            last.ops.extend(cmd.ops);
            return true;
        }
        self.undo.push(cmd);
        self.redo.clear();
        true
    }

    /// Undoes the last command; returns it.
    pub fn undo(&mut self, project: &mut Project) -> Option<&Command> {
        let cmd = self.undo.pop()?;
        cmd.revert(project);
        self.redo.push(cmd);
        self.redo.last()
    }

    pub fn redo(&mut self, project: &mut Project) -> Option<&Command> {
        let cmd = self.redo.pop()?;
        cmd.apply(project);
        self.undo.push(cmd);
        self.undo.last()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|c| c.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|c| c.label.as_str())
    }

    pub fn mark_saved(&mut self) {
        self.saved = Some(self.undo.len());
    }

    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.undo.len())
    }

    pub fn len(&self) -> usize {
        self.undo.len()
    }

    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Kind;
    use crate::test_util::{add, wire};

    fn sample() -> Project {
        let mut c = Circuit::new("main");
        let a = add(&mut c, Kind::Input, "a");
        let g = add(&mut c, Kind::And, "");
        let o = add(&mut c, Kind::Output, "o");
        wire(&mut c, a, "out", g, "in0");
        wire(&mut c, g, "out", o, "in");
        Project::new(c)
    }

    #[test]
    fn delete_undo_redo() {
        let mut p = sample();
        let original = p.clone();
        let mut h = History::new();
        let cmd = Command::delete(p.main(), &[2], &[]);
        assert!(h.execute(&mut p, cmd));
        assert_eq!(p.main().components.len(), 2);
        assert!(p.main().wires.is_empty());
        assert!(h.is_dirty());
        h.undo(&mut p);
        assert_eq!(p, original);
        assert!(!h.is_dirty());
        h.redo(&mut p);
        assert_eq!(p.main().components.len(), 2);
    }

    #[test]
    fn unlimited_multi_element_history() {
        let mut p = sample();
        let original = p.clone();
        let mut h = History::new();
        for i in 0..500 {
            let cmd = Command::translate(p.main(), &[1, 2, 3], 1, i % 3);
            h.execute(&mut p, cmd);
        }
        assert_eq!(h.len(), 500);
        while h.undo(&mut p).is_some() {}
        assert_eq!(p, original);
    }

    #[test]
    fn update_removes_dangling_wires() {
        let mut p = sample();
        let mut h = History::new();
        let m = add(&mut p.circuits[0], Kind::Mux, "");
        wire(&mut p.circuits[0], 1, "out", m, "in3");
        let mut edited = p.main().component(m).unwrap().clone();
        edited.props.select = 2;
        // With 2 select bits "in3" exists: nothing to remove.
        let cmd = Command::update(&p, p.main(), vec![edited.clone()]);
        assert_eq!(cmd.ops.len(), 1);
        h.execute(&mut p, cmd);
        edited.props.select = 1;
        let cmd = Command::update(&p, p.main(), vec![edited]);
        assert!(cmd.ops.iter().any(|o| matches!(o, Op::RemoveWire(_))));
    }

    #[test]
    fn coalesced_edits_undo_together() {
        let mut p = sample();
        let original = p.clone();
        let mut h = History::new();
        for label in ["x", "xy", "xyz"] {
            let mut c = p.main().component(1).unwrap().clone();
            c.props.label = label.into();
            let mut cmd = Command::update(&p, p.main(), vec![c]);
            cmd.merge = Some("label 1".into());
            h.execute(&mut p, cmd);
        }
        assert_eq!(h.len(), 1);
        assert_eq!(p.main().component(1).unwrap().props.label, "xyz");
        h.undo(&mut p);
        assert_eq!(p, original);
    }

    #[test]
    fn paste_assigns_fresh_ids() {
        let p = sample();
        let clip = Clip::copy(p.main(), &[1, 2]);
        assert_eq!(clip.wires.len(), 1);
        let (cmd, ids) = clip.paste(p.main(), (5, 5));
        assert_eq!(ids, vec![4, 5]);
        let mut p2 = p.clone();
        let mut h = History::new();
        h.execute(&mut p2, cmd);
        assert_eq!(p2.main().components.len(), 5);
        assert!(p2.main().wires.iter().any(|w| w.from.0 == 4 && w.to.0 == 5));
    }

    #[test]
    fn rename_circuit_updates_references() {
        let mut p = sample();
        let mut sub = Circuit::new("child");
        add(&mut sub, Kind::Input, "x");
        p.circuits.push(sub);
        let s = add(&mut p.circuits[0], Kind::Subcircuit, "");
        p.circuits[0].component_mut(s).unwrap().props.circuit = "child".into();
        let original = p.clone();
        let mut h = History::new();
        let mut cmd = Command::new("Rename", "child");
        cmd.push(Op::RenameCircuit { from: "child".into(), to: "kid".into() });
        h.execute(&mut p, cmd);
        assert!(p.circuit("kid").is_some());
        assert_eq!(p.main().component(s).unwrap().props.circuit, "kid");
        h.undo(&mut p);
        assert_eq!(p, original);
    }
}
