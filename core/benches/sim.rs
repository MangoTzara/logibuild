//! Benchmarks of the simulation engine (SPEC §5.4: at least one million gate
//! evaluations per second on one core; §5.1: compiling 10 000 components in
//! under 50 ms).

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use lb_core::prelude::*;

/// An input driving a chain of `n` alternating NOT/XOR gates.
fn chain(n: usize) -> Project {
    let mut c = Circuit::new("chain");
    let input = c.add(Kind::Input, (0, 0));
    let one = c.add(Kind::Constant, (0, 4));
    c.component_mut(one).unwrap().props.value = 1;
    let mut prev = input;
    for i in 0..n {
        let g = if i % 2 == 0 {
            let g = c.add(Kind::Not, (0, 0));
            c.connect((prev, "out"), (g, "in"));
            g
        } else {
            let g = c.add(Kind::Xor, (0, 0));
            c.connect((prev, "out"), (g, "in0"));
            c.connect((one, "out"), (g, "in1"));
            g
        };
        prev = g;
    }
    Project::new(c)
}

/// A 16-bit ripple counter of T flip-flops plus an 8-bit adder fed by it:
/// mixed sequential/combinational activity driven by a clock.
fn counter_and_adder() -> Project {
    let mut c = Circuit::new("busy");
    let clk = c.add(Kind::Clock, (0, 0));
    c.component_mut(clk).unwrap().props.period = 4;
    let one = c.add(Kind::Constant, (0, 0));
    c.component_mut(one).unwrap().props.value = 1;
    let mut q = (clk, "out");
    let mut bits = Vec::new();
    for _ in 0..16 {
        let t = c.add(Kind::TFlipFlop, (0, 0));
        c.component_mut(t).unwrap().props.edge = lb_core::component::Edge::Falling;
        c.connect((one, "out"), (t, "t"));
        c.connect(q, (t, "clk"));
        q = (t, "q");
        bits.push(t);
    }
    let m = c.add(Kind::Merger, (0, 0));
    c.component_mut(m).unwrap().props.width = 16;
    for (i, b) in bits.iter().enumerate() {
        c.connect((*b, "q"), (m, &format!("in{i}")));
    }
    let add = c.add(Kind::Adder, (0, 0));
    c.component_mut(add).unwrap().props.width = 16;
    c.connect((m, "out"), (add, "a"));
    c.connect((m, "out"), (add, "b"));
    Project::new(c)
}

fn bench_eval(cr: &mut Criterion) {
    const N: usize = 10_000;
    let p = chain(N);
    let nl = compile(&p, "chain").unwrap();
    let input = nl.comp_by_path("input1").unwrap();
    for (name, delay) in [("chain_toggle_delay", 1), ("chain_toggle_zero_delay", 0)] {
        let mut sim = Simulator::new(nl.clone(), SimConfig { gate_delay: delay, delta_limit: 1_000_000 });
        let mut v = false;
        let mut g = cr.benchmark_group("simulation");
        g.throughput(Throughput::Elements(N as u64));
        g.bench_function(name, |b| {
            b.iter(|| {
                v = !v;
                sim.set_input(input, Signal::bool(v));
                black_box(sim.settle(usize::MAX).unwrap());
            })
        });
        g.finish();
    }

    let p = counter_and_adder();
    let nl = compile(&p, "busy").unwrap();
    let mut sim = Simulator::new(nl, SimConfig::default());
    cr.bench_function("simulation/clocked_counter_1000_ticks", |b| {
        b.iter(|| {
            let t = sim.time();
            sim.run_until(t + 1000).unwrap();
        })
    });
}

fn bench_compile(cr: &mut Criterion) {
    let p = chain(10_000);
    cr.bench_function("compile/10k_components", |b| b.iter(|| compile(black_box(&p), "chain").unwrap()));
}

criterion_group!(benches, bench_eval, bench_compile);
criterion_main!(benches);
