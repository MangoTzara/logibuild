//! Event-driven simulation engine.
//!
//! Events are output changes ordered by `(time, delta, sequence number)`. All
//! events with the same `(time, delta)` form a batch: they are applied
//! together, the affected nets are resolved, and every component reading a
//! net that changed is evaluated once. Outputs that change are scheduled
//! `delay` ticks later, or in the next delta cycle of the same instant when
//! the delay is zero.
//!
//! Clocks are not queued: the engine knows their next toggle time and fires
//! them before any event of a later instant. Sequence numbers make the whole
//! process deterministic.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::component::eval::{Prim, State};
use crate::netlist::{NO_NET, NetId, Netlist};
use crate::trace::Trace;
use crate::value::Signal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimConfig {
    /// Default propagation delay of components, in ticks. `0` selects the
    /// zero-delay mode where only delta cycles separate cause and effect.
    pub gate_delay: u32,
    /// Delta cycles allowed within one instant before declaring an
    /// oscillation.
    pub delta_limit: u32,
}

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig { gate_delay: 1, delta_limit: 1000 }
    }
}

/// Simulation stopped because the circuit did not settle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Oscillation {
    pub time: u64,
    /// Nets that kept changing in the last delta cycles.
    pub nets: Vec<NetId>,
}

impl std::fmt::Display for Oscillation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Oscillation detected at time {} ({} nets involved)", self.time, self.nets.len())
    }
}

impl std::error::Error for Oscillation {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Some events were processed.
    Progress,
    /// Nothing left to do.
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Key {
    time: u64,
    delta: u32,
}

#[derive(Clone, Copy, Debug)]
struct Event {
    key: Key,
    seq: u64,
    out: u32,
    value: Signal,
}

impl PartialEq for Event {
    fn eq(&self, o: &Self) -> bool {
        (self.key, self.seq) == (o.key, o.seq)
    }
}
impl Eq for Event {}
impl PartialOrd for Event {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Event {
    // Reversed: `BinaryHeap` is a max-heap and we want the earliest event.
    fn cmp(&self, o: &Self) -> Ordering {
        (o.key, o.seq).cmp(&(self.key, self.seq))
    }
}

/// Deltas before the limit during which changing nets are collected for
/// the oscillation report.
const OSC_WINDOW: u32 = 16;

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub events: u64,
    pub evaluations: u64,
}

#[derive(Clone)]
pub struct Simulator {
    nl: Netlist,
    cfg: SimConfig,
    states: Vec<State>,
    out_vals: Vec<Signal>,
    /// Last value scheduled for each output (equals `out_vals` when nothing
    /// is pending).
    out_proj: Vec<Signal>,
    net_vals: Vec<Signal>,
    queue: BinaryHeap<Event>,
    seq: u64,
    now: u64,
    cur: Option<Key>,
    clocks: Vec<u32>,
    clock_next: Vec<u64>,
    net_dirty: Vec<bool>,
    dirty_nets: Vec<NetId>,
    comp_dirty: Vec<bool>,
    dirty_comps: Vec<u32>,
    in_buf: Vec<Signal>,
    out_buf: Vec<Signal>,
    osc: Option<Oscillation>,
    osc_nets: Vec<NetId>,
    /// First delta of the current settling episode: delta cycles are counted
    /// from here for oscillation detection, so that many input changes in
    /// the same instant (zero-delay mode) do not add up.
    delta_base: Key,
    pub trace: Trace,
    pub stats: Stats,
}

impl Simulator {
    pub fn new(netlist: Netlist, cfg: SimConfig) -> Self {
        let n_out = netlist.out_nets.len();
        let n_nets = netlist.nets.len();
        let n_comps = netlist.comps.len();
        let clocks = (0..n_comps as u32)
            .filter(|&c| matches!(netlist.comps[c as usize].prim, Prim::Clock { .. }))
            .collect::<Vec<_>>();
        let mut sim = Simulator {
            states: netlist.states.clone(),
            out_vals: vec![Signal::default(); n_out],
            out_proj: vec![Signal::default(); n_out],
            net_vals: vec![Signal::default(); n_nets],
            queue: BinaryHeap::new(),
            seq: 0,
            now: 0,
            cur: None,
            clock_next: vec![0; clocks.len()],
            clocks,
            net_dirty: vec![false; n_nets],
            dirty_nets: Vec::new(),
            comp_dirty: vec![false; n_comps],
            dirty_comps: Vec::new(),
            in_buf: Vec::new(),
            out_buf: Vec::new(),
            osc: None,
            osc_nets: Vec::new(),
            delta_base: Key { time: 0, delta: 0 },
            trace: Trace::default(),
            stats: Stats::default(),
            nl: netlist,
            cfg,
        };
        sim.initialize(0, true);
        sim
    }

    /// Builds a simulator for a recompiled circuit, keeping the time, the
    /// trace and the state of every component that still exists (same
    /// instance path, id and kind): input values, flip-flops, memories and
    /// current outputs, so that unchanged parts do not glitch.
    pub fn new_from(netlist: Netlist, cfg: SimConfig, old: &Simulator) -> Self {
        let mut sim = Simulator::new(netlist, cfg);
        let mut index = std::collections::HashMap::new();
        for (i, c) in old.nl.comps.iter().enumerate() {
            index.insert((&c.inst, c.id), i);
        }
        for (i, c) in sim.nl.comps.iter().enumerate() {
            let Some(&j) = index.get(&(&c.inst, c.id)) else { continue };
            let oc = &old.nl.comps[j];
            if oc.prim != c.prim {
                continue;
            }
            let (new_st, old_st) = (&mut sim.states[i], &old.states[j]);
            // Constants and ROMs take their value from the (possibly edited)
            // properties.
            let keep = old_st.q.width() == new_st.q.width()
                && old_st.mem.len() == new_st.mem.len()
                && !matches!(c.prim, Prim::Constant | Prim::Rom);
            if keep {
                *new_st = old_st.clone();
            }
            let (no, oo) = (&c.outs, &oc.outs);
            if no.len() == oo.len() {
                for (g, h) in no.clone().zip(oo.clone()) {
                    let v = old.out_vals[h as usize];
                    // Unknown values are recomputed from scratch: carrying an
                    // X into a new feedback loop would mask oscillations.
                    if v.width() == sim.nl.out_widths[g as usize] && v.is_known() {
                        sim.out_vals[g as usize] = v;
                    }
                }
            }
        }
        sim.trace = old.trace.clone();
        let nl = &sim.nl;
        sim.trace.rebind(|s| {
            let net = match &s.key {
                Some((inst, id, pin)) => nl.pin_net(inst, *id, *pin)?,
                None => nl.net_by_name(&s.name)?,
            };
            Some((net, nl.nets[net as usize].width))
        });
        sim.initialize(old.now, false);
        for i in 0..sim.trace.signals().len() {
            let net = sim.trace.signals()[i].net;
            sim.trace.record(net, old.now, sim.net_vals[net as usize]);
        }
        sim
    }

    pub fn netlist(&self) -> &Netlist {
        &self.nl
    }

    pub fn config(&self) -> SimConfig {
        self.cfg
    }

    pub fn set_config(&mut self, cfg: SimConfig) {
        self.cfg = cfg;
    }

    /// Restarts from time 0 with the initial component states.
    pub fn reset(&mut self) {
        self.states = self.nl.states.clone();
        self.initialize(0, true);
        let nets = &self.net_vals;
        self.trace.restart(0, |n| nets.get(n as usize).copied().unwrap_or_default());
    }

    /// Sets the time to `t`, optionally initializes outputs to 0, recomputes
    /// nets and evaluates every component once.
    fn initialize(&mut self, t: u64, zero_outputs: bool) {
        self.queue.clear();
        self.seq = 0;
        self.now = t;
        self.cur = None;
        self.delta_base = Key { time: t, delta: 0 };
        self.osc = None;
        self.osc_nets.clear();
        if zero_outputs {
            for (g, w) in self.nl.out_widths.iter().enumerate() {
                self.out_vals[g] = Signal::zero(*w);
            }
        }
        for (k, &c) in self.clocks.iter().enumerate() {
            let Prim::Clock { high, low } = self.nl.comps[c as usize].prim else { unreachable!() };
            let period = (high + low) as u64;
            let phase = t % period;
            let (q, next) =
                if phase < low as u64 { (false, t - phase + low as u64) } else { (true, t - phase + period) };
            self.states[c as usize].q = Signal::bool(q);
            self.clock_next[k] = next;
        }
        for n in 0..self.nl.nets.len() {
            self.net_vals[n] = self.resolve_net(n as NetId);
        }
        self.relax();
        self.out_proj.clone_from(&self.out_vals);
        for c in 0..self.nl.comps.len() as u32 {
            self.eval_comp(c);
        }
    }

    /// Initial settling by sequential relaxation: components are evaluated
    /// one at a time and their outputs applied immediately. Unlike the
    /// simultaneous updates of the event loop, this breaks the symmetry of
    /// cross-coupled gates (an SR latch built from NANDs starts in a valid
    /// state instead of oscillating), while true oscillators such as a NOT
    /// ring never converge and are then caught by the event loop.
    ///
    /// Component state is not modified: sequential elements are evaluated on
    /// a scratch copy and memories are skipped.
    fn relax(&mut self) {
        const PASSES: usize = 32;
        let mut scratch = State::new(Signal::zero(1));
        for _ in 0..PASSES {
            let mut changed = false;
            for c in 0..self.nl.comps.len() {
                let fc = &self.nl.comps[c];
                if !self.states[c].mem.is_empty() || matches!(fc.prim, Prim::Sink) {
                    continue;
                }
                self.in_buf.clear();
                for k in fc.ins.start..fc.ins.end {
                    let net = self.nl.in_nets[k as usize];
                    let w = self.nl.in_widths[k as usize];
                    let v = if net == NO_NET { Signal::z(w) } else { self.net_vals[net as usize] };
                    self.in_buf.push(if v.width() == w { v } else { v.resize(w) });
                }
                self.out_buf.clear();
                self.out_buf.extend_from_slice(&self.out_vals[fc.outs.start as usize..fc.outs.end as usize]);
                scratch.clone_from(&self.states[c]);
                fc.prim.eval(&mut scratch, &self.in_buf, &mut self.out_buf);
                for (j, g) in (fc.outs.start..fc.outs.end).enumerate() {
                    if self.out_buf[j] != self.out_vals[g as usize] {
                        self.out_vals[g as usize] = self.out_buf[j];
                        let net = self.nl.out_nets[g as usize];
                        if net != NO_NET {
                            self.net_vals[net as usize] = self.resolve_net(net);
                        }
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    // ---- inspection ----------------------------------------------------------

    pub fn time(&self) -> u64 {
        self.now
    }

    pub fn net_value(&self, net: NetId) -> Signal {
        self.net_vals.get(net as usize).copied().unwrap_or_default()
    }

    pub fn net_values(&self) -> &[Signal] {
        &self.net_vals
    }

    pub fn state(&self, comp: u32) -> &State {
        &self.states[comp as usize]
    }

    /// Value on the `i`-th input of a component.
    pub fn comp_input(&self, comp: u32, i: usize) -> Signal {
        let c = &self.nl.comps[comp as usize];
        let k = c.ins.start as usize + i;
        match self.nl.in_nets.get(k) {
            Some(&n) if k < c.ins.end as usize && n != NO_NET => self.net_vals[n as usize],
            _ => Signal::z(1),
        }
    }

    pub fn oscillation(&self) -> Option<&Oscillation> {
        self.osc.as_ref()
    }

    /// No pending events (clocks aside).
    pub fn is_stable(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn pending_events(&self) -> usize {
        self.queue.len()
    }

    pub fn has_clocks(&self) -> bool {
        !self.clocks.is_empty()
    }

    /// Time of the next clock toggle.
    pub fn next_clock_time(&self) -> Option<u64> {
        self.clock_next.iter().copied().min()
    }

    // ---- user interaction --------------------------------------------------

    /// Changes the value of an `Input` (switch) or `Button`.
    pub fn set_input(&mut self, comp: u32, value: Signal) {
        let st = &mut self.states[comp as usize];
        st.q = value.resize(st.q.width());
        self.stimulus();
        self.eval_comp(comp);
    }

    /// An external change starts a new settling episode if the circuit was
    /// stable.
    fn stimulus(&mut self) {
        if self.queue.is_empty() {
            self.delta_base = self.next_delta_key();
        }
    }

    fn next_delta_key(&self) -> Key {
        match self.cur {
            Some(k) if k.time == self.now => Key { time: self.now, delta: k.delta + 1 },
            _ => Key { time: self.now, delta: 0 },
        }
    }

    pub fn input_value(&self, comp: u32) -> Signal {
        self.states[comp as usize].q
    }

    /// Types `keys` (ASCII) into a keyboard; returns how many fitted in its
    /// buffer.
    pub fn type_keys(&mut self, comp: u32, keys: &[u8]) -> usize {
        let st = &mut self.states[comp as usize];
        let n = keys.iter().take_while(|&&k| crate::component::eval::keyboard_push(st, k)).count();
        if n > 0 {
            self.stimulus();
            self.eval_comp(comp);
        }
        n
    }

    /// Empties the buffer of a keyboard.
    pub fn clear_keys(&mut self, comp: u32) {
        if let Some(n) = self.states[comp as usize].mem.first_mut() {
            *n = 0;
            self.stimulus();
            self.eval_comp(comp);
        }
    }

    /// Overwrites one word of a RAM/ROM instance.
    pub fn write_memory(&mut self, comp: u32, addr: usize, value: u64) {
        if let Some(w) = self.states[comp as usize].mem.get_mut(addr) {
            *w = value;
            self.stimulus();
            self.eval_comp(comp);
        }
    }

    // ---- running -------------------------------------------------------------

    /// Processes a single event.
    pub fn step_event(&mut self) -> Result<Step, Oscillation> {
        self.process(1)
    }

    /// Processes one delta cycle (all events with the same time and delta).
    pub fn step_delta(&mut self) -> Result<Step, Oscillation> {
        self.process(usize::MAX)
    }

    /// Advances by one tick.
    pub fn step_tick(&mut self) -> Result<(), Oscillation> {
        self.run_until(self.now + 1)
    }

    /// Advances to the next clock edge and lets the circuit propagate up to
    /// the following one. Without clocks, runs until stable.
    pub fn step_clock(&mut self) -> Result<(), Oscillation> {
        // Finish what is pending at the current instant first.
        let Some(t1) = self.next_clock_time() else {
            return self.settle(1_000_000).map(|_| ());
        };
        self.run_until(t1)?;
        let t2 = self.next_clock_time().unwrap_or(t1 + 1);
        self.run_until(t2.saturating_sub(1).max(t1))
    }

    /// Processes everything up to and including time `t`, then sets the
    /// current time to `t` (if later).
    pub fn run_until(&mut self, t: u64) -> Result<(), Oscillation> {
        self.run_until_limited(t, u64::MAX).map(|_| ())
    }

    /// Like [`Simulator::run_until`] but stops after about `max_evals`
    /// component evaluations. Returns whether `t` was reached.
    pub fn run_until_limited(&mut self, t: u64, max_evals: u64) -> Result<bool, Oscillation> {
        let budget_end = self.stats.evaluations.saturating_add(max_evals);
        loop {
            if let Some(o) = &self.osc {
                return Err(o.clone());
            }
            match self.next_time() {
                Some(next) if next <= t => {}
                _ => break,
            }
            if self.stats.evaluations >= budget_end {
                return Ok(false);
            }
            self.process(usize::MAX)?;
        }
        if self.now < t {
            self.now = t;
        }
        Ok(true)
    }

    /// Runs until no events are pending or `max_batches` batches have been
    /// processed; returns whether the circuit settled. Clocks are frozen:
    /// the time spent settling (with non-zero delays) shifts their next
    /// toggles, so that their phase is preserved.
    pub fn settle(&mut self, max_batches: usize) -> Result<bool, Oscillation> {
        let start = self.now;
        let mut result = Ok(());
        for _ in 0..max_batches {
            if self.queue.is_empty() {
                break;
            }
            if let Err(e) = self.process_inner(usize::MAX, false) {
                result = Err(e);
                break;
            }
        }
        let elapsed = self.now - start;
        for n in &mut self.clock_next {
            *n += elapsed;
        }
        result.map(|_| self.queue.is_empty())
    }

    /// Time of the next thing to do: an event or a clock toggle.
    fn next_time(&self) -> Option<u64> {
        let ev = self.queue.peek().map(|e| e.key.time);
        match (ev, self.next_clock_time()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn process(&mut self, max_events: usize) -> Result<Step, Oscillation> {
        self.process_inner(max_events, true)
    }

    fn process_inner(&mut self, max_events: usize, clocks: bool) -> Result<Step, Oscillation> {
        if let Some(o) = &self.osc {
            return Err(o.clone());
        }
        if clocks
            && let Some(tc) = self.next_clock_time()
            && self.queue.peek().is_none_or(|e| tc <= e.key.time)
        {
            self.fire_clocks(tc);
        }
        let Some(top) = self.queue.peek() else {
            return Ok(Step::Idle);
        };
        let key = top.key;
        if key.time > self.now {
            self.now = key.time;
        }
        let deltas =
            if key.time == self.delta_base.time { key.delta.saturating_sub(self.delta_base.delta) } else { key.delta };
        if deltas >= self.cfg.delta_limit {
            let mut nets = std::mem::take(&mut self.osc_nets);
            nets.sort_unstable();
            nets.dedup();
            let o = Oscillation { time: key.time, nets };
            self.osc = Some(o.clone());
            return Err(o);
        }
        self.cur = Some(key);

        let mut n = 0;
        while n < max_events {
            match self.queue.peek() {
                Some(e) if e.key == key => {}
                _ => break,
            }
            let e = self.queue.pop().unwrap();
            self.out_vals[e.out as usize] = e.value;
            let net = self.nl.out_nets[e.out as usize];
            if net != NO_NET && !self.net_dirty[net as usize] {
                self.net_dirty[net as usize] = true;
                self.dirty_nets.push(net);
            }
            n += 1;
        }
        self.stats.events += n as u64;

        let track = deltas + OSC_WINDOW >= self.cfg.delta_limit;
        let mut dirty_nets = std::mem::take(&mut self.dirty_nets);
        for &net in &dirty_nets {
            self.net_dirty[net as usize] = false;
            let v = self.resolve_net(net);
            if v != self.net_vals[net as usize] {
                self.net_vals[net as usize] = v;
                self.trace.record(net, key.time, v);
                if track {
                    self.osc_nets.push(net);
                }
                for &r in &self.nl.nets[net as usize].readers {
                    if !self.comp_dirty[r as usize] {
                        self.comp_dirty[r as usize] = true;
                        self.dirty_comps.push(r);
                    }
                }
            }
        }
        dirty_nets.clear();
        self.dirty_nets = dirty_nets;

        let mut comps = std::mem::take(&mut self.dirty_comps);
        comps.sort_unstable();
        for &c in &comps {
            self.comp_dirty[c as usize] = false;
            self.eval_comp(c);
        }
        comps.clear();
        self.dirty_comps = comps;
        Ok(Step::Progress)
    }

    fn fire_clocks(&mut self, t: u64) {
        if t > self.now {
            self.now = t;
        }
        for k in 0..self.clocks.len() {
            if self.clock_next[k] != t {
                continue;
            }
            let c = self.clocks[k];
            let Prim::Clock { high, low } = self.nl.comps[c as usize].prim else { unreachable!() };
            let st = &mut self.states[c as usize];
            let rising = !st.q.is_one();
            st.q = Signal::bool(rising);
            self.clock_next[k] = t + if rising { high } else { low } as u64;
            self.eval_comp(c);
        }
    }

    fn resolve_net(&self, n: NetId) -> Signal {
        let net = &self.nl.nets[n as usize];
        if net.width_error {
            return Signal::x(net.width);
        }
        let mut it = net.drivers.iter();
        match it.next() {
            None => Signal::z(net.width),
            Some(&g) => it.fold(self.out_vals[g as usize], |acc, &g| acc.resolve(self.out_vals[g as usize])),
        }
    }

    fn eval_comp(&mut self, c: u32) {
        let fc = &self.nl.comps[c as usize];
        self.in_buf.clear();
        for k in fc.ins.start..fc.ins.end {
            let net = self.nl.in_nets[k as usize];
            let w = self.nl.in_widths[k as usize];
            let v = if net == NO_NET { Signal::z(w) } else { self.net_vals[net as usize] };
            self.in_buf.push(if v.width() == w { v } else { v.resize(w) });
        }
        self.out_buf.clear();
        self.out_buf.extend_from_slice(&self.out_proj[fc.outs.start as usize..fc.outs.end as usize]);
        fc.prim.eval(&mut self.states[c as usize], &self.in_buf, &mut self.out_buf);
        self.stats.evaluations += 1;

        let delay = if fc.prim.is_source() { 0 } else { fc.delay.unwrap_or(self.cfg.gate_delay) };
        let key = if delay == 0 { self.next_delta_key() } else { Key { time: self.now + delay as u64, delta: 0 } };
        for (j, g) in (fc.outs.start..fc.outs.end).enumerate() {
            let v = self.out_buf[j];
            if v != self.out_proj[g as usize] {
                self.out_proj[g as usize] = v;
                self.seq += 1;
                self.queue.push(Event { key, seq: self.seq, out: g, value: v });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::{Circuit, Project};
    use crate::component::Kind;
    use crate::netlist::compile;
    use crate::test_util::{add, wire};

    fn zero_delay() -> SimConfig {
        SimConfig { gate_delay: 0, ..SimConfig::default() }
    }

    #[test]
    fn not_ring_oscillates() {
        let mut c = Circuit::new("osc");
        let n = add(&mut c, Kind::Not, "");
        wire(&mut c, n, "out", n, "in");
        let nl = compile(&Project::new(c), "osc").unwrap();
        let mut sim = Simulator::new(nl, zero_delay());
        let err = sim.settle(10_000).unwrap_err();
        assert_eq!(err.time, 0);
        assert_eq!(err.nets.len(), 1);
        assert!(sim.oscillation().is_some());
        sim.reset();
        assert!(sim.oscillation().is_none());
    }

    #[test]
    fn ring_oscillator_with_delay_is_a_clock() {
        let mut c = Circuit::new("osc");
        let n = add(&mut c, Kind::Not, "");
        wire(&mut c, n, "out", n, "in");
        let nl = compile(&Project::new(c), "osc").unwrap();
        let mut sim = Simulator::new(nl, SimConfig { gate_delay: 3, ..SimConfig::default() });
        sim.trace.add("n", 0, sim.net_value(0), 0, None);
        sim.run_until(20).unwrap();
        let times: Vec<u64> = sim.trace.signals()[0].samples.iter().map(|s| s.0).collect();
        assert_eq!(times, [0, 3, 6, 9, 12, 15, 18]);
    }

    #[test]
    fn driver_conflict_gives_x() {
        let mut c = Circuit::new("conflict");
        let a = add(&mut c, Kind::Input, "a");
        let b = add(&mut c, Kind::Input, "b");
        let o = add(&mut c, Kind::Output, "o");
        wire(&mut c, a, "out", o, "in");
        wire(&mut c, b, "out", o, "in");
        let nl = compile(&Project::new(c), "conflict").unwrap();
        let net = nl.pin_net(&[], o, 0).unwrap();
        let (ca, cb) = (nl.comp_by_path("a").unwrap(), nl.comp_by_path("b").unwrap());
        let mut sim = Simulator::new(nl, zero_delay());
        sim.settle(100).unwrap();
        assert_eq!(sim.net_value(net), Signal::bool(false));
        sim.set_input(ca, Signal::bool(true));
        sim.settle(100).unwrap();
        assert_eq!(sim.net_value(net), Signal::x(1));
        sim.set_input(cb, Signal::bool(true));
        sim.settle(100).unwrap();
        assert_eq!(sim.net_value(net), Signal::bool(true));
    }

    #[test]
    fn tristate_bus_sharing() {
        let mut c = Circuit::new("bus");
        let d0 = add(&mut c, Kind::Constant, "");
        let d1 = add(&mut c, Kind::Constant, "");
        c.component_mut(d1).unwrap().props.value = 1;
        let sel = add(&mut c, Kind::Input, "sel");
        let inv = add(&mut c, Kind::Not, "");
        let t0 = add(&mut c, Kind::TriBuffer, "");
        let t1 = add(&mut c, Kind::TriBuffer, "");
        let o = add(&mut c, Kind::Output, "bus");
        wire(&mut c, d0, "out", t0, "in");
        wire(&mut c, d1, "out", t1, "in");
        wire(&mut c, sel, "out", inv, "in");
        wire(&mut c, inv, "out", t0, "en");
        wire(&mut c, sel, "out", t1, "en");
        wire(&mut c, t0, "out", o, "in");
        wire(&mut c, t1, "out", o, "in");
        let nl = compile(&Project::new(c), "bus").unwrap();
        assert!(nl.diagnostics.is_empty(), "{:?}", nl.diagnostics);
        let bus = nl.net_by_name("bus").unwrap();
        let sel = nl.comp_by_path("sel").unwrap();
        let mut sim = Simulator::new(nl, SimConfig::default());
        sim.settle(100).unwrap();
        assert_eq!(sim.net_value(bus), Signal::bool(false));
        sim.set_input(sel, Signal::bool(true));
        sim.settle(100).unwrap();
        assert_eq!(sim.net_value(bus), Signal::bool(true));
    }

    #[test]
    fn deterministic_event_sequence() {
        let build = || {
            let mut c = Circuit::new("d");
            let clk = add(&mut c, Kind::Clock, "clk");
            c.component_mut(clk).unwrap().props.period = 4;
            let t = add(&mut c, Kind::TFlipFlop, "t");
            let one = add(&mut c, Kind::Constant, "");
            c.component_mut(one).unwrap().props.value = 1;
            let x = add(&mut c, Kind::Xor, "x");
            wire(&mut c, one, "out", t, "t");
            wire(&mut c, clk, "out", t, "clk");
            wire(&mut c, t, "q", x, "in0");
            wire(&mut c, clk, "out", x, "in1");
            let nl = compile(&Project::new(c), "d").unwrap();
            let mut sim = Simulator::new(nl, SimConfig::default());
            for n in 0..sim.netlist().nets.len() as u32 {
                let name = sim.netlist().nets[n as usize].name.clone();
                sim.trace.add(name, n, sim.net_value(n), 0, None);
            }
            sim.run_until(200).unwrap();
            (sim.trace.export_vcd("1ns", 200), sim.stats.events)
        };
        assert_eq!(build(), build());
    }

    #[test]
    fn step_granularity() {
        let mut c = Circuit::new("s");
        let a = add(&mut c, Kind::Input, "a");
        let n1 = add(&mut c, Kind::Not, "");
        let n2 = add(&mut c, Kind::Not, "");
        let o = add(&mut c, Kind::Output, "o");
        wire(&mut c, a, "out", n1, "in");
        wire(&mut c, n1, "out", n2, "in");
        wire(&mut c, n2, "out", o, "in");
        let nl = compile(&Project::new(c), "s").unwrap();
        let out = nl.net_by_name("o").unwrap();
        let ca = nl.comp_by_path("a").unwrap();
        let mut sim = Simulator::new(nl, SimConfig::default());
        // The initial relaxation already settled the chain.
        assert!(sim.is_stable());
        assert_eq!(sim.time(), 0);
        sim.set_input(ca, Signal::bool(true));
        assert_eq!(sim.step_delta(), Ok(Step::Progress)); // input applied at t=0
        assert_eq!(sim.net_value(out), Signal::bool(false));
        sim.step_tick().unwrap(); // t=1: first NOT
        assert_eq!(sim.net_value(out), Signal::bool(false));
        sim.step_tick().unwrap(); // t=2: second NOT
        assert_eq!(sim.net_value(out), Signal::bool(true));
        assert!(sim.is_stable());
        assert_eq!(sim.step_event(), Ok(Step::Idle));
    }

    #[test]
    fn clock_waveform_and_step_clock() {
        let mut c = Circuit::new("c");
        let clk = add(&mut c, Kind::Clock, "clk");
        c.component_mut(clk).unwrap().props.period = 10;
        c.component_mut(clk).unwrap().props.duty = 30;
        let o = add(&mut c, Kind::Output, "o");
        wire(&mut c, clk, "out", o, "in");
        let nl = compile(&Project::new(c), "c").unwrap();
        let net = nl.net_by_name("clk").unwrap();
        let mut sim = Simulator::new(nl, SimConfig::default());
        sim.trace.add("clk", net, sim.net_value(net), 0, None);
        sim.run_until(30).unwrap();
        let samples: Vec<(u64, bool)> = sim.trace.signals()[0].samples.iter().map(|(t, v)| (*t, v.is_one())).collect();
        assert_eq!(samples, [(0, false), (7, true), (10, false), (17, true), (20, false), (27, true), (30, false)]);
        sim.step_clock().unwrap();
        assert_eq!(sim.time(), 39);
        assert!(sim.net_value(net).is_one());
        sim.step_clock().unwrap();
        assert_eq!(sim.time(), 46);
        assert!(!sim.net_value(net).is_one());
    }

    #[test]
    fn loop_closed_by_an_edit_oscillates() {
        // A NOT with a floating input outputs X; closing the loop afterwards
        // must still be detected as an oscillation.
        let mut c = Circuit::new("o");
        let n = add(&mut c, Kind::Not, "");
        let p = Project::new(c);
        let mut sim = Simulator::new(compile(&p, "o").unwrap(), zero_delay());
        sim.settle(100).unwrap();
        let mut p2 = p.clone();
        wire(&mut p2.circuits[0], n, "out", n, "in");
        let mut sim2 = Simulator::new_from(compile(&p2, "o").unwrap(), zero_delay(), &sim);
        assert!(sim2.settle(10_000).is_err());
    }

    #[test]
    fn state_survives_recompile() {
        let mut c = Circuit::new("r");
        let a = add(&mut c, Kind::Input, "a");
        let o = add(&mut c, Kind::Output, "o");
        wire(&mut c, a, "out", o, "in");
        let p = Project::new(c);
        let nl = compile(&p, "r").unwrap();
        let ca = nl.comp_by_path("a").unwrap();
        let mut sim = Simulator::new(nl, SimConfig::default());
        sim.set_input(ca, Signal::bool(true));
        sim.run_until(10).unwrap();
        let mut p2 = p.clone();
        add(&mut p2.circuits[0], Kind::Led, "");
        let sim2 = Simulator::new_from(compile(&p2, "r").unwrap(), SimConfig::default(), &sim);
        assert_eq!(sim2.time(), 10);
        assert_eq!(sim2.input_value(ca), Signal::bool(true));
    }
}
