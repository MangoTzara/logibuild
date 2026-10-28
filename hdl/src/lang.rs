//! The two languages: file types, the design units declared in a source
//! and the words to highlight.

use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Lang {
    Vhdl,
    Verilog,
}

impl Lang {
    /// The language of a source file, from its extension.
    pub fn of(path: &Path) -> Option<Lang> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "vhd" | "vhdl" => Some(Lang::Vhdl),
            "v" | "sv" => Some(Lang::Verilog),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::Vhdl => "VHDL",
            Lang::Verilog => "Verilog",
        }
    }

    pub fn case_sensitive(self) -> bool {
        self == Lang::Verilog
    }

    pub fn is_keyword(self, word: &str) -> bool {
        match self {
            Lang::Vhdl => VHDL_KEYWORDS.contains(&word.to_ascii_lowercase().as_str()),
            Lang::Verilog => VERILOG_KEYWORDS.contains(&word),
        }
    }

    pub fn is_type(self, word: &str) -> bool {
        match self {
            Lang::Vhdl => VHDL_TYPES.contains(&word.to_ascii_lowercase().as_str()),
            Lang::Verilog => VERILOG_TYPES.contains(&word),
        }
    }

    /// A new source declaring an empty unit called `name`.
    pub fn template(self, name: &str) -> String {
        match self {
            Lang::Vhdl => format!(
                "library ieee;\nuse ieee.std_logic_1164.all;\nuse ieee.numeric_std.all;\n\n\
                 entity {name} is\n    port (\n        clk : in  std_logic;\n        \
                 a   : in  std_logic_vector(7 downto 0);\n        y   : out std_logic_vector(7 downto 0)\n    );\n\
                 end entity;\n\narchitecture rtl of {name} is\nbegin\n    y <= a;\nend architecture;\n"
            ),
            Lang::Verilog => format!(
                "module {name} (\n    input  wire       clk,\n    input  wire [7:0] a,\n    output wire [7:0] y\n);\n\n    \
                 assign y = a;\n\nendmodule\n"
            ),
        }
    }
}

/// A design unit declared in a source: a VHDL entity or a Verilog module.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Unit {
    /// Lower case for VHDL, as the compiler names it.
    pub name: String,
    /// 1-based line of the declaration.
    pub line: usize,
}

/// The entities or modules declared in `text`.
pub fn units(text: &str, lang: Lang) -> Vec<Unit> {
    let words = words(text, lang);
    let mut out = Vec::new();
    for (k, (word, line)) in words.iter().enumerate() {
        let next = words.get(k + 1).map(|w| w.0.as_str());
        let after = words.get(k + 2).map(|w| w.0.to_ascii_lowercase());
        let found = match lang {
            Lang::Vhdl => word.eq_ignore_ascii_case("entity") && after.as_deref() == Some("is"),
            Lang::Verilog => word == "module" || word == "macromodule",
        };
        if found && let Some(name) = next {
            let name = if lang == Lang::Vhdl { name.to_ascii_lowercase() } else { name.to_string() };
            out.push(Unit { name, line: *line });
        }
    }
    out
}

/// Identifiers and keywords outside comments and strings, with their line.
fn words(text: &str, lang: Lang) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut block = false;
    for (ln, line) in text.lines().enumerate() {
        for tok in Tokens::new(line, lang, &mut block) {
            if tok.class == Class::Word || tok.class == Class::Keyword || tok.class == Class::Type {
                out.push((line[tok.range].to_string(), ln + 1));
            }
        }
    }
    out
}

/// Kind of a piece of source text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Keyword,
    Type,
    Word,
    Number,
    String,
    Comment,
    Other,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Token {
    pub range: std::ops::Range<usize>,
    pub class: Class,
}

/// The tokens of one line. `block` tracks a Verilog `/* */` comment
/// spanning lines: it is read on entry and updated on exit.
pub struct Tokens<'a> {
    line: &'a str,
    pos: usize,
    lang: Lang,
    block: &'a mut bool,
}

impl<'a> Tokens<'a> {
    pub fn new(line: &'a str, lang: Lang, block: &'a mut bool) -> Self {
        Tokens { line, pos: 0, lang, block }
    }
}

impl Iterator for Tokens<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        let bytes = self.line.as_bytes();
        let start = self.pos;
        if start >= bytes.len() {
            return None;
        }
        let rest = &self.line[start..];
        let class;
        if *self.block {
            match rest.find("*/") {
                Some(i) => {
                    self.pos = start + i + 2;
                    *self.block = false;
                }
                None => self.pos = bytes.len(),
            }
            class = Class::Comment;
        } else if rest.starts_with("--") && self.lang == Lang::Vhdl
            || rest.starts_with("//") && self.lang == Lang::Verilog
        {
            self.pos = bytes.len();
            class = Class::Comment;
        } else if let Some(body) = rest.strip_prefix("/*") {
            match body.find("*/") {
                Some(i) => self.pos = start + 2 + i + 2,
                None => {
                    self.pos = bytes.len();
                    *self.block = true;
                }
            }
            class = Class::Comment;
        } else if let Some(body) = rest.strip_prefix('"') {
            self.pos = start + 1 + body.find('"').map_or(body.len(), |i| i + 1);
            class = Class::String;
        } else {
            let c = rest.chars().next().unwrap();
            if c.is_ascii_alphabetic() || c == '_' || c == '\\' || c == '$' || c == '`' {
                let first = c.len_utf8();
                let len = rest[first..]
                    .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'))
                    .map_or(rest.len(), |i| first + i);
                self.pos = start + len;
                let word = &self.line[start..self.pos];
                class = if self.lang.is_keyword(word) {
                    Class::Keyword
                } else if self.lang.is_type(word) {
                    Class::Type
                } else {
                    Class::Word
                };
            } else if c.is_ascii_digit() || c == '\'' && self.lang == Lang::Verilog {
                // Numbers with bases and units: 8'hFF, 16#FF#, 10.5, 3e2.
                let len = rest
                    .find(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '\'' | '#' | '.')))
                    .unwrap_or(rest.len());
                self.pos = start + len.max(1);
                class = Class::Number;
            } else if c == '\'' && self.lang == Lang::Vhdl && rest.len() >= 3 && rest.as_bytes()[2] == b'\'' {
                // Character literal: '0', '1', 'Z'.
                self.pos = start + 3;
                class = Class::Number;
            } else {
                self.pos = start + c.len_utf8();
                class = Class::Other;
            }
        }
        Some(Token { range: start..self.pos, class })
    }
}

const VHDL_KEYWORDS: &[&str] = &[
    "abs",
    "access",
    "after",
    "alias",
    "all",
    "and",
    "architecture",
    "array",
    "assert",
    "attribute",
    "begin",
    "block",
    "body",
    "buffer",
    "bus",
    "case",
    "component",
    "configuration",
    "constant",
    "context",
    "disconnect",
    "downto",
    "else",
    "elsif",
    "end",
    "entity",
    "exit",
    "file",
    "for",
    "force",
    "function",
    "generate",
    "generic",
    "group",
    "guarded",
    "if",
    "impure",
    "in",
    "inertial",
    "inout",
    "is",
    "label",
    "library",
    "linkage",
    "literal",
    "loop",
    "map",
    "mod",
    "nand",
    "new",
    "next",
    "nor",
    "not",
    "null",
    "of",
    "on",
    "open",
    "or",
    "others",
    "out",
    "package",
    "port",
    "postponed",
    "procedure",
    "process",
    "protected",
    "pure",
    "range",
    "record",
    "register",
    "reject",
    "release",
    "rem",
    "report",
    "return",
    "rol",
    "ror",
    "select",
    "severity",
    "shared",
    "signal",
    "sla",
    "sll",
    "sra",
    "srl",
    "subtype",
    "then",
    "to",
    "transport",
    "type",
    "unaffected",
    "units",
    "until",
    "use",
    "variable",
    "wait",
    "when",
    "while",
    "with",
    "xnor",
    "xor",
];

const VHDL_TYPES: &[&str] = &[
    "bit",
    "bit_vector",
    "boolean",
    "character",
    "integer",
    "natural",
    "positive",
    "real",
    "signed",
    "std_logic",
    "std_logic_vector",
    "std_ulogic",
    "std_ulogic_vector",
    "string",
    "time",
    "unsigned",
    "rising_edge",
    "falling_edge",
    "to_integer",
    "to_unsigned",
    "to_signed",
    "resize",
    "ieee",
    "std_logic_1164",
    "numeric_std",
];

const VERILOG_KEYWORDS: &[&str] = &[
    "always",
    "always_comb",
    "always_ff",
    "always_latch",
    "and",
    "assign",
    "automatic",
    "begin",
    "case",
    "casex",
    "casez",
    "default",
    "defparam",
    "else",
    "end",
    "endcase",
    "endfunction",
    "endgenerate",
    "endmodule",
    "endtask",
    "for",
    "forever",
    "function",
    "generate",
    "genvar",
    "if",
    "initial",
    "inout",
    "input",
    "localparam",
    "macromodule",
    "module",
    "nand",
    "negedge",
    "nor",
    "not",
    "or",
    "output",
    "parameter",
    "posedge",
    "repeat",
    "signed",
    "task",
    "unsigned",
    "while",
    "xnor",
    "xor",
    "`define",
    "`include",
    "`ifdef",
    "`ifndef",
    "`else",
    "`endif",
    "`timescale",
];

const VERILOG_TYPES: &[&str] = &["integer", "logic", "reg", "supply0", "supply1", "tri", "wire", "bit", "byte", "int"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_units() {
        let vhdl =
            "-- entity fake is\nlibrary ieee;\nENTITY Alu is port (a : in bit);\nend;\nentity work.x port map;\n";
        assert_eq!(units(vhdl, Lang::Vhdl), vec![Unit { name: "alu".into(), line: 3 }]);
        let v = "/* module fake;\n*/ module top(input a);\nendmodule\n// module nope\nmodule Sub #(parameter W=1) ();";
        let names: Vec<_> = units(v, Lang::Verilog).into_iter().map(|u| (u.name, u.line)).collect();
        assert_eq!(names, [("top".to_string(), 2), ("Sub".to_string(), 5)]);
    }

    #[test]
    fn classifies_tokens() {
        let mut block = false;
        let line = "signal q : std_logic_vector(7 downto 0) := x\"0F\"; -- note";
        let toks: Vec<(String, Class)> =
            Tokens::new(line, Lang::Vhdl, &mut block).map(|t| (line[t.range].to_string(), t.class)).collect();
        assert!(toks.contains(&("signal".into(), Class::Keyword)));
        assert!(toks.contains(&("std_logic_vector".into(), Class::Type)));
        assert!(toks.contains(&("7".into(), Class::Number)));
        assert!(toks.contains(&("\"0F\"".into(), Class::String)));
        assert!(toks.contains(&("-- note".into(), Class::Comment)));
        let mut block = false;
        let toks: Vec<Class> = Tokens::new("a /* open", Lang::Verilog, &mut block).map(|t| t.class).collect();
        assert_eq!(toks.last(), Some(&Class::Comment));
        assert!(block, "comment continues on the next line");
        let toks: Vec<Class> = Tokens::new("still */ wire", Lang::Verilog, &mut block).map(|t| t.class).collect();
        assert_eq!(toks, [Class::Comment, Class::Other, Class::Type]);
        assert!(!block);
    }

    #[test]
    fn languages() {
        assert_eq!(Lang::of(Path::new("a/B.VHD")), Some(Lang::Vhdl));
        assert_eq!(Lang::of(Path::new("x.sv")), Some(Lang::Verilog));
        assert_eq!(Lang::of(Path::new("x.txt")), None);
        for lang in [Lang::Vhdl, Lang::Verilog] {
            let names: Vec<String> = units(&lang.template("blink"), lang).into_iter().map(|u| u.name).collect();
            assert_eq!(names, ["blink"]);
        }
    }
}
