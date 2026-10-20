//! Loading and saving `.lbc` project files (RON) and `.hex` memory images.
//!
//! The writer lays out the file by hand, one component or wire per line, so
//! that edits produce small, readable diffs. Saving is atomic (temporary
//! file + rename) and keeps the previous version as `<name>.lbc.bak`.

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::circuit::{Circuit, Component, Origin, Project, Wire};
use crate::component::Kind;

pub const FORMAT_VERSION: u32 = 1;
pub const EXTENSION: &str = "lbc";

#[derive(Debug)]
pub enum IoError {
    Io(PathBuf, std::io::Error),
    Parse(PathBuf, String),
    Version(PathBuf, u32),
    Hex(PathBuf, String),
}

impl std::fmt::Display for IoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IoError::Io(p, e) => write!(f, "{}: {e}", p.display()),
            IoError::Parse(p, e) => write!(f, "{}: invalid file: {e}", p.display()),
            IoError::Version(p, v) => {
                write!(f, "{}: format version {v} is not supported (maximum {FORMAT_VERSION})", p.display())
            }
            IoError::Hex(p, e) => write!(f, "{}: invalid hex file: {e}", p.display()),
        }
    }
}

impl std::error::Error for IoError {}

#[derive(Serialize, Deserialize)]
struct FileV1 {
    version: u32,
    name: String,
    #[serde(default)]
    components: Vec<Component>,
    #[serde(default)]
    wires: Vec<Wire>,
    #[serde(default)]
    subcircuits: Vec<Circuit>,
    #[serde(default)]
    libraries: Vec<String>,
}

fn compact<T: Serialize>(v: &T) -> String {
    let cfg = ron::ser::PrettyConfig::new().new_line("").indentor("").struct_names(false).separate_tuple_members(false);
    let s = ron::ser::to_string_pretty(v, cfg).expect("serializable");
    tidy(&s)
}

/// Adds a space after commas outside string literals: `(a: 1,b: 2)` becomes
/// `(a: 1, b: 2)`.
fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    let mut in_str = false;
    let mut escaped = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            ',' => match chars.peek() {
                Some(')') | Some(']') => {
                    out.pop();
                }
                Some(' ') => {}
                _ => out.push(' '),
            },
            _ => {}
        }
    }
    out
}

fn write_circuit_body(out: &mut String, c: &Circuit, indent: &str) {
    let _ = writeln!(out, "{indent}name: {},", compact(&c.name));
    let _ = writeln!(out, "{indent}components: [");
    let mut comps: Vec<&Component> = c.components.iter().collect();
    comps.sort_by_key(|c| c.id);
    for comp in comps {
        let _ = writeln!(out, "{indent}    {},", compact(comp));
    }
    let _ = writeln!(out, "{indent}],");
    let _ = writeln!(out, "{indent}wires: [");
    let mut wires: Vec<&Wire> = c.wires.iter().collect();
    wires.sort();
    for w in wires {
        let _ = writeln!(out, "{indent}    {},", compact(w));
    }
    let _ = writeln!(out, "{indent}],");
}

/// Serializes a project to the textual `.lbc` format. Circuits loaded from
/// library folders are not embedded.
pub fn to_ron(project: &Project) -> String {
    let mut out = String::from("(\n");
    let _ = writeln!(out, "    version: {FORMAT_VERSION},");
    write_circuit_body(&mut out, project.main(), "    ");
    let embedded: Vec<&Circuit> = project.circuits[1..].iter().filter(|c| c.origin == Origin::Embedded).collect();
    if !embedded.is_empty() {
        out.push_str("    subcircuits: [\n");
        let mut sorted = embedded;
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        for c in sorted {
            out.push_str("        (\n");
            write_circuit_body(&mut out, c, "            ");
            out.push_str("        ),\n");
        }
        out.push_str("    ],\n");
    }
    if !project.libraries.is_empty() {
        let _ = writeln!(out, "    libraries: {},", compact(&project.libraries));
    }
    out.push_str(")\n");
    out
}

/// Parses the textual format. Library references are not resolved.
pub fn from_ron(text: &str, path: &Path) -> Result<Project, IoError> {
    let file: FileV1 = ron::from_str(text).map_err(|e| IoError::Parse(path.to_path_buf(), e.to_string()))?;
    if file.version == 0 || file.version > FORMAT_VERSION {
        return Err(IoError::Version(path.to_path_buf(), file.version));
    }
    let mut main = Circuit::new(file.name);
    main.components = file.components;
    main.wires = file.wires;
    let mut circuits = vec![main];
    circuits.extend(file.subcircuits);
    for c in &mut circuits {
        c.normalize();
    }
    Ok(Project { circuits, libraries: file.libraries })
}

/// Loads a project file, its external `.hex` memory images and the
/// subcircuits found in its library folders.
pub fn load(path: &Path) -> Result<Project, IoError> {
    let text = fs::read_to_string(path).map_err(|e| IoError::Io(path.to_path_buf(), e))?;
    let mut project = from_ron(&text, path)?;
    let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    load_data_files(&mut project, &base)?;
    resolve_libraries(&mut project, &base)?;
    Ok(project)
}

fn load_data_files(project: &mut Project, base: &Path) -> Result<(), IoError> {
    for c in &mut project.circuits {
        for comp in &mut c.components {
            if matches!(comp.kind, Kind::Rom | Kind::Ram) && !comp.props.data_file.is_empty() {
                let p = base.join(&comp.props.data_file);
                let text = fs::read_to_string(&p).map_err(|e| IoError::Io(p.clone(), e))?;
                let words = parse_hex(&text).map_err(|e| IoError::Hex(p.clone(), e))?;
                comp.props.data = format_hex(&words, false);
            }
        }
    }
    Ok(())
}

/// Loads missing subcircuits from `<base>/<library>/<name>.lbc`.
fn resolve_libraries(project: &mut Project, base: &Path) -> Result<(), IoError> {
    if project.libraries.is_empty() {
        return Ok(());
    }
    let dirs: Vec<PathBuf> = project.libraries.iter().map(|l| base.join(l)).collect();
    loop {
        let missing: Vec<String> = project
            .circuits
            .iter()
            .flat_map(|c| c.subcircuit_refs())
            .filter(|n| project.circuit(n).is_none())
            .map(str::to_string)
            .collect();
        let mut found_any = false;
        for name in missing {
            if project.circuit(&name).is_some() {
                continue;
            }
            let Some(file) = dirs.iter().map(|d| d.join(format!("{name}.{EXTENSION}"))).find(|p| p.exists()) else {
                continue;
            };
            let text = fs::read_to_string(&file).map_err(|e| IoError::Io(file.clone(), e))?;
            let mut lib = from_ron(&text, &file)?;
            let lib_base = file.parent().unwrap_or(Path::new(".")).to_path_buf();
            load_data_files(&mut lib, &lib_base)?;
            for mut c in lib.circuits {
                if project.circuit(&c.name).is_none() {
                    c.origin = Origin::Library(file.clone());
                    project.circuits.push(c);
                    found_any = true;
                }
            }
        }
        if !found_any {
            return Ok(());
        }
    }
}

/// Writes `contents` to `path` atomically, keeping the previous file as
/// `<path>.bak`.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), IoError> {
    let err = |e| IoError::Io(path.to_path_buf(), e);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut f = fs::File::create(&tmp).map_err(err)?;
        f.write_all(contents).map_err(err)?;
        f.sync_all().map_err(err)?;
    }
    if path.exists() {
        let mut bak = path.as_os_str().to_owned();
        bak.push(".bak");
        fs::copy(path, PathBuf::from(bak)).map_err(err)?;
    }
    fs::rename(&tmp, path).map_err(err)
}

/// Saves the project. Memories with a `data_file` are written to that file
/// (relative to the project) instead of inline.
pub fn save(project: &Project, path: &Path) -> Result<(), IoError> {
    let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut copy = project.clone();
    for c in copy.circuits.iter_mut().filter(|c| c.origin == Origin::Embedded) {
        for comp in &mut c.components {
            if matches!(comp.kind, Kind::Rom | Kind::Ram) && !comp.props.data_file.is_empty() {
                let words = parse_hex(&comp.props.data).unwrap_or_default();
                let p = base.join(&comp.props.data_file);
                write_atomic(&p, format_hex(&words, true).as_bytes())?;
                comp.props.data.clear();
            }
        }
    }
    write_atomic(path, to_ron(&copy).as_bytes())
}

// ---- hex memory images --------------------------------------------------------

/// Parses a memory image: whitespace-separated hexadecimal words, with
/// `N*value` for runs, `#` comments and an optional `v2.0 raw` header
/// (compatible with Logisim images).
pub fn parse_hex(text: &str) -> Result<Vec<u64>, String> {
    let mut words = Vec::new();
    for (ln, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if ln == 0 && line.starts_with("v2.0") {
            continue;
        }
        for tok in line.split_whitespace() {
            let (count, val) = match tok.split_once('*') {
                Some((n, v)) => (n.parse::<usize>().map_err(|_| format!("invalid repeat count: {tok}"))?, v),
                None => (1, tok),
            };
            let val = val.trim_start_matches("0x");
            let v = u64::from_str_radix(val, 16).map_err(|_| format!("invalid value: {tok}"))?;
            if words.len() + count > 1 << 24 {
                return Err("image too large".into());
            }
            words.extend(std::iter::repeat_n(v, count));
        }
    }
    Ok(words)
}

/// Formats memory words, eight per line, compressing runs of four or more
/// equal words and dropping trailing zeros.
pub fn format_hex(words: &[u64], header: bool) -> String {
    let end = words.iter().rposition(|&w| w != 0).map_or(0, |i| i + 1);
    let words = &words[..end];
    let mut toks = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let mut j = i;
        while j < words.len() && words[j] == words[i] {
            j += 1;
        }
        let run = j - i;
        if run >= 4 {
            toks.push(format!("{run}*{:x}", words[i]));
        } else {
            toks.extend(std::iter::repeat_n(format!("{:x}", words[i]), run));
        }
        i = j;
    }
    let body = toks.chunks(8).map(|c| c.join(" ")).collect::<Vec<_>>().join("\n");
    if header { format!("v2.0 raw\n{body}\n") } else { body }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{add, wire};

    #[test]
    fn old_hex_displays_become_number_displays() {
        let text = "(version: 1, name: \"m\", components: [(id: 1, kind: HexDisplay, pos: (2, 3))])";
        let p = from_ron(text, Path::new("old.lbc")).unwrap();
        let c = &p.main().components[0];
        assert_eq!(c.kind, Kind::NumDisplay);
        assert_eq!((c.props.width(), c.props.radix, c.props.num_digits()), (4, crate::value::Radix::Hex, 1));
        assert!(to_ron(&p).contains("kind: NumDisplay, pos: (2, 3), props: (width: 4, digits: 1)"));
    }

    fn half_adder() -> Project {
        let mut c = Circuit::new("half_adder");
        let a = add(&mut c, Kind::Input, "A");
        let b = add(&mut c, Kind::Input, "B");
        let x = add(&mut c, Kind::Xor, "");
        let n = add(&mut c, Kind::And, "");
        let s = add(&mut c, Kind::Output, "SUM");
        let k = add(&mut c, Kind::Output, "CARRY");
        for (id, pos) in [(a, (0, 0)), (b, (0, 4)), (x, (6, 0)), (n, (6, 4)), (s, (12, 0)), (k, (12, 4))] {
            c.component_mut(id).unwrap().pos = pos;
        }
        wire(&mut c, a, "out", x, "in0");
        wire(&mut c, b, "out", x, "in1");
        wire(&mut c, a, "out", n, "in0");
        wire(&mut c, b, "out", n, "in1");
        wire(&mut c, x, "out", s, "in");
        wire(&mut c, n, "out", k, "in");
        Project::new(c)
    }

    #[test]
    fn golden_half_adder() {
        let text = to_ron(&half_adder());
        let golden = include_str!("../tests/golden/half_adder.lbc");
        assert_eq!(text, golden);
    }

    #[test]
    fn spec_example_parses() {
        let text = r#"(
    version: 1,
    name: "half_adder",
    components: [
        (id: 1, kind: Input,  pos: (0, 0),  props: (width: 1, label: "A")),
        (id: 2, kind: Input,  pos: (0, 4),  props: (width: 1, label: "B")),
        (id: 3, kind: Xor,    pos: (6, 0),  props: (inputs: 2)),
        (id: 4, kind: And,    pos: (6, 4),  props: (inputs: 2)),
        (id: 5, kind: Output, pos: (12, 0), props: (width: 1, label: "SUM")),
        (id: 6, kind: Output, pos: (12, 4), props: (width: 1, label: "CARRY")),
    ],
    wires: [
        (from: (1, "out"), to: (3, "in0")),
        (from: (2, "out"), to: (3, "in1")),
        (from: (1, "out"), to: (4, "in0")),
        (from: (2, "out"), to: (4, "in1")),
        (from: (3, "out"), to: (5, "in")),
        (from: (4, "out"), to: (6, "in")),
    ],
)"#;
        let p = from_ron(text, Path::new("x.lbc")).unwrap();
        assert_eq!(p.main().components.len(), 6);
        assert_eq!(p.main().wires.len(), 6);
        assert_eq!(p.main().components[0].props.label, "A");
    }

    #[test]
    fn round_trip_with_subcircuits() {
        let mut p = half_adder();
        let mut top = Circuit::new("top");
        let s = add(&mut top, Kind::Subcircuit, "ha");
        top.component_mut(s).unwrap().props.circuit = "half_adder".into();
        top.component_mut(s).unwrap().rot = 1;
        let r = add(&mut top, Kind::Rom, "rom");
        top.component_mut(r).unwrap().props.data = "1 2 3".into();
        let t = add(&mut top, Kind::Text, "note with \"quotes\", commas");
        top.component_mut(t).unwrap().mirror = true;
        p.circuits.insert(0, top);
        p.libraries.push("lib".into());
        let text = to_ron(&p);
        let back = from_ron(&text, Path::new("x.lbc")).unwrap();
        assert_eq!(back, p);
        assert_eq!(to_ron(&back), text, "stable output");
    }

    #[test]
    fn version_checked() {
        let text = "(version: 99, name: \"x\")";
        assert!(matches!(from_ron(text, Path::new("x")), Err(IoError::Version(_, 99))));
    }

    #[test]
    fn hex_round_trip() {
        let words = vec![1, 2, 0xff, 0, 0, 0, 0, 0, 7, 7, 7, 7, 7, 1, 2, 3, 4, 5, 6, 0, 0];
        let text = format_hex(&words, true);
        assert!(text.starts_with("v2.0 raw\n"));
        let back = parse_hex(&text).unwrap();
        assert_eq!(back, words[..19].to_vec());
        assert_eq!(parse_hex("# comment\n3*a 0x10").unwrap(), vec![10, 10, 10, 16]);
        assert!(parse_hex("zz").is_err());
        assert_eq!(format_hex(&[1, 2, 3], false), "1 2 3");
    }

    #[test]
    fn save_and_load_files() {
        let dir = std::env::temp_dir().join(format!("lb_io_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("lib")).unwrap();
        // Library with the half adder.
        save(&half_adder(), &dir.join("lib/half_adder.lbc")).unwrap();
        // Project using it, with an external ROM image.
        let mut top = Circuit::new("top");
        let s = add(&mut top, Kind::Subcircuit, "");
        top.component_mut(s).unwrap().props.circuit = "half_adder".into();
        let r = add(&mut top, Kind::Rom, "");
        top.component_mut(r).unwrap().props.data = "de ad be ef".into();
        top.component_mut(r).unwrap().props.data_file = "rom.hex".into();
        let mut p = Project::new(top);
        p.libraries.push("lib".into());
        let path = dir.join("top.lbc");
        save(&p, &path).unwrap();
        save(&p, &path).unwrap();
        assert!(dir.join("top.lbc.bak").exists(), "backup kept");
        assert!(!fs::read_to_string(&path).unwrap().contains("dead"), "data not inline");
        assert!(fs::read_to_string(dir.join("rom.hex")).unwrap().contains("de ad be ef"));

        let back = load(&path).unwrap();
        assert_eq!(back.circuits.len(), 2);
        assert!(matches!(back.circuits[1].origin, Origin::Library(_)));
        assert_eq!(back.main().components[1].props.data, "de ad be ef");
        assert!(crate::netlist::compile(&back, "top").is_ok());
        // Library circuits are not embedded when saving.
        assert!(!to_ron(&back).contains("subcircuits"));
        let _ = fs::remove_dir_all(&dir);
    }
}
