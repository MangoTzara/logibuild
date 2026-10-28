//! Syntax highlighting of VHDL and Verilog in the source editor.

use std::ops::Range;

use iced::advanced::text::highlighter::{self, Highlighter};
use iced::{Font, Theme};

use lb_hdl::lang::{Class, Lang, Tokens};

use crate::theme;

/// Highlights one line at a time, remembering for each line whether it
/// starts inside a Verilog block comment.
pub struct Hdl {
    lang: Option<Lang>,
    line: usize,
    /// `in_block[i]`: line `i` starts inside a block comment.
    in_block: Vec<bool>,
}

impl Highlighter for Hdl {
    type Settings = Option<Lang>;
    type Highlight = Class;
    type Iterator<'a> = std::vec::IntoIter<(Range<usize>, Class)>;

    fn new(settings: &Self::Settings) -> Self {
        Hdl { lang: *settings, line: 0, in_block: vec![false] }
    }

    fn update(&mut self, new_settings: &Self::Settings) {
        *self = Hdl::new(new_settings);
    }

    fn change_line(&mut self, line: usize) {
        self.line = line.min(self.in_block.len() - 1);
        self.in_block.truncate(self.line + 1);
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        let mut block = self.in_block.get(self.line).copied().unwrap_or(false);
        let tokens: Vec<(Range<usize>, Class)> = match self.lang {
            Some(lang) => Tokens::new(line, lang, &mut block)
                .filter(|t| !matches!(t.class, Class::Word | Class::Other))
                .map(|t| (t.range, t.class))
                .collect(),
            None => Vec::new(),
        };
        self.in_block.truncate(self.line + 1);
        self.in_block.push(block);
        self.line += 1;
        tokens.into_iter()
    }

    fn current_line(&self) -> usize {
        self.line
    }
}

/// Colours of the token classes, from the active scheme.
pub fn format(class: &Class, _: &Theme) -> highlighter::Format<Font> {
    let c = theme::colors();
    let (color, font) = match class {
        Class::Keyword => (c.link, Some(Font { weight: iced::font::Weight::Bold, ..Font::MONOSPACE })),
        Class::Type => (c.positive, None),
        Class::Number => (c.neutral, None),
        Class::String => (c.negative, None),
        Class::Comment => (c.muted, None),
        Class::Word | Class::Other => (c.text, None),
    };
    highlighter::Format { color: Some(color), font }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_comments_carry_over_and_reset_on_change() {
        let mut h = Hdl::new(&Some(Lang::Verilog));
        let classes = |h: &mut Hdl, l: &str| h.highlight_line(l).map(|t| t.1).collect::<Vec<_>>();
        assert_eq!(classes(&mut h, "wire a; /* start"), [Class::Type, Class::Comment]);
        assert_eq!(classes(&mut h, "inside"), [Class::Comment]);
        assert_eq!(classes(&mut h, "end */ module"), [Class::Comment, Class::Keyword]);
        // The first line is edited: the comment no longer opens.
        h.change_line(0);
        assert_eq!(h.current_line(), 0);
        assert_eq!(classes(&mut h, "wire a;"), [Class::Type]);
        assert_eq!(classes(&mut h, "inside"), Vec::<Class>::new());
    }
}
