//! Four-valued signal model (`0`, `1`, `X`, `Z`) for buses of 1 to 64 bits.
//!
//! A [`Signal`] stores three bit masks. For each bit position `i < width`:
//!
//! | `hiz` | `unk` | `val` | meaning |
//! |-------|-------|-------|---------|
//! | 1     | 0     | 0     | `Z`     |
//! | 0     | 1     | 0     | `X`     |
//! | 0     | 0     | v     | `v`     |
//!
//! Bits at or above `width` are always zero, and `val` is zero wherever the bit
//! is `X` or `Z`. Every constructor normalizes to this form, so derived
//! equality is semantic equality.

use std::fmt;

use serde::{Deserialize, Serialize};

pub const MAX_WIDTH: u8 = 64;

/// One bit of a [`Signal`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Bit {
    Zero,
    One,
    X,
    Z,
}

impl Bit {
    pub fn to_char(self) -> char {
        match self {
            Bit::Zero => '0',
            Bit::One => '1',
            Bit::X => 'x',
            Bit::Z => 'z',
        }
    }
}

/// Display radix for bus values.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum Radix {
    Bin,
    #[default]
    Hex,
    Dec,
    Signed,
}

impl Radix {
    pub const ALL: [Radix; 4] = [Radix::Bin, Radix::Hex, Radix::Dec, Radix::Signed];
}

impl fmt::Display for Radix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Radix::Bin => "Bin",
            Radix::Hex => "Hex",
            Radix::Dec => "Dec",
            Radix::Signed => "Signed",
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Signal {
    width: u8,
    val: u64,
    unk: u64,
    hiz: u64,
}

impl Default for Signal {
    fn default() -> Self {
        Signal::z(1)
    }
}

/// Mask with the low `width` bits set.
#[inline]
pub const fn mask(width: u8) -> u64 {
    if width >= 64 { u64::MAX } else { (1u64 << width) - 1 }
}

#[inline]
fn clamp_width(width: u8) -> u8 {
    width.clamp(1, MAX_WIDTH)
}

impl Signal {
    /// A fully known value, truncated to `width` bits.
    #[inline]
    pub fn new(width: u8, val: u64) -> Self {
        let width = clamp_width(width);
        Signal { width, val: val & mask(width), unk: 0, hiz: 0 }
    }

    #[inline]
    pub fn bool(b: bool) -> Self {
        Signal::new(1, b as u64)
    }

    #[inline]
    pub fn zero(width: u8) -> Self {
        Signal::new(width, 0)
    }

    #[inline]
    pub fn ones(width: u8) -> Self {
        Signal::new(width, u64::MAX)
    }

    /// All bits unknown.
    #[inline]
    pub fn x(width: u8) -> Self {
        let width = clamp_width(width);
        Signal { width, val: 0, unk: mask(width), hiz: 0 }
    }

    /// All bits floating.
    #[inline]
    pub fn z(width: u8) -> Self {
        let width = clamp_width(width);
        Signal { width, val: 0, unk: 0, hiz: mask(width) }
    }

    /// Builds a signal from raw masks, normalizing them (`hiz` wins over `unk`,
    /// which wins over `val`).
    #[inline]
    pub fn from_parts(width: u8, val: u64, unk: u64, hiz: u64) -> Self {
        let width = clamp_width(width);
        let m = mask(width);
        let hiz = hiz & m;
        let unk = unk & m & !hiz;
        let val = val & m & !(unk | hiz);
        Signal { width, val, unk, hiz }
    }

    #[inline]
    pub fn width(&self) -> u8 {
        self.width
    }

    #[inline]
    pub fn val(&self) -> u64 {
        self.val
    }

    #[inline]
    pub fn unk(&self) -> u64 {
        self.unk
    }

    #[inline]
    pub fn hiz(&self) -> u64 {
        self.hiz
    }

    #[inline]
    pub fn mask(&self) -> u64 {
        mask(self.width)
    }

    /// Bits that are a definite `0` or `1`.
    #[inline]
    pub fn known_mask(&self) -> u64 {
        self.mask() & !(self.unk | self.hiz)
    }

    /// Bits that are a definite `0`.
    #[inline]
    fn zero_mask(&self) -> u64 {
        self.known_mask() & !self.val
    }

    #[inline]
    pub fn is_known(&self) -> bool {
        self.unk | self.hiz == 0
    }

    #[inline]
    pub fn is_all_z(&self) -> bool {
        self.hiz == self.mask()
    }

    #[inline]
    pub fn has_x(&self) -> bool {
        self.unk != 0
    }

    #[inline]
    pub fn has_z(&self) -> bool {
        self.hiz != 0
    }

    /// The numeric value, if every bit is known.
    #[inline]
    pub fn to_u64(&self) -> Option<u64> {
        self.is_known().then_some(self.val)
    }

    /// The value sign-extended to 64 bits, if every bit is known.
    pub fn to_i64(&self) -> Option<i64> {
        let v = self.to_u64()?;
        let shift = 64 - self.width as u32;
        Some(((v << shift) as i64) >> shift)
    }

    /// `Some(true/false)` if bit 0 is known.
    #[inline]
    pub fn as_bool(&self) -> Option<bool> {
        match self.bit(0) {
            Bit::Zero => Some(false),
            Bit::One => Some(true),
            _ => None,
        }
    }

    #[inline]
    pub fn is_one(&self) -> bool {
        self.as_bool() == Some(true)
    }

    #[inline]
    pub fn bit(&self, i: u8) -> Bit {
        if i >= self.width {
            return Bit::Z;
        }
        let b = 1u64 << i;
        if self.hiz & b != 0 {
            Bit::Z
        } else if self.unk & b != 0 {
            Bit::X
        } else if self.val & b != 0 {
            Bit::One
        } else {
            Bit::Zero
        }
    }

    pub fn with_bit(self, i: u8, bit: Bit) -> Self {
        if i >= self.width {
            return self;
        }
        let b = 1u64 << i;
        let (mut val, mut unk, mut hiz) = (self.val & !b, self.unk & !b, self.hiz & !b);
        match bit {
            Bit::Zero => {}
            Bit::One => val |= b,
            Bit::X => unk |= b,
            Bit::Z => hiz |= b,
        }
        Signal::from_parts(self.width, val, unk, hiz)
    }

    /// Converts `Z` bits to `X`: what a gate input sees when it reads a
    /// floating line.
    #[inline]
    pub fn z_to_x(self) -> Self {
        Signal { width: self.width, val: self.val, unk: self.unk | self.hiz, hiz: 0 }
    }

    /// Truncates or zero-extends to `width`.
    #[inline]
    pub fn resize(self, width: u8) -> Self {
        Signal::from_parts(width, self.val, self.unk, self.hiz)
    }

    /// Extends to `width`, replicating the most significant bit.
    pub fn sign_extend(self, width: u8) -> Self {
        let width = clamp_width(width);
        if width <= self.width {
            return self.resize(width);
        }
        let ext = mask(width) & !self.mask();
        let top = self.bit(self.width - 1);
        let (mut val, mut unk, mut hiz) = (self.val, self.unk, self.hiz);
        match top {
            Bit::Zero => {}
            Bit::One => val |= ext,
            Bit::X => unk |= ext,
            Bit::Z => hiz |= ext,
        }
        Signal::from_parts(width, val, unk, hiz)
    }

    /// Bits `lo .. lo + width`.
    #[inline]
    pub fn slice(self, lo: u8, width: u8) -> Self {
        if lo >= 64 {
            return Signal::z(width);
        }
        Signal::from_parts(width, self.val >> lo, self.unk >> lo, self.hiz >> lo)
    }

    /// Places `other` at bit `lo` of `self` (overwriting those bits).
    pub fn insert(self, lo: u8, other: Signal) -> Self {
        if lo >= self.width {
            return self;
        }
        let m = (other.mask() << lo) & self.mask();
        Signal::from_parts(
            self.width,
            (self.val & !m) | ((other.val << lo) & m),
            (self.unk & !m) | ((other.unk << lo) & m),
            (self.hiz & !m) | ((other.hiz << lo) & m),
        )
    }

    // ---- logic -------------------------------------------------------------
    //
    // Gate operations treat `Z` inputs as `X`. Both operands are expected to
    // have the same width; the result takes the width of `self`.

    #[inline]
    pub fn and(self, o: Signal) -> Signal {
        let m = self.mask();
        let zero = self.zero_mask() | o.zero_mask();
        let one = self.val & o.val;
        Signal::from_parts(self.width, one, m & !(zero | one), 0)
    }

    #[inline]
    pub fn or(self, o: Signal) -> Signal {
        let m = self.mask();
        let one = self.val | o.val;
        let zero = self.zero_mask() & o.zero_mask();
        Signal::from_parts(self.width, one, m & !(zero | one), 0)
    }

    #[inline]
    pub fn xor(self, o: Signal) -> Signal {
        let known = self.known_mask() & o.known_mask();
        Signal::from_parts(self.width, (self.val ^ o.val) & known, self.mask() & !known, 0)
    }

    /// Wire resolution of two drivers on the same net: `Z` yields to anything,
    /// equal values stay, different non-`Z` values give `X`.
    #[inline]
    pub fn resolve(self, o: Signal) -> Signal {
        let m = self.mask();
        let hiz = self.hiz & o.hiz;
        // Where exactly one side is Z, take the other side.
        let only_o = self.hiz & !o.hiz;
        let only_s = o.hiz & !self.hiz;
        let both = m & !(self.hiz | o.hiz);
        let conflict = both & (self.unk | o.unk | (self.val ^ o.val));
        let unk = (only_o & o.unk) | (only_s & self.unk) | conflict;
        let val = (only_o & o.val) | (only_s & self.val) | (both & self.val & !conflict);
        Signal::from_parts(self.width, val, unk, hiz)
    }

    // ---- formatting --------------------------------------------------------

    /// Binary digits, most significant first.
    pub fn to_bin_string(&self) -> String {
        (0..self.width).rev().map(|i| self.bit(i).to_char()).collect()
    }

    /// Hexadecimal digits; a nibble containing unknown bits prints as `x`
    /// (or `z` if it is entirely floating).
    pub fn to_hex_string(&self) -> String {
        let digits = self.width.div_ceil(4);
        let mut s = String::with_capacity(digits as usize);
        for d in (0..digits).rev() {
            let lo = d * 4;
            let nib = self.slice(lo, (self.width - lo).min(4));
            s.push(if nib.is_all_z() {
                'z'
            } else if !nib.is_known() {
                'x'
            } else {
                char::from_digit(nib.val as u32, 16).unwrap()
            });
        }
        s
    }

    /// Formats according to `radix`. Unknown values print as `x`/`z` in the
    /// decimal radixes.
    pub fn format(&self, radix: Radix) -> String {
        let unknown = || if self.is_all_z() { "z".to_string() } else { "x".to_string() };
        match radix {
            Radix::Bin => self.to_bin_string(),
            Radix::Hex => self.to_hex_string(),
            Radix::Dec => self.to_u64().map_or_else(unknown, |v| v.to_string()),
            Radix::Signed => self.to_i64().map_or_else(unknown, |v| v.to_string()),
        }
    }

    /// Parses `0`/`1`/`x`/`z` digits (binary), `0x..` hex or decimal.
    pub fn parse(width: u8, s: &str) -> Option<Signal> {
        let s = s.trim().replace('_', "");
        if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            return u64::from_str_radix(h, 16).ok().map(|v| Signal::new(width, v));
        }
        if let Some(b) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
            let mut sig = Signal::zero(width);
            let len = b.len();
            if len == 0 || len > 64 {
                return None;
            }
            for (i, c) in b.chars().rev().enumerate() {
                let bit = match c {
                    '0' => Bit::Zero,
                    '1' => Bit::One,
                    'x' | 'X' => Bit::X,
                    'z' | 'Z' => Bit::Z,
                    _ => return None,
                };
                sig = sig.with_bit(i as u8, bit);
            }
            return Some(sig);
        }
        if let Some(n) = s.strip_prefix('-') {
            return n.parse::<u64>().ok().map(|v| Signal::new(width, v.wrapping_neg()));
        }
        s.parse::<u64>().ok().map(|v| Signal::new(width, v))
    }
}

impl std::ops::Not for Signal {
    type Output = Signal;

    /// Bitwise NOT; unknown and floating bits give `X`.
    #[inline]
    fn not(self) -> Signal {
        let known = self.known_mask();
        Signal::from_parts(self.width, !self.val & known, self.mask() & !known, 0)
    }
}

impl fmt::Debug for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}'b{}", self.width, self.to_bin_string())
    }
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.width == 1 { write!(f, "{}", self.bit(0).to_char()) } else { write!(f, "0x{}", self.to_hex_string()) }
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Not;

    use super::*;

    const ALL: [Bit; 4] = [Bit::Zero, Bit::One, Bit::X, Bit::Z];

    fn b(bit: Bit) -> Signal {
        Signal::zero(1).with_bit(0, bit)
    }

    #[test]
    fn normalization() {
        let s = Signal::from_parts(4, 0xff, 0b0010, 0b0011);
        assert_eq!(s.hiz(), 0b0011);
        assert_eq!(s.unk(), 0);
        assert_eq!(s.val(), 0b1100);
        assert_eq!(s.to_bin_string(), "11zz");
        assert_eq!(Signal::new(64, u64::MAX).val(), u64::MAX);
    }

    #[test]
    fn and_truth_table() {
        use Bit::*;
        for a in ALL {
            for c in ALL {
                let r = b(a).and(b(c)).bit(0);
                let expect = match (a, c) {
                    (Zero, _) | (_, Zero) => Zero,
                    (One, One) => One,
                    _ => X,
                };
                assert_eq!(r, expect, "{a:?} & {c:?}");
            }
        }
    }

    #[test]
    fn or_truth_table() {
        use Bit::*;
        for a in ALL {
            for c in ALL {
                let r = b(a).or(b(c)).bit(0);
                let expect = match (a, c) {
                    (One, _) | (_, One) => One,
                    (Zero, Zero) => Zero,
                    _ => X,
                };
                assert_eq!(r, expect, "{a:?} | {c:?}");
            }
        }
    }

    #[test]
    fn xor_not_truth_table() {
        use Bit::*;
        for a in ALL {
            for c in ALL {
                let r = b(a).xor(b(c)).bit(0);
                let expect = match (a, c) {
                    (Zero, Zero) | (One, One) => Zero,
                    (Zero, One) | (One, Zero) => One,
                    _ => X,
                };
                assert_eq!(r, expect, "{a:?} ^ {c:?}");
            }
            let expect = match a {
                Zero => One,
                One => Zero,
                _ => X,
            };
            assert_eq!(b(a).not().bit(0), expect);
        }
    }

    #[test]
    fn resolution() {
        use Bit::*;
        for a in ALL {
            for c in ALL {
                let r = b(a).resolve(b(c)).bit(0);
                let expect = match (a, c) {
                    (Z, o) | (o, Z) => o,
                    (p, q) if p == q => p,
                    _ => X,
                };
                assert_eq!(r, expect, "{a:?} ~ {c:?}");
                assert_eq!(r, b(c).resolve(b(a)).bit(0), "commutative");
            }
        }
    }

    #[test]
    fn bus_ops() {
        let a = Signal::new(8, 0b1100_1010);
        let c = Signal::new(8, 0b1010_0110);
        assert_eq!(a.and(c), Signal::new(8, 0b1000_0010));
        assert_eq!(a.or(c), Signal::new(8, 0b1110_1110));
        assert_eq!(a.xor(c), Signal::new(8, 0b0110_1100));
        assert_eq!(a.not(), Signal::new(8, 0b0011_0101));
    }

    #[test]
    fn slicing_and_extension() {
        let a = Signal::new(8, 0xa5);
        assert_eq!(a.slice(4, 4), Signal::new(4, 0xa));
        assert_eq!(Signal::zero(8).insert(4, Signal::new(4, 0xc)), Signal::new(8, 0xc0));
        assert_eq!(Signal::new(4, 0b1000).sign_extend(8), Signal::new(8, 0xf8));
        assert_eq!(Signal::new(4, 0b0111).sign_extend(8), Signal::new(8, 0x07));
        assert_eq!(Signal::new(4, 0b1000).resize(8), Signal::new(8, 0x08));
        assert_eq!(Signal::new(8, 0xf0).to_i64(), Some(-16));
    }

    #[test]
    fn formatting() {
        let s = Signal::new(8, 0x3c);
        assert_eq!(s.format(Radix::Hex), "3c");
        assert_eq!(s.format(Radix::Dec), "60");
        assert_eq!(s.format(Radix::Bin), "00111100");
        assert_eq!(Signal::new(8, 0xff).format(Radix::Signed), "-1");
        let mixed = Signal::new(8, 0x30).with_bit(0, Bit::X);
        assert_eq!(mixed.format(Radix::Hex), "3x");
        assert_eq!(Signal::z(8).format(Radix::Hex), "zz");
        assert_eq!(Signal::z(8).format(Radix::Dec), "z");
        assert_eq!(Signal::x(1).to_string(), "x");
    }

    #[test]
    fn parsing() {
        assert_eq!(Signal::parse(8, "0x3c"), Some(Signal::new(8, 0x3c)));
        assert_eq!(Signal::parse(8, "60"), Some(Signal::new(8, 60)));
        assert_eq!(Signal::parse(8, "-1"), Some(Signal::new(8, 0xff)));
        assert_eq!(Signal::parse(4, "0b1x0z").map(|s| s.to_bin_string()), Some("1x0z".into()));
        assert_eq!(Signal::parse(4, "nope"), None);
    }
}
