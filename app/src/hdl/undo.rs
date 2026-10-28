//! Undo and redo in the source editor, which the text editor of iced lacks.
//!
//! Each step keeps the whole text and the cursor before it; HDL sources are
//! small. Typing a word, or a run of deletions, is one step. Undoing replaces
//! only the part of the text that differs, so that the editor keeps its
//! scroll position and its highlighting of the other lines.

use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::widget::text_editor::{Action, Content, Cursor, Edit, Position};

/// Steps kept for each file.
const LIMIT: usize = 500;

/// Edits closer than this belong to the same step.
const PAUSE: Duration = Duration::from_secs(1);

#[derive(Debug, Default)]
pub struct Undo {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The kind of the last edit and when it was made, while it can grow.
    last: Option<(Kind, Instant)>,
}

#[derive(Debug)]
struct Snapshot {
    text: String,
    cursor: Cursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Characters typed; `true` for blanks.
    Type(bool),
    Delete,
}

impl Undo {
    /// Takes note of an action about to be performed on `content`.
    pub fn record(&mut self, content: &Content, action: &Action) {
        let kind = match action {
            Action::Edit(Edit::Insert(c)) => Some(Kind::Type(c.is_whitespace())),
            Action::Edit(Edit::Backspace | Edit::Delete) => Some(Kind::Delete),
            Action::Edit(_) => None,
            // Scrolling leaves the step open; moving the cursor ends it.
            Action::Scroll { .. } => return,
            _ => {
                self.last = None;
                return;
            }
        };
        let now = Instant::now();
        // A step goes on with edits of its kind, until a pause; typing a
        // blank after a word starts a new one.
        let goes_on = match (self.last, kind) {
            (Some((last, at)), Some(kind)) if now - at < PAUSE => {
                last == kind || (last == Kind::Type(true) && kind == Kind::Type(false))
            }
            _ => false,
        };
        if !goes_on {
            self.checkpoint(content);
        }
        self.last = kind.map(|k| (k, now));
    }

    /// Starts a new step before an edit of its own.
    pub fn checkpoint(&mut self, content: &Content) {
        if self.undo.len() == LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(Snapshot::of(content));
        self.redo.clear();
        self.last = None;
    }

    /// Goes back one step; false when there is none.
    pub fn undo(&mut self, content: &mut Content) -> bool {
        step(&mut self.undo, &mut self.redo, content) && {
            self.last = None;
            true
        }
    }

    /// Goes forward one step; false when there is none.
    pub fn redo(&mut self, content: &mut Content) -> bool {
        step(&mut self.redo, &mut self.undo, content) && {
            self.last = None;
            true
        }
    }
}

/// Moves `content` to the last state of `from`, keeping the current one in `to`.
fn step(from: &mut Vec<Snapshot>, to: &mut Vec<Snapshot>, content: &mut Content) -> bool {
    let Some(snapshot) = from.pop() else { return false };
    to.push(Snapshot::of(content));
    snapshot.restore(content);
    true
}

impl Snapshot {
    fn of(content: &Content) -> Self {
        Snapshot { text: text(content), cursor: content.cursor() }
    }

    fn restore(&self, content: &mut Content) {
        let now = text(content);
        let (a, b) = (now.as_bytes(), self.text.as_bytes());
        let mut start = a.iter().zip(b).take_while(|(x, y)| x == y).count();
        while !now.is_char_boundary(start) {
            start -= 1;
        }
        let max = a.len().min(b.len()) - start;
        let mut end = a.iter().rev().zip(b.iter().rev()).take(max).take_while(|(x, y)| x == y).count();
        while !now.is_char_boundary(a.len() - end) || !self.text.is_char_boundary(b.len() - end) {
            end -= 1;
        }
        let (from, to) = (position(&now, start), position(&now, a.len() - end));
        let new = &self.text[start..b.len() - end];
        if from != to || !new.is_empty() {
            content.move_to(Cursor { position: to, selection: (from != to).then_some(from) });
            let edit = if new.is_empty() { Edit::Delete } else { Edit::Paste(Arc::new(new.to_string())) };
            content.perform(Action::Edit(edit));
        }
        content.move_to(self.cursor);
    }
}

/// The text with every line ending as `\n`, so that byte offsets map to
/// positions in the editor.
fn text(content: &Content) -> String {
    content.lines().map(|l| l.text.into_owned()).collect::<Vec<_>>().join("\n")
}

fn position(text: &str, offset: usize) -> Position {
    let before = &text[..offset];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    Position { line: before.matches('\n').count(), column: offset - line_start }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(undo: &mut Undo, content: &mut Content, action: Action) {
        undo.record(content, &action);
        content.perform(action);
    }

    fn typed(undo: &mut Undo, content: &mut Content, s: &str) {
        for c in s.chars() {
            edit(undo, content, Action::Edit(Edit::Insert(c)));
        }
    }

    #[test]
    fn words_are_undone_one_at_a_time_and_redone() {
        let mut content = Content::with_text("entity è is\nend;");
        let mut undo = Undo::default();
        content.move_to(Cursor { position: Position { line: 0, column: 9 }, selection: None });
        typed(&mut undo, &mut content, " foo bar");
        assert_eq!(content.text(), "entity è foo bar is\nend;");
        assert!(undo.undo(&mut content));
        assert_eq!(content.text(), "entity è foo is\nend;");
        assert!(undo.undo(&mut content));
        assert_eq!(content.text(), "entity è is\nend;");
        assert_eq!(content.cursor().position, Position { line: 0, column: 9 });
        assert!(!undo.undo(&mut content));
        assert!(undo.redo(&mut content) && undo.redo(&mut content));
        assert_eq!(content.text(), "entity è foo bar is\nend;");
        assert!(!undo.redo(&mut content));
    }

    #[test]
    fn deletions_across_lines_and_moves_make_separate_steps() {
        let mut content = Content::with_text("a\nbc\nd");
        let mut undo = Undo::default();
        content.move_to(Cursor { position: Position { line: 2, column: 0 }, selection: None });
        edit(&mut undo, &mut content, Action::Edit(Edit::Backspace));
        edit(&mut undo, &mut content, Action::Edit(Edit::Backspace));
        assert_eq!(content.text(), "a\nbd");
        edit(&mut undo, &mut content, Action::Move(iced::widget::text_editor::Motion::DocumentStart));
        edit(&mut undo, &mut content, Action::Edit(Edit::Delete));
        assert_eq!(content.text(), "\nbd");
        assert!(undo.undo(&mut content));
        assert_eq!(content.text(), "a\nbd");
        assert!(undo.undo(&mut content));
        assert_eq!(content.text(), "a\nbc\nd");
        // A new edit drops what could be redone.
        edit(&mut undo, &mut content, Action::Edit(Edit::Paste(Arc::new("x\ny".into()))));
        assert!(!undo.redo(&mut content));
        assert!(undo.undo(&mut content));
        assert_eq!(content.text(), "a\nbc\nd");
    }
}
