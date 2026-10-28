# Patched iced crates

Copies of two crates of [iced](https://github.com/iced-rs/iced) 0.14 (MIT, by
the iced authors), used instead of the crates.io versions through
`[patch.crates-io]` in the workspace `Cargo.toml`. Every change is marked
with a `Patched` comment.

- `iced_graphics` 0.14.0, `src/text/editor.rs`:
  - the fix of [iced-rs/iced#3491](https://github.com/iced-rs/iced/pull/3491):
    the text editor no longer panics with "Line layout should be cached" when
    the window is resized while the cursor is outside the visible lines
    ([#3483](https://github.com/iced-rs/iced/issues/3483));
  - tabs and indentation four columns wide instead of eight.
- `iced_widget` 0.14.2, `src/text_editor.rs`: `Content::scroll`, the scroll
  position of the editor, for the scrollbar of the HDL source editor.

When moving to a newer iced, drop the copies and the patch section, then
check the release for the fix (in 0.15) and for a way to read the scroll
position and set the tab width.
