//! Messages of the tools, with the source location they point to.

use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Level {
    Info,
    Warning,
    Error,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub level: Level,
    pub text: String,
    pub file: Option<PathBuf>,
    /// 1-based line and column.
    pub line: Option<usize>,
    pub col: Option<usize>,
}

impl Diagnostic {
    pub fn new(level: Level, text: impl Into<String>) -> Self {
        Diagnostic { level, text: text.into(), file: None, line: None, col: None }
    }

    pub fn info(text: impl Into<String>) -> Self {
        Diagnostic::new(Level::Info, text)
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Diagnostic::new(Level::Warning, text)
    }

    pub fn error(text: impl Into<String>) -> Self {
        Diagnostic::new(Level::Error, text)
    }
}

const EXTENSIONS: [&str; 4] = [".vhdl:", ".vhd:", ".sv:", ".v:"];

/// Reads a line of GHDL or Yosys output: `alu.vhd:12:5:error: ...`,
/// `top.v:3: ERROR: ...`, `Warning: ...`. A located message without a level
/// is an error, as GHDL reports them.
pub fn parse_line(line: &str) -> Diagnostic {
    let lower = line.to_ascii_lowercase();
    let mut d = Diagnostic::info(line.trim_end());
    if let Some((start, end)) = EXTENSIONS.iter().filter_map(|e| lower.find(e).map(|i| (i, i + e.len()))).min() {
        let from =
            line[..start].rfind(|c: char| c.is_whitespace() || c == '`' || c == '\'' || c == '(').map_or(0, |i| i + 1);
        let digits = |s: &str| {
            let n = s.bytes().take_while(u8::is_ascii_digit).count();
            (s[..n].parse::<usize>().ok(), n)
        };
        let (line_no, n) = digits(&line[end..]);
        if let Some(l) = line_no {
            d.file = Some(PathBuf::from(&line[from..end - 1]));
            d.line = Some(l);
            let after = &line[end + n..];
            if let Some(rest) = after.strip_prefix(':') {
                d.col = digits(rest).0;
            }
        }
    }
    let worded = |w: &str| lower.contains(w);
    d.level = if worded("error") || worded("fatal") {
        Level::Error
    } else if worded("warning") {
        Level::Warning
    } else if d.line.is_some() && !worded("note") && !worded("info") {
        Level::Error
    } else {
        Level::Info
    };
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghdl_and_yosys_messages() {
        let d = parse_line("/home/u/p/alu.vhd:12:5:error: missing \";\" at end of statement");
        assert_eq!(d.level, Level::Error);
        assert_eq!(d.file.as_deref(), Some(std::path::Path::new("/home/u/p/alu.vhd")));
        assert_eq!((d.line, d.col), (Some(12), Some(5)));

        let d = parse_line("cpu.vhd:3:1:warning: signal \"x\" is never read");
        assert_eq!((d.level, d.line), (Level::Warning, Some(3)));

        let d = parse_line("top.vhd:7:9: no declaration for \"foo\"");
        assert_eq!(d.level, Level::Error, "located GHDL message without level");

        let d = parse_line("/tmp/my top.v:4: ERROR: syntax error, unexpected TOK_ID");
        assert_eq!(d.file.as_deref(), Some(std::path::Path::new("top.v")), "path cut at spaces");
        assert_eq!((d.level, d.line, d.col), (Level::Error, Some(4), None));

        let d = parse_line("C:\\work\\uart.sv:10:2: error: x");
        assert_eq!(d.file.as_deref(), Some(std::path::Path::new("C:\\work\\uart.sv")));

        let d = parse_line("Warning: Replacing memory \\mem with list of registers.");
        assert_eq!((d.level, d.file.clone()), (Level::Warning, None));
        assert_eq!(parse_line("1. Executing Verilog frontend.").level, Level::Info);
        assert_eq!(parse_line("version.v:1: Note: x").level, Level::Info);
    }
}
