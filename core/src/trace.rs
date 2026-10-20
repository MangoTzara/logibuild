//! Signal recording for the waveform view, with CSV and VCD export.
//!
//! Each traced signal keeps its value changes `(time, value)` in a ring
//! buffer of bounded length: when it is full the oldest change is dropped.

use std::collections::VecDeque;
use std::fmt::Write;

use crate::circuit::CompId;
use crate::netlist::NetId;
use crate::value::{Bit, Radix, Signal};

pub const DEFAULT_CAPACITY: usize = 100_000;

/// Identifies the pin a signal was taken from, so that it can be found again
/// after the circuit is recompiled: `(instance path, component id, layout pin
/// index)`.
pub type SignalKey = (Box<[CompId]>, CompId, usize);

#[derive(Clone, Debug)]
pub struct TraceSignal {
    pub name: String,
    pub net: NetId,
    pub width: u8,
    pub radix: Radix,
    pub key: Option<SignalKey>,
    pub samples: VecDeque<(u64, Signal)>,
}

impl TraceSignal {
    /// Value at `time`, if recorded.
    pub fn value_at(&self, time: u64) -> Option<Signal> {
        let i = self.samples.partition_point(|(t, _)| *t <= time);
        (i > 0).then(|| self.samples[i - 1].1)
    }

    /// First change strictly after `time`.
    pub fn next_change(&self, time: u64) -> Option<u64> {
        let i = self.samples.partition_point(|(t, _)| *t <= time);
        self.samples.get(i).map(|s| s.0)
    }

    /// Last change strictly before `time`.
    pub fn prev_change(&self, time: u64) -> Option<u64> {
        let i = self.samples.partition_point(|(t, _)| *t < time);
        (i > 0).then(|| self.samples[i - 1].0)
    }

    pub fn first_time(&self) -> Option<u64> {
        self.samples.front().map(|s| s.0)
    }
}

#[derive(Clone, Debug)]
pub struct Trace {
    capacity: usize,
    signals: Vec<TraceSignal>,
    /// Signal index of each net, `u32::MAX` if not traced.
    watch: Vec<u32>,
}

impl Default for Trace {
    fn default() -> Self {
        Trace::new(DEFAULT_CAPACITY)
    }
}

impl Trace {
    pub fn new(capacity: usize) -> Self {
        Trace { capacity: capacity.max(2), signals: Vec::new(), watch: Vec::new() }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(2);
        for s in &mut self.signals {
            while s.samples.len() > self.capacity {
                s.samples.pop_front();
            }
        }
    }

    pub fn signals(&self) -> &[TraceSignal] {
        &self.signals
    }

    pub fn signals_mut(&mut self) -> &mut [TraceSignal] {
        &mut self.signals
    }

    pub fn is_empty(&self) -> bool {
        self.signals.is_empty()
    }

    pub fn index_of_net(&self, net: NetId) -> Option<usize> {
        match self.watch.get(net as usize) {
            Some(&i) if i != u32::MAX => Some(i as usize),
            _ => None,
        }
    }

    /// Starts tracing `net`, whose current value is `value` at `time`.
    /// Returns the signal index (the existing one if already traced).
    pub fn add(
        &mut self,
        name: impl Into<String>,
        net: NetId,
        value: Signal,
        time: u64,
        key: Option<SignalKey>,
    ) -> usize {
        if let Some(i) = self.index_of_net(net) {
            return i;
        }
        let mut samples = VecDeque::new();
        samples.push_back((time, value));
        let width = value.width();
        self.signals.push(TraceSignal {
            name: name.into(),
            net,
            width,
            radix: if width == 1 { Radix::Bin } else { Radix::Hex },
            key,
            samples,
        });
        self.rebuild_watch();
        self.signals.len() - 1
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.signals.len() {
            self.signals.remove(i);
            self.rebuild_watch();
        }
    }

    pub fn move_signal(&mut self, from: usize, to: usize) {
        if from < self.signals.len() && to < self.signals.len() {
            let s = self.signals.remove(from);
            self.signals.insert(to, s);
            self.rebuild_watch();
        }
    }

    fn rebuild_watch(&mut self) {
        self.watch.clear();
        for (i, s) in self.signals.iter().enumerate() {
            if s.net as usize >= self.watch.len() {
                self.watch.resize(s.net as usize + 1, u32::MAX);
            }
            self.watch[s.net as usize] = i as u32;
        }
    }

    /// Records a value change of `net`. Several changes at the same time (delta
    /// cycles) collapse into the last one.
    #[inline]
    pub fn record(&mut self, net: NetId, time: u64, value: Signal) {
        let Some(i) = self.index_of_net(net) else { return };
        let s = &mut self.signals[i];
        if let Some(last) = s.samples.back_mut()
            && last.0 == time
        {
            last.1 = value;
            // Collapse a glitch that returned to the previous value.
            let n = s.samples.len();
            if n >= 2 && s.samples[n - 2].1 == value {
                s.samples.pop_back();
            }
            return;
        }
        if s.samples.back().is_some_and(|l| l.1 == value) {
            return;
        }
        if s.samples.len() >= self.capacity {
            s.samples.pop_front();
        }
        s.samples.push_back((time, value));
    }

    /// Drops all samples, restarting every signal from `value_of(net)` at
    /// `time`.
    pub fn restart(&mut self, time: u64, value_of: impl Fn(NetId) -> Signal) {
        for s in &mut self.signals {
            s.samples.clear();
            s.samples.push_back((time, value_of(s.net)));
        }
    }

    /// Re-attaches signals to the nets of a new netlist, dropping those that
    /// no longer exist. `resolve` gets each signal and returns its new net.
    pub fn rebind(&mut self, mut resolve: impl FnMut(&TraceSignal) -> Option<(NetId, u8)>) {
        self.signals.retain_mut(|s| match resolve(s) {
            Some((net, width)) => {
                if width != s.width {
                    s.samples.clear();
                    s.width = width;
                }
                s.net = net;
                true
            }
            None => false,
        });
        self.rebuild_watch();
    }

    /// Earliest recorded time.
    pub fn start_time(&self) -> u64 {
        self.signals.iter().filter_map(|s| s.first_time()).min().unwrap_or(0)
    }

    /// All change times of all signals, sorted and deduplicated.
    fn change_times(&self) -> Vec<u64> {
        let mut times: Vec<u64> = self.signals.iter().flat_map(|s| s.samples.iter().map(|x| x.0)).collect();
        times.sort_unstable();
        times.dedup();
        times
    }

    /// CSV with one row per change time and one column per signal.
    pub fn export_csv(&self) -> String {
        let mut out = String::from("time");
        for s in &self.signals {
            out.push(',');
            out.push_str(&csv_field(&s.name));
        }
        out.push('\n');
        for t in self.change_times() {
            let _ = write!(out, "{t}");
            for s in &self.signals {
                out.push(',');
                if let Some(v) = s.value_at(t) {
                    out.push_str(&v.format(if s.width == 1 { Radix::Bin } else { s.radix }));
                }
            }
            out.push('\n');
        }
        out
    }

    /// Value Change Dump (IEEE 1364), readable by GTKWave. One tick is
    /// written as one `timescale` unit. Hierarchical names (`a/b/c`) become
    /// nested scopes.
    pub fn export_vcd(&self, timescale: &str, end_time: u64) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "$version LogiBuild {} $end", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(out, "$timescale {timescale} $end");

        let ids: Vec<String> = (0..self.signals.len()).map(vcd_id).collect();
        // Group by scope: sort indices by path segments.
        let mut order: Vec<usize> = (0..self.signals.len()).collect();
        let split = |i: usize| -> Vec<String> { self.signals[i].name.split('/').map(vcd_name).collect() };
        order.sort_by_key(|&i| {
            let mut p = split(i);
            p.pop();
            p
        });
        let _ = writeln!(out, "$scope module logibuild $end");
        let mut scope: Vec<String> = Vec::new();
        for &i in &order {
            let mut path = split(i);
            let leaf = path.pop().unwrap_or_default();
            let common = scope.iter().zip(&path).take_while(|(a, b)| a == b).count();
            while scope.len() > common {
                scope.pop();
                let _ = writeln!(out, "$upscope $end");
            }
            for seg in &path[common..] {
                let _ = writeln!(out, "$scope module {seg} $end");
                scope.push(seg.clone());
            }
            let s = &self.signals[i];
            let _ = writeln!(out, "$var wire {} {} {} $end", s.width, ids[i], leaf);
        }
        for _ in scope {
            let _ = writeln!(out, "$upscope $end");
        }
        let _ = writeln!(out, "$upscope $end");
        let _ = writeln!(out, "$enddefinitions $end");

        let fmt = |out: &mut String, v: Signal, id: &str| {
            if v.width() == 1 {
                let _ = writeln!(out, "{}{id}", v.bit(0).to_char());
            } else {
                let _ = writeln!(out, "b{} {id}", v.to_bin_string());
            }
        };
        let times = self.change_times();
        let mut cursors = vec![0usize; self.signals.len()];
        let mut first = true;
        for t in times {
            let _ = writeln!(out, "#{t}");
            if first {
                out.push_str("$dumpvars\n");
            }
            for (i, s) in self.signals.iter().enumerate() {
                let c = &mut cursors[i];
                while *c < s.samples.len() && s.samples[*c].0 < t {
                    *c += 1;
                }
                if first {
                    let v = s.value_at(t).unwrap_or(Signal::x(s.width));
                    fmt(&mut out, v, &ids[i]);
                } else if *c < s.samples.len() && s.samples[*c].0 == t {
                    fmt(&mut out, s.samples[*c].1, &ids[i]);
                }
            }
            if first {
                out.push_str("$end\n");
                first = false;
            }
        }
        let _ = writeln!(out, "#{end_time}");
        out
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.into() }
}

fn vcd_name(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_ascii_graphic() && c != '$' { c } else { '_' }).collect();
    if s.is_empty() { "_".into() } else { s }
}

/// Short VCD identifier made of printable characters `!`..`~`.
fn vcd_id(mut i: usize) -> String {
    let mut s = String::new();
    loop {
        s.push((b'!' + (i % 94) as u8) as char);
        i /= 94;
        if i == 0 {
            break;
        }
        i -= 1;
    }
    s
}

/// Whether a value counts as an edge for "find next edge" on 1-bit signals.
pub fn is_edge(prev: Signal, next: Signal) -> bool {
    prev.bit(0) != next.bit(0) && matches!(next.bit(0), Bit::Zero | Bit::One)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_and_lookup() {
        let mut t = Trace::new(3);
        t.add("a", 0, Signal::bool(false), 0, None);
        for (time, v) in [(5, true), (10, false), (15, true)] {
            t.record(0, time, Signal::bool(v));
        }
        let s = &t.signals()[0];
        assert_eq!(s.samples.len(), 3);
        assert_eq!(s.first_time(), Some(5));
        assert_eq!(s.value_at(12), Some(Signal::bool(false)));
        assert_eq!(s.value_at(4), None);
        assert_eq!(s.next_change(5), Some(10));
        assert_eq!(s.prev_change(10), Some(5));
    }

    #[test]
    fn delta_glitches_collapse() {
        let mut t = Trace::new(10);
        t.add("a", 3, Signal::bool(false), 0, None);
        t.record(3, 4, Signal::bool(true));
        t.record(3, 4, Signal::bool(false));
        assert_eq!(t.signals()[0].samples.len(), 1);
        t.record(7, 4, Signal::bool(true)); // untraced net
        assert_eq!(t.signals().len(), 1);
    }

    #[test]
    fn csv_and_vcd() {
        let mut t = Trace::new(100);
        t.add("clk", 0, Signal::bool(false), 0, None);
        t.add("cpu/alu/out", 1, Signal::new(4, 3), 0, None);
        t.record(0, 2, Signal::bool(true));
        t.record(1, 3, Signal::x(4));
        let csv = t.export_csv();
        assert_eq!(csv, "time,clk,cpu/alu/out\n0,0,3\n2,1,3\n3,1,x\n");
        let vcd = t.export_vcd("1ns", 5);
        assert!(vcd.contains("$var wire 1 ! clk $end"));
        assert!(vcd.contains("$scope module cpu $end\n$scope module alu $end\n$var wire 4 \" out $end"));
        assert!(vcd.contains("#0\n$dumpvars\n0!\nb0011 \"\n$end\n#2\n1!\n#3\nbxxxx \"\n#5\n"));
    }

    #[test]
    fn vcd_ids_unique() {
        let ids: std::collections::HashSet<String> = (0..20_000).map(vcd_id).collect();
        assert_eq!(ids.len(), 20_000);
    }
}
