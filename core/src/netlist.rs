//! Compilation of a schematic into a flat netlist.
//!
//! Subcircuits are expanded recursively: the `Input`/`Output` components of a
//! nested circuit are not instantiated but become aliases of the instance's
//! terminals. Pins joined by wires (directly or through terminals) or by
//! tunnels with the same label in the same instance are merged with a
//! union-find into numbered nets.

use std::collections::HashMap;

use crate::circuit::{CompId, Project, ProjectError};
use crate::component::eval::{Prim, State};
use crate::component::layout::terminals;
use crate::component::{Kind, PinDir};

pub type NetId = u32;
/// Marks an input or output that is not attached to any net.
pub const NO_NET: NetId = u32::MAX;

/// A primitive component of the flattened circuit.
#[derive(Clone, Debug)]
pub struct FlatComp {
    pub prim: Prim,
    pub kind: Kind,
    /// Instance path of the containing circuit (ids of the subcircuit
    /// components from the top level down) — empty for the top level.
    pub inst: Box<[CompId]>,
    /// Id of the component within its circuit.
    pub id: CompId,
    /// Hierarchical name, e.g. `alu/adder0/xor3`.
    pub path: String,
    /// Range into [`Netlist::in_nets`].
    pub ins: std::ops::Range<u32>,
    /// Range into [`Netlist::out_nets`].
    pub outs: std::ops::Range<u32>,
    /// Propagation delay override in ticks (`None` = simulator default).
    pub delay: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct Net {
    pub width: u8,
    /// Global output indices driving this net.
    pub drivers: Vec<u32>,
    /// Components reading this net (deduplicated, ascending).
    pub readers: Vec<u32>,
    /// Hierarchical name, from labels of probes and pins when available.
    pub name: String,
    /// Pins of different widths are connected: the net always reads `X`.
    pub width_error: bool,
}

/// Mapping from the pins of one circuit instance to nets and flat components.
#[derive(Clone, Debug, Default)]
pub struct Instance {
    pub circuit: String,
    /// Net of every pin, indexed by the component's layout pin index.
    pub pins: HashMap<CompId, Vec<NetId>>,
    /// Flat component index of every primitive component.
    pub comps: HashMap<CompId, u32>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum DiagnosticKind {
    WidthMismatch { widths: Vec<u8> },
    UnknownPin { comp: CompId, pin: String },
    MultipleDrivers,
    LoneTunnel { comp: CompId, label: String },
}

/// A non-fatal problem found during compilation.
#[derive(Clone, PartialEq, Debug)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub inst: Box<[CompId]>,
    pub net: Option<NetId>,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct Netlist {
    pub comps: Vec<FlatComp>,
    /// Initial state of every component.
    pub states: Vec<State>,
    pub in_nets: Vec<NetId>,
    pub in_widths: Vec<u8>,
    pub out_nets: Vec<NetId>,
    pub out_widths: Vec<u8>,
    /// Owning component of every global output.
    pub out_comp: Vec<u32>,
    pub nets: Vec<Net>,
    pub instances: HashMap<Box<[CompId]>, Instance>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Netlist {
    pub fn instance(&self, inst: &[CompId]) -> Option<&Instance> {
        self.instances.get(inst)
    }

    /// Net of pin `pin_index` (layout order) of component `id` in `inst`.
    pub fn pin_net(&self, inst: &[CompId], id: CompId, pin_index: usize) -> Option<NetId> {
        let n = *self.instance(inst)?.pins.get(&id)?.get(pin_index)?;
        (n != NO_NET).then_some(n)
    }

    /// Flat index of component `id` in `inst`.
    pub fn comp_index(&self, inst: &[CompId], id: CompId) -> Option<u32> {
        self.instance(inst)?.comps.get(&id).copied()
    }

    /// Finds a net by hierarchical name.
    pub fn net_by_name(&self, name: &str) -> Option<NetId> {
        self.nets.iter().position(|n| n.name == name).map(|i| i as NetId)
    }

    /// Finds a component by hierarchical name.
    pub fn comp_by_path(&self, path: &str) -> Option<u32> {
        self.comps.iter().position(|c| c.path == path).map(|i| i as u32)
    }

    pub fn comp_inputs(&self, c: u32) -> &[NetId] {
        let r = &self.comps[c as usize].ins;
        &self.in_nets[r.start as usize..r.end as usize]
    }

    pub fn comp_outputs(&self, c: u32) -> &[NetId] {
        let r = &self.comps[c as usize].outs;
        &self.out_nets[r.start as usize..r.end as usize]
    }
}

/// Circuit name, pin nodes and flat component indices of an instance.
type PendingInstance = (String, HashMap<CompId, Vec<u32>>, HashMap<CompId, u32>);

struct PendingComp {
    comp: FlatComp,
    state: State,
    in_nodes: Vec<u32>,
    out_nodes: Vec<u32>,
}

struct Builder<'a> {
    project: &'a Project,
    parent: Vec<u32>,
    /// Width of every pin node.
    nodes: Vec<u8>,
    comps: Vec<PendingComp>,
    instances: HashMap<Box<[CompId]>, PendingInstance>,
    /// Name candidates: node → (depth, name).
    names: Vec<(u32, usize, String)>,
    diagnostics: Vec<Diagnostic>,
    /// Circuits whose lone tunnels were reported (once, not per instance).
    tunnels_checked: std::collections::HashSet<String>,
}

impl Builder<'_> {
    fn node(&mut self, width: u8) -> u32 {
        let id = self.nodes.len() as u32;
        self.nodes.push(width);
        self.parent.push(id);
        id
    }

    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let p = self.parent[x as usize];
            self.parent[x as usize] = self.parent[p as usize];
            x = p;
        }
        x
    }

    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            // Keep the smaller id as root, for determinism.
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            self.parent[hi as usize] = lo;
        }
    }

    /// Expands `circuit` as instance `inst`. `terms` maps the ids of the
    /// circuit's Input/Output components to the terminal nodes of the
    /// enclosing instance (`None` at the top level).
    fn expand(&mut self, circuit_name: &str, inst: Vec<CompId>, prefix: &str, terms: Option<&HashMap<CompId, u32>>) {
        let project = self.project;
        let circuit = project.circuit(circuit_name).expect("hierarchy checked");
        let depth = inst.len();
        let mut local: HashMap<CompId, Vec<u32>> = HashMap::new();
        let mut comp_idx: HashMap<CompId, u32> = HashMap::new();
        // Tunnel pin nodes by label, in declaration order.
        let mut tunnels: Vec<(&str, CompId, u32)> = Vec::new();

        for c in &circuit.components {
            if c.kind.is_annotation() {
                continue;
            }
            let layout = project.layout(c);
            let label = c.props.label.trim();

            if let Some(node) = terms.and_then(|t| t.get(&c.id))
                && matches!(c.kind, Kind::Input | Kind::Output)
            {
                local.insert(c.id, vec![*node]);
                if !label.is_empty() {
                    self.names.push((*node, depth, format!("{prefix}{label}")));
                }
                continue;
            }

            if c.kind == Kind::Tunnel {
                let node = self.node(c.props.width());
                if !label.is_empty() {
                    self.names.push((node, depth, format!("{prefix}{label}")));
                }
                tunnels.push((label, c.id, node));
                local.insert(c.id, vec![node]);
                continue;
            }

            if c.kind == Kind::Subcircuit {
                let sub_name = c.props.circuit.as_str();
                let sub = project.circuit(sub_name).expect("hierarchy checked");
                let nodes: Vec<u32> = layout.pins.iter().map(|p| self.node(p.width)).collect();
                let (ins, outs) = terminals(sub);
                let map: HashMap<CompId, u32> =
                    ins.iter().chain(outs.iter()).zip(nodes.iter()).map(|((_, _, id), n)| (*id, *n)).collect();
                let inst_name = if label.is_empty() { format!("{sub_name}{}", c.id) } else { label.to_string() };
                let mut child = inst.clone();
                child.push(c.id);
                self.expand(sub_name, child, &format!("{prefix}{inst_name}/"), Some(&map));
                local.insert(c.id, nodes);
                continue;
            }

            let Some((prim, state)) = Prim::from_component(c) else { continue };
            let ci = self.comps.len() as u32;
            let mut nodes = Vec::with_capacity(layout.pins.len());
            let (mut in_nodes, mut out_nodes) = (Vec::new(), Vec::new());
            for p in &layout.pins {
                let n = match p.dir {
                    PinDir::In => {
                        let n = self.node(p.width);
                        in_nodes.push(n);
                        n
                    }
                    PinDir::Out => {
                        let n = self.node(p.width);
                        out_nodes.push(n);
                        n
                    }
                };
                nodes.push(n);
            }
            let name = if label.is_empty() { format!("{}{}", kind_slug(c.kind), c.id) } else { label.to_string() };
            if !label.is_empty()
                && matches!(c.kind, Kind::Input | Kind::Output | Kind::Probe | Kind::Clock | Kind::Button | Kind::Led)
            {
                self.names.push((nodes[0], depth, format!("{prefix}{label}")));
            }
            for (p, &n) in layout.pins.iter().zip(&nodes) {
                if p.dir == PinDir::Out {
                    let pin_name = if layout.outputs().count() == 1 {
                        format!("{prefix}{name}")
                    } else {
                        format!("{prefix}{name}.{}", p.name)
                    };
                    self.names.push((n, depth + 1, pin_name));
                }
            }
            let delay = (c.props.delay > 0 && !prim.is_source()).then_some(c.props.delay);
            self.comps.push(PendingComp {
                comp: FlatComp {
                    prim,
                    kind: c.kind,
                    inst: inst.clone().into(),
                    id: c.id,
                    path: format!("{prefix}{name}"),
                    ins: 0..0,
                    outs: 0..0,
                    delay,
                },
                state,
                in_nodes,
                out_nodes,
            });
            comp_idx.insert(c.id, ci);
            local.insert(c.id, nodes);
        }

        for w in &circuit.wires {
            let mut ends = [None, None];
            for (k, pr) in [&w.from, &w.to].into_iter().enumerate() {
                let node = circuit.component(pr.0).and_then(|c| {
                    let l = project.layout(c);
                    let (i, _) = l.pin(&pr.1)?;
                    local.get(&pr.0)?.get(i).copied()
                });
                if node.is_none() {
                    self.diagnostics.push(Diagnostic {
                        kind: DiagnosticKind::UnknownPin { comp: pr.0, pin: pr.1.clone() },
                        inst: inst.clone().into(),
                        net: None,
                        message: format!(
                            "{}: a wire is attached to pin \"{}\" of component {}, which does not exist",
                            circuit.name, pr.1, pr.0
                        ),
                    });
                }
                ends[k] = node;
            }
            if let [Some(a), Some(b)] = ends {
                self.union(a, b);
            }
        }

        // Label → (first node, number of tunnels).
        let mut groups: HashMap<&str, (u32, usize)> = HashMap::new();
        for &(label, _, node) in tunnels.iter().filter(|t| !t.0.is_empty()) {
            let g = groups.entry(label).or_insert((node, 0));
            g.1 += 1;
            let first = g.0;
            self.union(first, node);
        }
        let report = self.tunnels_checked.insert(circuit.name.clone());
        for (label, id, _) in tunnels {
            if report && (label.is_empty() || groups[label].1 == 1) {
                let message = if label.is_empty() {
                    format!("{}: tunnel {id} has no label", circuit.name)
                } else {
                    format!("{}: tunnel \"{label}\" has no other tunnel with the same label", circuit.name)
                };
                self.diagnostics.push(Diagnostic {
                    kind: DiagnosticKind::LoneTunnel { comp: id, label: label.to_string() },
                    inst: inst.clone().into(),
                    net: None,
                    message,
                });
            }
        }

        self.instances.insert(inst.into(), (circuit.name.clone(), local, comp_idx));
    }
}

fn kind_slug(k: Kind) -> String {
    format!("{k:?}").to_lowercase()
}

/// Compiles the circuit `root` of `project` into a flat netlist.
pub fn compile(project: &Project, root: &str) -> Result<Netlist, ProjectError> {
    project.check_hierarchy(root)?;
    let mut b = Builder {
        project,
        parent: Vec::new(),
        nodes: Vec::new(),
        comps: Vec::new(),
        instances: HashMap::new(),
        names: Vec::new(),
        diagnostics: Vec::new(),
        tunnels_checked: Default::default(),
    };
    b.expand(root, Vec::new(), "", None);

    // Number the nets in order of their root node.
    let n_nodes = b.nodes.len();
    let mut net_of_root: HashMap<u32, NetId> = HashMap::new();
    let mut node_net = vec![NO_NET; n_nodes];
    let mut nets: Vec<Net> = Vec::new();
    let mut widths: Vec<Vec<u8>> = Vec::new();
    for i in 0..n_nodes as u32 {
        let r = b.find(i);
        let id = *net_of_root.entry(r).or_insert_with(|| {
            nets.push(Net::default());
            widths.push(Vec::new());
            (nets.len() - 1) as NetId
        });
        node_net[i as usize] = id;
        widths[id as usize].push(b.nodes[i as usize]);
    }

    // Net names: shallowest, then first declared.
    let mut best: Vec<Option<(usize, usize)>> = vec![None; nets.len()];
    for (k, (node, depth, _)) in b.names.iter().enumerate() {
        let net = node_net[*node as usize] as usize;
        if best[net].is_none_or(|(d, _)| *depth < d) {
            best[net] = Some((*depth, k));
        }
    }
    for (i, net) in nets.iter_mut().enumerate() {
        net.name = match best[i] {
            Some((_, k)) => b.names[k].2.clone(),
            None => format!("net{i}"),
        };
    }

    for (i, w) in widths.iter().enumerate() {
        let max = *w.iter().max().unwrap_or(&1);
        nets[i].width = max;
        if w.iter().any(|&x| x != max) {
            let mut ws = w.clone();
            ws.sort_unstable();
            ws.dedup();
            nets[i].width_error = true;
            b.diagnostics.push(Diagnostic {
                kind: DiagnosticKind::WidthMismatch { widths: ws.clone() },
                inst: Box::new([]),
                net: Some(i as NetId),
                message: format!("Net \"{}\" connects pins of different widths: {ws:?}", nets[i].name),
            });
        }
    }

    // Drivers are known before readers so that gates can drop floating
    // inputs.
    let mut nl = Netlist::default();
    for (ci, pc) in b.comps.iter().enumerate() {
        for &n in &pc.out_nodes {
            let g = nl.out_nets.len() as u32;
            let net = node_net[n as usize];
            nl.out_nets.push(net);
            nl.out_widths.push(b.nodes[n as usize]);
            nl.out_comp.push(ci as u32);
            nets[net as usize].drivers.push(g);
        }
    }
    let mut out_cursor = 0u32;
    for (ci, pc) in b.comps.into_iter().enumerate() {
        let mut comp = pc.comp;
        let start = nl.in_nets.len() as u32;
        let drop_floating = matches!(comp.prim, Prim::Gate { .. });
        for &n in &pc.in_nodes {
            let net = node_net[n as usize];
            if drop_floating && nets[net as usize].drivers.is_empty() {
                continue;
            }
            nl.in_nets.push(net);
            nl.in_widths.push(b.nodes[n as usize]);
            let readers = &mut nets[net as usize].readers;
            if readers.last() != Some(&(ci as u32)) {
                readers.push(ci as u32);
            }
        }
        comp.ins = start..nl.in_nets.len() as u32;
        let n_out = pc.out_nodes.len() as u32;
        comp.outs = out_cursor..out_cursor + n_out;
        out_cursor += n_out;
        nl.comps.push(comp);
        nl.states.push(pc.state);
    }

    for (i, net) in nets.iter().enumerate() {
        let strong = net
            .drivers
            .iter()
            .filter(|&&g| !matches!(nl.comps[nl.out_comp[g as usize] as usize].prim, Prim::TriBuffer))
            .count();
        if strong > 1 {
            b.diagnostics.push(Diagnostic {
                kind: DiagnosticKind::MultipleDrivers,
                inst: Box::new([]),
                net: Some(i as NetId),
                message: format!("Net \"{}\" has {strong} active drivers", net.name),
            });
        }
    }

    nl.nets = nets;
    nl.instances = b
        .instances
        .into_iter()
        .map(|(k, (circuit, pins, comps))| {
            let pins = pins
                .into_iter()
                .map(|(id, nodes)| (id, nodes.iter().map(|&n| node_net[n as usize]).collect()))
                .collect();
            (k, Instance { circuit, pins, comps })
        })
        .collect();
    nl.diagnostics = b.diagnostics;
    Ok(nl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::Circuit;
    use crate::test_util::{add, wire};

    fn half_adder() -> Circuit {
        let mut c = Circuit::new("half_adder");
        let a = add(&mut c, Kind::Input, "A");
        let b = add(&mut c, Kind::Input, "B");
        let x = add(&mut c, Kind::Xor, "");
        let n = add(&mut c, Kind::And, "");
        let s = add(&mut c, Kind::Output, "SUM");
        let k = add(&mut c, Kind::Output, "CARRY");
        wire(&mut c, a, "out", x, "in0");
        wire(&mut c, b, "out", x, "in1");
        wire(&mut c, a, "out", n, "in0");
        wire(&mut c, b, "out", n, "in1");
        wire(&mut c, x, "out", s, "in");
        wire(&mut c, n, "out", k, "in");
        c
    }

    #[test]
    fn flat_half_adder() {
        let p = Project::new(half_adder());
        let nl = compile(&p, "half_adder").unwrap();
        assert_eq!(nl.comps.len(), 6);
        assert_eq!(nl.nets.len(), 4);
        assert!(nl.diagnostics.is_empty(), "{:?}", nl.diagnostics);
        let a = nl.net_by_name("A").unwrap();
        assert_eq!(nl.nets[a as usize].readers.len(), 2);
        assert!(nl.net_by_name("SUM").is_some());
    }

    #[test]
    fn nested_paths() {
        let mut top = Circuit::new("top");
        let i1 = add(&mut top, Kind::Input, "x");
        let i2 = add(&mut top, Kind::Input, "y");
        let h = add(&mut top, Kind::Subcircuit, "ha0");
        top.component_mut(h).unwrap().props.circuit = "half_adder".into();
        let o = add(&mut top, Kind::Output, "c");
        wire(&mut top, i1, "out", h, "A");
        wire(&mut top, i2, "out", h, "B");
        wire(&mut top, h, "CARRY", o, "in");
        let p = Project { circuits: vec![top, half_adder()], libraries: vec![], hdl: Default::default() };
        let nl = compile(&p, "top").unwrap();
        // 3 top-level primitives + XOR and AND (inner I/O pins are aliases).
        assert_eq!(nl.comps.len(), 5);
        assert!(nl.comp_by_path("ha0/xor3").is_some());
        // The top-level label wins over the inner one.
        assert!(nl.net_by_name("c").is_some());
        assert!(nl.net_by_name("ha0/SUM").is_some());
        let inner = nl.instance(&[h]).unwrap();
        assert_eq!(inner.circuit, "half_adder");
        assert_eq!(nl.pin_net(&[], i1, 0), nl.pin_net(&[h], 1, 0));
    }

    #[test]
    fn width_mismatch_reported() {
        let mut c = Circuit::new("w");
        let a = add(&mut c, Kind::Input, "a");
        let o = add(&mut c, Kind::Output, "o");
        c.component_mut(o).unwrap().props.width = 4;
        wire(&mut c, a, "out", o, "in");
        let nl = compile(&Project::new(c), "w").unwrap();
        assert!(matches!(nl.diagnostics[0].kind, DiagnosticKind::WidthMismatch { .. }));
    }

    #[test]
    fn unknown_pin_reported() {
        let mut c = Circuit::new("u");
        let a = add(&mut c, Kind::Input, "a");
        wire(&mut c, a, "out", 99, "in");
        let nl = compile(&Project::new(c), "u").unwrap();
        assert!(matches!(nl.diagnostics[0].kind, DiagnosticKind::UnknownPin { .. }));
    }

    #[test]
    fn tunnels_name_their_net() {
        let mut c = Circuit::new("t");
        let k = add(&mut c, Kind::Constant, "");
        let n = add(&mut c, Kind::Not, "");
        let t1 = add(&mut c, Kind::Tunnel, "clk");
        let t2 = add(&mut c, Kind::Tunnel, "clk");
        wire(&mut c, k, "out", t1, "in");
        wire(&mut c, t2, "in", n, "in");
        let nl = compile(&Project::new(c), "t").unwrap();
        assert!(nl.diagnostics.is_empty(), "{:?}", nl.diagnostics);
        let clk = nl.net_by_name("clk").unwrap();
        assert_eq!(nl.pin_net(&[], k, 0), Some(clk));
        assert_eq!(nl.pin_net(&[], n, 0), Some(clk));
    }

    #[test]
    fn lone_tunnels_reported_once_per_circuit() {
        let mut inner = Circuit::new("inner");
        add(&mut inner, Kind::Tunnel, "rst");
        add(&mut inner, Kind::Tunnel, "");
        let mut top = Circuit::new("top");
        for _ in 0..2 {
            let s = add(&mut top, Kind::Subcircuit, "");
            top.component_mut(s).unwrap().props.circuit = "inner".into();
        }
        let p = Project { circuits: vec![top, inner], libraries: vec![], hdl: Default::default() };
        let nl = compile(&p, "top").unwrap();
        let labels: Vec<_> = nl
            .diagnostics
            .iter()
            .map(|d| match &d.kind {
                DiagnosticKind::LoneTunnel { label, .. } => label.as_str(),
                k => panic!("unexpected {k:?}"),
            })
            .collect();
        assert_eq!(labels, ["rst", ""]);
    }

    #[test]
    fn tunnel_width_mismatch_reported() {
        let mut c = Circuit::new("t");
        add(&mut c, Kind::Tunnel, "bus");
        let wide = add(&mut c, Kind::Tunnel, "bus");
        c.component_mut(wide).unwrap().props.width = 8;
        let nl = compile(&Project::new(c), "t").unwrap();
        assert!(matches!(nl.diagnostics[..], [Diagnostic { kind: DiagnosticKind::WidthMismatch { .. }, .. }]));
    }

    #[test]
    fn recursion_rejected() {
        let mut a = Circuit::new("a");
        let s = add(&mut a, Kind::Subcircuit, "");
        a.component_mut(s).unwrap().props.circuit = "a".into();
        let err = compile(&Project::new(a), "a").unwrap_err();
        assert!(matches!(err, ProjectError::Recursion(_)));
    }
}
