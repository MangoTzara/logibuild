//! The documentation of each component, shown from the properties panel: its
//! section of `docs/COMPONENTS.md`, built into the program so that the file
//! is the only place to edit it.

use lb_core::component::Kind;

const COMPONENTS: &str = include_str!("../../docs/COMPONENTS.md");

/// The section of a component, after its `### Name` heading and up to the
/// next heading.
pub fn section(kind: Kind) -> Option<&'static str> {
    let heading = format!("### {}\n", kind.name());
    let start = COMPONENTS.find(&heading)? + heading.len();
    let rest = &COMPONENTS[start..];
    let end = ["\n## ", "\n### "].iter().filter_map(|h| rest.find(h)).min().unwrap_or(rest.len());
    Some(rest[..end].trim())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use lb_core::component::layout::layout;

    use super::*;

    /// The pins a cell of the Pin column stands for, given the pins of the
    /// component: `` `a` ``, `` `a`, `b` ``, a range `` `x0` … `x7` `` or
    /// `` `a` … `g` ``, or an open list `` `in0`, `in1`, … `` (every pin
    /// named like its first one).
    fn covered(cell: &str, pins: &[String]) -> BTreeSet<String> {
        let names: Vec<&str> = cell.split('`').skip(1).step_by(2).collect();
        let base = |n: &str| n.trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
        if cell.trim_end().ends_with(", …") {
            return pins.iter().filter(|p| base(p) == base(names[0])).cloned().collect();
        }
        if let [from, to] = names[..]
            && cell.contains("` … `")
        {
            let (fb, tb) = (base(from), base(to));
            if !fb.is_empty() && fb == tb {
                let n = |s: &str| s[fb.len()..].parse::<u32>().unwrap();
                return (n(from)..=n(to)).map(|i| format!("{fb}{i}")).collect();
            }
            let (f, t) = (from.chars().next().unwrap(), to.chars().next().unwrap());
            return (f..=t).map(String::from).collect();
        }
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn every_component_is_documented_with_its_pins() {
        let file_format = include_str!("../../docs/FILE_FORMAT.md");
        for kind in Kind::ALL {
            let text = section(kind).unwrap_or_else(|| panic!("no section “### {}”", kind.name()));
            let name = kind.name();
            assert!(file_format.contains(&format!("| `{kind:?}`")), "{name} in the types of FILE_FORMAT.md");
            if kind == Kind::Subcircuit {
                continue;
            }
            let pins: Vec<String> =
                layout(kind, &kind.default_props(), None).pins.iter().map(|p| p.name.to_string()).collect();
            let mut documented = BTreeSet::new();
            for row in text.lines().filter(|l| l.starts_with("| `")) {
                let cell = row.split('|').nth(1).unwrap();
                documented.extend(covered(cell, &pins));
            }
            let actual: BTreeSet<String> = pins.into_iter().collect();
            assert_eq!(documented, actual, "pins of {name}");
        }
    }

    #[test]
    fn sections_stop_at_the_next_heading() {
        let and = section(Kind::And).unwrap();
        assert!(and.starts_with("1 when all inputs are 1."));
        assert!(and.ends_with("| `out` | out | Width | AND of the inputs |"));
        assert!(section(Kind::Rect).unwrap().ends_with("take part in the simulation."));
    }
}
