//! Running GHDL and Yosys on the sources of a project.
//!
//! *Compile* checks the sources: GHDL imports the VHDL files (in any order)
//! and analyzes and elaborates the top entities, Yosys reads the Verilog
//! files. *Synthesize* compiles, then runs Yosys once per top to write a
//! netlist in JSON and imports it. Yosys keeps operators whole (`proc`,
//! `opt`, `memory -nomap`) so the circuits stay readable; only the operators
//! without a LogiBuild component (multiplication, division…) are lowered to
//! gates.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use lb_core::circuit::{Circuit, VhdlStd};
use lb_core::component::Kind;

use crate::import::{ImportError, Imported, import};
use crate::json::Design;
use crate::lang::{Lang, units};
use crate::log::{Diagnostic, parse_line};

/// Operators lowered to gates, having no component of their own.
const LOWERED: &str = "t:$mul t:$div t:$mod t:$divfloor t:$modfloor t:$pow t:$macc t:$alu t:$lcu";

/// Finds a program: at the configured path (the program itself, or the
/// folder holding it) when one is given, else on the `PATH` or where the
/// OSS CAD Suite is usually unpacked.
pub fn find(program: &str, configured: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{program}.exe") } else { program.to_string() };
    let configured = configured.trim();
    if !configured.is_empty() {
        let p = match configured.strip_prefix("~/").zip(home()) {
            Some((rest, home)) => home.join(rest),
            None => PathBuf::from(configured),
        };
        return if p.is_dir() { Some(p.join(&exe)).filter(|f| f.is_file()) } else { p.is_file().then_some(p) };
    }
    let on_path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    let mut usual: Vec<PathBuf> = home().into_iter().map(|h| h.join("oss-cad-suite").join("bin")).collect();
    usual.push(PathBuf::from(if cfg!(windows) { "C:\\oss-cad-suite\\bin" } else { "/opt/oss-cad-suite/bin" }));
    on_path.into_iter().chain(usual).map(|d| d.join(&exe)).find(|p| p.is_file())
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// The first line printed by `tool arg` (a version banner).
pub fn version(tool: &Path, arg: &str) -> Result<String, String> {
    let out = command(tool).arg(arg).output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(if out.stdout.is_empty() { &out.stderr } else { &out.stdout }).into_owned();
    Ok(text.lines().next().unwrap_or("").trim().to_string())
}

fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // No console window flashing for each run.
        c.creation_flags(0x0800_0000);
    }
    c
}

/// Lets another thread stop a job; the running tool is killed.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Compile,
    Synthesize,
}

#[derive(Clone, Debug)]
pub struct Job {
    /// Absolute paths, in project order.
    pub sources: Vec<PathBuf>,
    pub tops: Vec<String>,
    pub std: VhdlStd,
    /// Where GHDL keeps its library and Yosys its scripts and netlists.
    pub workdir: PathBuf,
    pub ghdl: Option<PathBuf>,
    pub yosys: Option<PathBuf>,
    /// Digest of the sources, stored in the circuits synthesized.
    pub digest: String,
}

#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub log: Vec<Diagnostic>,
    pub ok: bool,
    pub cancelled: bool,
    /// The circuits of a successful synthesis, children first.
    pub imported: Option<Imported>,
}

struct Stop;

/// Compiles or synthesizes; never panics on tool failures, which end up in
/// the log.
pub fn run(job: &Job, action: Action, cancel: &Cancel) -> Outcome {
    let mut out = Outcome::default();
    let r = run_inner(job, action, cancel, &mut out);
    out.cancelled = cancel.is_cancelled();
    out.ok = r.is_ok() && !out.cancelled;
    if out.cancelled {
        out.log.push(Diagnostic::warning("Cancelled."));
    }
    out
}

fn run_inner(job: &Job, action: Action, cancel: &Cancel, out: &mut Outcome) -> Result<(), Stop> {
    let log = &mut out.log;
    if job.sources.is_empty() {
        log.push(Diagnostic::error("The project has no HDL sources: add VHDL or Verilog files first."));
        return Err(Stop);
    }
    std::fs::create_dir_all(&job.workdir).map_err(|e| {
        log.push(Diagnostic::error(format!("Cannot create {}: {e}", job.workdir.display())));
        Stop
    })?;
    // The language of every unit, to pick the tools for each top.
    let mut unit_lang: HashMap<String, Lang> = HashMap::new();
    let (mut vhdl, mut verilog) = (Vec::new(), Vec::new());
    for s in &job.sources {
        let Some(lang) = Lang::of(s) else {
            log.push(Diagnostic::warning(format!("{}: not a VHDL or Verilog file, skipped", s.display())));
            continue;
        };
        match std::fs::read_to_string(s) {
            Ok(text) => {
                for u in units(&text, lang) {
                    unit_lang.entry(u.name).or_insert(lang);
                }
            }
            Err(e) => {
                log.push(Diagnostic::error(format!("Cannot read {}: {e}", s.display())));
                return Err(Stop);
            }
        }
        if lang == Lang::Vhdl { vhdl.push(s.clone()) } else { verilog.push(s.clone()) }
    }
    let lang_of = |top: &str| unit_lang.get(top).or_else(|| unit_lang.get(&top.to_ascii_lowercase())).copied();
    for t in &job.tops {
        if lang_of(t).is_none() {
            log.push(Diagnostic::error(format!("No entity or module “{t}” in the sources.")));
            return Err(Stop);
        }
    }
    let ghdl_opts = ghdl_options(job);

    // Compile.
    if !vhdl.is_empty() {
        let ghdl = tool(&job.ghdl, "ghdl", log)?;
        let mut c = command(&ghdl);
        c.arg("-i").args(&ghdl_opts).args(&vhdl);
        exec(&mut c, &job.workdir, cancel, log)?;
        // Elaborate the VHDL tops, or every entity when none is chosen yet.
        let mut checked: Vec<String> =
            job.tops.iter().filter(|t| lang_of(t) == Some(Lang::Vhdl)).map(|t| t.to_ascii_lowercase()).collect();
        if checked.is_empty() {
            checked = unit_lang.iter().filter(|(_, l)| **l == Lang::Vhdl).map(|(u, _)| u.clone()).collect();
            checked.sort();
        }
        for unit in checked {
            let mut c = command(&ghdl);
            c.arg("-m").args(&ghdl_opts).arg(&unit);
            exec(&mut c, &job.workdir, cancel, log)?;
        }
    }
    if !verilog.is_empty() && action == Action::Compile {
        let yosys = tool(&job.yosys, "yosys", log)?;
        let mut script = format!("read_verilog -sv {}", quoted(&verilog));
        // Modules may instantiate VHDL entities, unknown to this check.
        if vhdl.is_empty() {
            script.push_str("; hierarchy -check");
        }
        let mut c = command(&yosys);
        c.arg("-q").arg("-p").arg(script);
        exec(&mut c, &job.workdir, cancel, log)?;
    }
    if action == Action::Compile {
        log.push(Diagnostic::info("Compilation finished without errors."));
        return Ok(());
    }

    // Synthesize each top, sharing the circuits of common children.
    if job.tops.is_empty() {
        log.push(Diagnostic::error("No entity to synthesize: add one to the project from the HDL sources list."));
        return Err(Stop);
    }
    let yosys = tool(&job.yosys, "yosys", log)?;
    let mut all = Imported::default();
    for top in &job.tops {
        let lang = lang_of(top).unwrap();
        let mut map_memories = false;
        let imported = loop {
            let json = job.workdir.join(format!("{}.json", file_safe(top)));
            let script = synth_script(top, lang, &verilog, &ghdl_opts, map_memories, &json);
            let script_path = job.workdir.join(format!("{}.ys", file_safe(top)));
            if let Err(e) = std::fs::write(&script_path, &script) {
                log.push(Diagnostic::error(format!("Cannot write {}: {e}", script_path.display())));
                return Err(Stop);
            }
            let mut c = command(&yosys);
            c.arg("-q");
            if lang == Lang::Vhdl {
                c.arg("-m").arg("ghdl");
            }
            c.arg("-s").arg(&script_path);
            exec(&mut c, &job.workdir, cancel, log)?;
            let text = std::fs::read_to_string(&json).map_err(|e| {
                log.push(Diagnostic::error(format!("Cannot read the netlist {}: {e}", json.display())));
                Stop
            })?;
            let design = Design::parse(&text).map_err(|e| {
                log.push(Diagnostic::error(e));
                Stop
            })?;
            match import(&design, std::slice::from_ref(top), &job.digest) {
                Ok(i) => break i,
                Err(ImportError::NeedsMemoryMap(why)) if !map_memories => {
                    log.push(Diagnostic::warning(format!("{why}: it becomes flip-flops and multiplexers.")));
                    map_memories = true;
                }
                Err(e) => {
                    log.push(Diagnostic::error(e.to_string()));
                    return Err(Stop);
                }
            }
        };
        for w in imported.warnings {
            log.push(Diagnostic::warning(w));
        }
        let mut circuits = imported.circuits;
        entity_names(&mut circuits, unit_lang.keys());
        for c in circuits {
            if all.circuits.iter().all(|x| x.name != c.name) {
                all.circuits.push(c);
            }
        }
    }
    let names: Vec<&str> = all.circuits.iter().map(|c| c.name.as_str()).collect();
    log.push(Diagnostic::info(format!("Synthesis finished: {}.", names.join(", "))));
    out.imported = Some(all);
    Ok(())
}

/// Names the circuits of VHDL entities after the entity: GHDL calls an
/// instantiated entity `<entity>_B<architecture>` (`counter_Brtl`). A name
/// another circuit already has is kept.
pub fn entity_names<'a>(circuits: &mut [Circuit], units: impl Iterator<Item = &'a String>) {
    let units: Vec<&String> = units.collect();
    let mut renames: HashMap<String, String> = HashMap::new();
    for c in circuits.iter() {
        if units.iter().any(|u| **u == c.name) {
            continue;
        }
        let entity = units.iter().filter(|u| c.name.starts_with(&format!("{u}_B"))).max_by_key(|u| u.len());
        if let Some(u) = entity
            && circuits.iter().all(|x| x.name != **u)
            && renames.values().all(|v| v != *u)
        {
            renames.insert(c.name.clone(), u.to_string());
        }
    }
    for c in circuits.iter_mut() {
        if let Some(n) = renames.get(&c.name) {
            c.name = n.clone();
        }
        for comp in c.components.iter_mut().filter(|x| x.kind == Kind::Subcircuit) {
            if let Some(n) = renames.get(&comp.props.circuit) {
                comp.props.circuit = n.clone();
            }
        }
    }
}

fn tool(configured: &Option<PathBuf>, name: &str, log: &mut Vec<Diagnostic>) -> Result<PathBuf, Stop> {
    configured.clone().ok_or_else(|| {
        log.push(Diagnostic::error(format!(
            "{name} not found: install it (e.g. the OSS CAD Suite) or set its path in Settings → HDL tools."
        )));
        Stop
    })
}

fn ghdl_options(job: &Job) -> Vec<String> {
    let std = match job.std {
        VhdlStd::V93 => "--std=93c",
        VhdlStd::V08 => "--std=08",
    };
    // Synopsys packages (std_logic_unsigned...) are common in course material.
    vec![std.into(), "-fsynopsys".into(), format!("--workdir={}", job.workdir.display())]
}

fn quoted(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| format!("\"{}\"", p.display())).collect::<Vec<_>>().join(" ")
}

/// A unit name usable in a file name.
fn file_safe(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

/// The Yosys script synthesizing `top` into `json`.
pub fn synth_script(
    top: &str,
    lang: Lang,
    verilog: &[PathBuf],
    ghdl_opts: &[String],
    map_memories: bool,
    json: &Path,
) -> String {
    let mut s = String::new();
    if !verilog.is_empty() {
        s.push_str(&format!("read_verilog -sv {}\n", quoted(verilog)));
    }
    if lang == Lang::Vhdl {
        s.push_str(&format!("ghdl {} {top}\n", ghdl_opts.join(" ")));
    }
    s.push_str(&format!("hierarchy -check -top {top}\n"));
    // `-mux_undef` drops the multiplexers that `proc` puts in front of
    // memory write ports (`we ? addr : x`), so reads and writes share an address.
    s.push_str("proc\nopt_expr -mux_undef -mux_bool\nopt_clean\n");
    s.push_str(if map_memories { "memory\n" } else { "memory -nomap\n" });
    s.push_str("opt -nosdff -mux_undef -mux_bool\npmuxtree\n");
    s.push_str(&format!("techmap {LOWERED}\n"));
    s.push_str("opt_clean\n");
    s.push_str(&format!("write_json \"{}\"\n", json.display()));
    s
}

/// Runs a tool in `dir`, logging its command line and output.
fn exec(c: &mut Command, dir: &Path, cancel: &Cancel, log: &mut Vec<Diagnostic>) -> Result<(), Stop> {
    let shown: Vec<String> =
        std::iter::once(c.get_program()).chain(c.get_args()).map(|a| a.to_string_lossy().into_owned()).collect();
    log.push(Diagnostic::info(format!("$ {}", shown.join(" "))));
    let child = c.current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = child.map_err(|e| {
        log.push(Diagnostic::error(format!("Cannot run {}: {e}", shown[0])));
        Stop
    })?;
    let reader = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut p) = pipe {
                let mut buf = Vec::new();
                let _ = p.read_to_end(&mut buf);
                s = String::from_utf8_lossy(&buf).into_owned();
            }
            s
        })
    };
    let out = reader(child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>));
    let err = reader(child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>));
    let status = loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Stop);
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                log.push(Diagnostic::error(format!("{}: {e}", shown[0])));
                return Err(Stop);
            }
        }
    };
    let text = out.join().unwrap_or_default() + &err.join().unwrap_or_default();
    log.extend(text.lines().filter(|l| !l.trim().is_empty()).map(parse_line));
    if status.success() {
        Ok(())
    } else {
        log.push(Diagnostic::error(format!("{} failed ({status}).", shown[0])));
        Err(Stop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_keeps_operators_whole() {
        let opts = vec!["--std=08".to_string(), "--workdir=/w".to_string()];
        let s = synth_script("cpu", Lang::Vhdl, &[PathBuf::from("/p/uart.v")], &opts, false, Path::new("/w/cpu.json"));
        assert!(s.starts_with("read_verilog -sv \"/p/uart.v\"\nghdl --std=08 --workdir=/w cpu\n"));
        assert!(s.contains("hierarchy -check -top cpu\n"));
        assert!(s.contains("memory -nomap\n"));
        assert!(!s.contains("flatten"), "hierarchy is kept");
        assert!(s.ends_with("write_json \"/w/cpu.json\"\n"));
        let s = synth_script("top", Lang::Verilog, &[], &opts, true, Path::new("t.json"));
        assert!(!s.contains("ghdl"));
        assert!(s.contains("memory\n"));
    }

    #[test]
    fn missing_tools_and_sources_are_reported() {
        let dir = std::env::temp_dir().join(format!("lb_hdl_tc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.vhd");
        std::fs::write(&src, "entity a is end;\narchitecture r of a is begin end;\n").unwrap();
        let mut job = Job {
            sources: vec![],
            tops: vec![],
            std: VhdlStd::V08,
            workdir: dir.join("work"),
            ghdl: None,
            yosys: None,
            digest: String::new(),
        };
        let o = run(&job, Action::Compile, &Cancel::default());
        assert!(!o.ok && o.log[0].text.contains("no HDL sources"));
        job.sources.push(src);
        job.tops.push("b".into());
        let o = run(&job, Action::Synthesize, &Cancel::default());
        assert!(o.log.iter().any(|d| d.text.contains("No entity or module “b”")));
        job.tops = vec!["A".into()];
        let o = run(&job, Action::Compile, &Cancel::default());
        assert!(!o.ok);
        assert!(o.log.iter().any(|d| d.text.starts_with("ghdl not found")), "{:?}", o.log);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ghdl_names_become_entity_names() {
        let mut top = Circuit::new("display_counter");
        let id = top.add(Kind::Subcircuit, (0, 0));
        top.component_mut(id).unwrap().props.circuit = "counter_Brtl".into();
        let mut cs = vec![Circuit::new("counter_Brtl"), Circuit::new("seg7_Bbehave"), top, Circuit::new("alu")];
        let units = ["counter".to_string(), "seg7".into(), "display_counter".into(), "alu".into()];
        entity_names(&mut cs, units.iter());
        let names: Vec<&str> = cs.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["counter", "seg7", "display_counter", "alu"]);
        assert_eq!(cs[2].components[0].props.circuit, "counter");
        // Two architectures of one entity: only the first gets the name.
        let mut cs = vec![Circuit::new("ff_Ba"), Circuit::new("ff_Bb")];
        entity_names(&mut cs, ["ff".to_string()].iter());
        assert_eq!((cs[0].name.as_str(), cs[1].name.as_str()), ("ff", "ff_Bb"));
    }

    #[test]
    fn finds_programs() {
        assert!(find("surely-not-a-real-program-name", "").is_none());
        assert!(find("x", "/definitely/missing/ghdl").is_none());
        // The program itself, or the folder holding it.
        let dir = std::env::temp_dir().join(format!("lb_hdl_find_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(if cfg!(windows) { "fake-tool.exe" } else { "fake-tool" });
        std::fs::write(&exe, "").unwrap();
        assert_eq!(find("fake-tool", &dir.display().to_string()), Some(exe.clone()));
        assert_eq!(find("fake-tool", &format!("  {}  ", exe.display())), Some(exe.clone()));
        assert!(find("other-tool", &dir.display().to_string()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
