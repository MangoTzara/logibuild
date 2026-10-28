//! A single line of text cut with an ellipsis to the width it is given, as
//! the file names of VS Code: in a row, it takes what the other items leave
//! (it fills) but is only as wide as its text, so what follows stays beside
//! it.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::text::{self, Paragraph as _};
use iced::advanced::widget::{self, Tree, Widget};
use iced::alignment::Vertical;
use iced::mouse;
use iced::{Color, Element, Font, Length, Pixels, Rectangle, Size};

pub struct Elide {
    content: String,
    size: f32,
    font: Option<Font>,
    color: Option<Color>,
}

pub fn elide(content: impl Into<String>) -> Elide {
    Elide { content: content.into(), size: 13.0, font: None, color: None }
}

impl Elide {
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn font(mut self, font: Font) -> Self {
        self.font = Some(font);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
}

/// The paragraph shown, and what it was made for.
#[derive(Default)]
struct State<P> {
    paragraph: P,
    key: Option<(String, f32, f32, Option<Font>)>,
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Elide
where
    Renderer: text::Renderer<Font = Font>,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State<Renderer::Paragraph>>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::<Renderer::Paragraph>::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let state = tree.state.downcast_mut::<State<Renderer::Paragraph>>();
        let max = limits.max().width;
        let key = (self.content.clone(), max, self.size, self.font);
        if state.key.as_ref() != Some(&key) {
            let font = self.font.unwrap_or_else(|| renderer.default_font());
            let para = |s: &str| {
                Renderer::Paragraph::with_text(text::Text {
                    content: s,
                    bounds: Size::INFINITE,
                    size: Pixels(self.size),
                    line_height: text::LineHeight::default(),
                    font,
                    align_x: text::Alignment::Default,
                    align_y: Vertical::Top,
                    shaping: text::Shaping::Advanced,
                    wrapping: text::Wrapping::None,
                })
            };
            let mut paragraph = para(&self.content);
            if paragraph.min_width() > max {
                // The longest start that fits with the ellipsis.
                let cuts: Vec<usize> = self.content.char_indices().map(|(i, _)| i).collect();
                let cut = |n: usize| format!("{}…", self.content[..cuts[n]].trim_end());
                let (mut lo, mut hi) = (0, cuts.len() - 1);
                while lo < hi {
                    let mid = (lo + hi).div_ceil(2);
                    if para(&cut(mid)).min_width() <= max { lo = mid } else { hi = mid - 1 }
                }
                paragraph = para(&cut(lo));
            }
            state.paragraph = paragraph;
            state.key = Some(key);
        }
        let size = state.paragraph.min_bounds();
        layout::Node::new(Size::new(size.width.min(max), size.height))
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State<Renderer::Paragraph>>();
        let bounds = layout.bounds();
        renderer.fill_paragraph(
            &state.paragraph,
            bounds.position(),
            self.color.unwrap_or(style.text_color),
            bounds.intersection(viewport).unwrap_or(bounds),
        );
    }
}

impl<'a, Message, Theme, Renderer> From<Elide> for Element<'a, Message, Theme, Renderer>
where
    Renderer: text::Renderer<Font = Font> + 'a,
{
    fn from(e: Elide) -> Self {
        Element::new(e)
    }
}
