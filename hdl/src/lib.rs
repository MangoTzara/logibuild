//! LogiBuild HDL support: VHDL and Verilog sources turned into circuits.
//!
//! The sources are compiled by external tools, GHDL for VHDL and Yosys for
//! synthesis ([`toolchain`]); the netlist Yosys writes in JSON ([`json`]) is
//! converted into ordinary LogiBuild circuits, one per entity or module
//! ([`import`]), laid out automatically ([`place`]). [`lang`] knows the
//! file types, the units a source declares and the words to highlight, and
//! [`log`] reads the messages of the tools.
//!
//! This crate has no GUI dependency.

pub mod import;
pub mod json;
pub mod lang;
pub mod log;
pub mod place;
pub mod toolchain;

use lb_core::circuit::VhdlStd;

/// Fingerprint of the sources a synthesis reads: the language revision,
/// then each file's name and contents (`None` for a missing file). Any
/// change gives another digest.
pub fn digest(std: VhdlStd, sources: &[(String, Option<Vec<u8>>)]) -> String {
    // FNV-1a, 64 bits.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        // Separator, so that ("ab", "c") and ("a", "bc") differ.
        h ^= 0xff;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    feed(std.name().as_bytes());
    for (name, data) in sources {
        feed(name.as_bytes());
        match data {
            Some(d) => feed(d),
            None => feed(b"\0missing"),
        }
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_changes_with_any_input() {
        let a = digest(VhdlStd::V08, &[("a.vhd".into(), Some(b"entity a".to_vec()))]);
        assert_eq!(a.len(), 16);
        assert_eq!(a, digest(VhdlStd::V08, &[("a.vhd".into(), Some(b"entity a".to_vec()))]));
        assert_ne!(a, digest(VhdlStd::V93, &[("a.vhd".into(), Some(b"entity a".to_vec()))]));
        assert_ne!(a, digest(VhdlStd::V08, &[("a.vhd".into(), Some(b"entity b".to_vec()))]));
        assert_ne!(a, digest(VhdlStd::V08, &[("a.vhd".into(), None)]));
        assert_ne!(
            digest(VhdlStd::V08, &[("ab".into(), Some(b"c".to_vec()))]),
            digest(VhdlStd::V08, &[("a".into(), Some(b"bc".to_vec()))])
        );
    }
}
