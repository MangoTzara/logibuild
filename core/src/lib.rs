//! LogiBuild core: signal model, components, schematic, netlist compiler,
//! event-driven simulator, signal tracing, file I/O and undo/redo.
//!
//! This crate has no GUI dependency and can be used headless:
//!
//! ```
//! use lb_core::prelude::*;
//!
//! let mut c = Circuit::new("inv");
//! let a = c.add(Kind::Input, (0, 0));
//! let n = c.add(Kind::Not, (4, 0));
//! c.connect((a, "out"), (n, "in"));
//! let project = Project::new(c);
//! let netlist = compile(&project, "inv").unwrap();
//! let out = netlist.pin_net(&[], n, 1).unwrap();
//! let mut sim = Simulator::new(netlist, SimConfig::default());
//! sim.settle(100).unwrap();
//! assert_eq!(sim.net_value(out), Signal::bool(true));
//! ```

pub mod circuit;
pub mod component;
pub mod history;
pub mod io;
pub mod netlist;
pub mod sim;
pub mod trace;
pub mod value;

pub mod prelude {
    pub use crate::circuit::{Circuit, CompId, Component, PinRef, Project, Wire};
    pub use crate::component::{Kind, Props};
    pub use crate::netlist::{Netlist, compile};
    pub use crate::sim::{SimConfig, Simulator};
    pub use crate::value::{Radix, Signal};
}

#[cfg(test)]
pub(crate) mod test_util {
    use crate::circuit::{Circuit, CompId, PinRef, Wire};
    use crate::component::Kind;

    pub fn add(c: &mut Circuit, kind: Kind, label: &str) -> CompId {
        let id = c.add(kind, (0, 0));
        c.component_mut(id).unwrap().props.label = label.into();
        id
    }

    pub fn wire(c: &mut Circuit, a: CompId, pa: &str, b: CompId, pb: &str) {
        c.insert_wire(Wire::new(PinRef::new(a, pa), PinRef::new(b, pb)));
    }
}
