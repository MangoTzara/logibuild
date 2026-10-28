//! The netlist written by Yosys' `write_json`: modules with their ports,
//! cells and named wires. Signals are vectors of bits, least significant
//! first; a bit is a net number or a constant.

use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Bit {
    Net(u64),
    Zero,
    One,
    X,
    Z,
}

impl Bit {
    fn parse(v: &Value) -> Option<Bit> {
        match v {
            Value::Number(n) => n.as_u64().map(Bit::Net),
            Value::String(s) => match s.as_str() {
                "0" => Some(Bit::Zero),
                "1" => Some(Bit::One),
                "x" => Some(Bit::X),
                "z" => Some(Bit::Z),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn is_const(self) -> bool {
        !matches!(self, Bit::Net(_))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Input,
    Output,
    Inout,
}

impl Dir {
    fn parse(s: &str) -> Dir {
        match s {
            "input" => Dir::Input,
            "output" => Dir::Output,
            _ => Dir::Inout,
        }
    }
}

/// A cell parameter: a constant (bits, least significant first) or a string.
#[derive(Clone, PartialEq, Debug)]
pub enum Param {
    Bits(Vec<Bit>),
    Str(String),
}

impl Param {
    fn parse(v: &Value) -> Param {
        match v {
            Value::Number(n) => {
                let v = n.as_i64().unwrap_or(0);
                Param::Bits((0..32).map(|i| if v >> i & 1 == 1 { Bit::One } else { Bit::Zero }).collect())
            }
            Value::String(s) if !s.is_empty() && s.bytes().all(|b| b"01xz".contains(&b)) => Param::Bits(
                s.bytes().rev().map(|b| Bit::parse(&Value::String((b as char).to_string())).unwrap()).collect(),
            ),
            // Strings that look like bit patterns get a trailing space.
            Value::String(s) => Param::Str(s.strip_suffix(' ').unwrap_or(s).to_string()),
            other => Param::Str(other.to_string()),
        }
    }

    /// The value as an unsigned integer, with unknown bits read as 0.
    pub fn uint(&self) -> u64 {
        match self {
            Param::Bits(bits) => {
                bits.iter().take(64).enumerate().fold(0, |acc, (i, b)| acc | ((*b == Bit::One) as u64) << i)
            }
            Param::Str(s) => s.trim().parse().unwrap_or(0),
        }
    }

    pub fn bits(&self) -> &[Bit] {
        match self {
            Param::Bits(b) => b,
            Param::Str(_) => &[],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Port {
    pub name: String,
    pub dir: Dir,
    pub bits: Vec<Bit>,
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub name: String,
    /// The name was made up by Yosys rather than written in the source.
    pub hidden: bool,
    pub ty: String,
    pub params: Vec<(String, Param)>,
    pub connections: Vec<(String, Vec<Bit>)>,
    pub directions: Vec<(String, Dir)>,
}

impl Cell {
    pub fn param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|(n, _)| n == name).map(|(_, p)| p)
    }

    /// An integer parameter, 0 when missing.
    pub fn uint(&self, name: &str) -> u64 {
        self.param(name).map_or(0, Param::uint)
    }

    pub fn flag(&self, name: &str) -> bool {
        self.uint(name) != 0
    }

    /// The bits connected to a port, empty when unconnected.
    pub fn port(&self, name: &str) -> &[Bit] {
        self.connections.iter().find(|(n, _)| n == name).map_or(&[], |(_, b)| b.as_slice())
    }

    pub fn direction(&self, port: &str) -> Option<Dir> {
        self.directions.iter().find(|(n, _)| n == port).map(|(_, d)| *d)
    }
}

/// A named wire of a module.
#[derive(Clone, Debug)]
pub struct NetName {
    pub name: String,
    pub hidden: bool,
    pub bits: Vec<Bit>,
}

#[derive(Clone, Debug)]
pub struct Module {
    pub name: String,
    /// Declared without a body (`blackbox` attribute).
    pub blackbox: bool,
    /// Ports in declaration order.
    pub ports: Vec<Port>,
    pub cells: Vec<Cell>,
    pub netnames: Vec<NetName>,
}

#[derive(Clone, Debug, Default)]
pub struct Design {
    pub modules: Vec<Module>,
}

impl Design {
    pub fn parse(text: &str) -> Result<Design, String> {
        let root: Value = serde_json::from_str(text).map_err(|e| format!("invalid Yosys JSON: {e}"))?;
        let modules = root.get("modules").and_then(Value::as_object).ok_or("invalid Yosys JSON: no modules")?;
        Ok(Design { modules: modules.iter().map(|(name, m)| parse_module(name, m)).collect() })
    }

    pub fn module(&self, name: &str) -> Option<&Module> {
        self.modules.iter().find(|m| m.name == name)
    }
}

fn bits(v: Option<&Value>) -> Vec<Bit> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Bit::parse).collect()).unwrap_or_default()
}

fn hidden(v: &Value) -> bool {
    v.get("hide_name").and_then(Value::as_u64).unwrap_or(0) != 0
}

fn entries(v: &Value, key: &str) -> Vec<(String, Value)> {
    v.get(key)
        .and_then(Value::as_object)
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

fn parse_module(name: &str, m: &Value) -> Module {
    let blackbox = m.get("attributes").and_then(|a| a.get("blackbox")).is_some_and(|v| Param::parse(v).uint() != 0);
    let ports = entries(m, "ports")
        .into_iter()
        .map(|(name, p)| Port {
            name,
            dir: Dir::parse(p.get("direction").and_then(Value::as_str).unwrap_or("")),
            bits: bits(p.get("bits")),
        })
        .collect();
    let cells = entries(m, "cells")
        .into_iter()
        .map(|(name, c)| Cell {
            hidden: hidden(&c),
            ty: c.get("type").and_then(Value::as_str).unwrap_or("").to_string(),
            params: entries(&c, "parameters").into_iter().map(|(k, v)| (k, Param::parse(&v))).collect(),
            connections: entries(&c, "connections").into_iter().map(|(k, v)| (k, bits(Some(&v)))).collect(),
            directions: entries(&c, "port_directions")
                .into_iter()
                .map(|(k, v)| (k, Dir::parse(v.as_str().unwrap_or(""))))
                .collect(),
            name,
        })
        .collect();
    let netnames = entries(m, "netnames")
        .into_iter()
        .map(|(name, n)| NetName { hidden: hidden(&n), bits: bits(n.get("bits")), name })
        .collect();
    Module { name: name.to_string(), blackbox, ports, cells, netnames }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ports_cells_and_params() {
        let text = r#"{
          "creator": "Yosys",
          "modules": {
            "inc": {
              "attributes": {"top": "00000000000000000000000000000001"},
              "ports": {
                "b": {"direction": "output", "bits": [4, 5]},
                "a": {"direction": "input", "bits": [2, 3]}
              },
              "cells": {
                "$add$inc.v:3$1": {
                  "hide_name": 1, "type": "$add",
                  "parameters": {"A_SIGNED": "00000000000000000000000000000000", "A_WIDTH": "00000000000000000000000000000010",
                                 "NAME": "10 ", "LABEL": "hello", "N": 5},
                  "port_directions": {"A": "input", "B": "input", "Y": "output"},
                  "connections": {"A": [2, 3], "B": ["1", "0"], "Y": [4, 5]}
                }
              },
              "netnames": {"a": {"hide_name": 0, "bits": [2, 3], "attributes": {}}}
            }
          }
        }"#;
        let d = Design::parse(text).unwrap();
        let m = d.module("inc").unwrap();
        assert_eq!(m.ports.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["b", "a"], "declaration order kept");
        let c = &m.cells[0];
        assert!(c.hidden);
        assert_eq!(c.uint("A_WIDTH"), 2);
        assert!(!c.flag("A_SIGNED"));
        assert_eq!(c.param("NAME"), Some(&Param::Str("10".into())));
        assert_eq!(c.param("LABEL"), Some(&Param::Str("hello".into())));
        assert_eq!(c.uint("N"), 5);
        assert_eq!(c.port("B"), [Bit::One, Bit::Zero]);
        assert_eq!(c.direction("Y"), Some(Dir::Output));
        assert!(!m.netnames[0].hidden);
    }
}
