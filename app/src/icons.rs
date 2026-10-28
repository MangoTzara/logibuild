//! Symbolic 16×16 icons in the style of Breeze: thin outlines drawn in black
//! and tinted when rendered, so that they follow the state of their button.

use std::sync::LazyLock;

use iced::widget::svg::{self, Handle, Svg};
use iced::{Color, Theme};

use crate::theme;

macro_rules! icons {
    ($($name:ident => $body:literal,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        // Some icons are used by the optional HDL feature only.
        #[cfg_attr(not(feature = "hdl"), allow(dead_code))]
        pub enum Icon { $($name,)* }

        impl Icon {
            fn handle(self) -> Handle {
                $(
                    #[allow(non_upper_case_globals)]
                    static $name: LazyLock<Handle> = LazyLock::new(|| Handle::from_memory(wrap($body)));
                )*
                match self { $(Icon::$name => $name.clone(),)* }
            }
        }
    };
}

fn wrap(body: &str) -> Vec<u8> {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1" stroke-linejoin="round">{body}</svg>"##
    )
    .into_bytes()
}

icons! {
    New => r##"<path d="M3.5 1.5h6l3 3v10h-9z"/><path d="M9.5 1.5v3h3"/>"##,
    Open => r##"<path d="M1.5 13.5v-11h4l1.5 1.5h7.5v10z"/><path d="M1.5 6.5h13"/>"##,
    Save => r##"<path d="M2.5 2.5h9l2 2v9h-11z"/><path d="M5.5 2.5v3h4v-3"/><path d="M4.5 13.5v-4h7v4"/>"##,
    Undo => r##"<path d="M5.5 3 2.5 6l3 3" stroke-linecap="round"/><path d="M2.5 6h7a3.5 3.5 0 0 1 0 7h-3" stroke-linecap="round"/>"##,
    Redo => r##"<path d="m10.5 3 3 3-3 3" stroke-linecap="round"/><path d="M13.5 6h-7a3.5 3.5 0 0 0 0 7h3" stroke-linecap="round"/>"##,
    Rotate => r##"<path d="M13 8.5A5 5 0 1 1 8 3.5h3" stroke-linecap="round"/><path d="m9 1.5 2 2-2 2" stroke-linecap="round"/>"##,
    Flip => r##"<path d="M8 1v14" stroke-dasharray="1.5 1.5"/><path d="M6 4v8.5H1.5z"/><path d="M10 4v8.5h4.5z" fill="#000"/>"##,
    Delete => r##"<path d="M2 4.5h12"/><path d="M6 4.5v-2h4v2"/><path d="m3.5 4.5 1 10h7l1-10"/><path d="M6.5 7v5M9.5 7v5"/>"##,
    Run => r##"<path d="M4.5 2.5v11l9-5.5z" fill="#000"/>"##,
    Pause => r##"<path d="M4 2.5h2.5v11H4zM9.5 2.5H12v11H9.5z" fill="#000" stroke="none"/>"##,
    Reset => r##"<path d="M3 3h1.5v10H3z" fill="#000" stroke="none"/><path d="M13 3.5v9L6 8z" fill="#000"/>"##,
    Step => r##"<path d="M3 3.5v9L10 8z" fill="#000"/><path d="M11.5 3H13v10h-1.5z" fill="#000" stroke="none"/>"##,
    Fit => r##"<path d="M1.5 5.5v-4h4M10.5 1.5h4v4M14.5 10.5v4h-4M5.5 14.5h-4v-4"/><path d="M5.5 5.5h5v5h-5z"/>"##,
    Wave => r##"<path d="M1 11.5h3.5v-7h4v7h3v-7H15"/>"##,
    Add => r##"<path d="M8 3v10M3 8h10"/>"##,
    Close => r##"<path d="m4 4 8 8M12 4l-8 8"/>"##,
    Check => r##"<path d="m3 8.5 3 3 7-7" stroke-width="1.5"/>"##,
    ChevronDown => r##"<path d="m4 6 4 4 4-4"/>"##,
    ChevronRight => r##"<path d="m6 4 4 4-4 4"/>"##,
    ChevronLeft => r##"<path d="M10 4 6 8l4 4"/>"##,
    Export => r##"<path d="M8 10.5v-9M5 4.5l3-3 3 3"/><path d="M2.5 9.5v5h11v-5"/>"##,
    Search => r##"<circle cx="6.5" cy="6.5" r="4"/><path d="m9.5 9.5 5 5"/>"##,
    Compile => r##"<circle cx="8" cy="8" r="6.5"/><path d="m5 8.2 2 2 4-4.2" stroke-linecap="round"/>"##,
    Chip => r##"<path d="M4.5 4.5h7v7h-7z"/><path d="M6.5 1.5v3M9.5 1.5v3M6.5 11.5v3M9.5 11.5v3M1.5 6.5h3M1.5 9.5h3M11.5 6.5h3M11.5 9.5h3"/>"##,
    Help => r##"<circle cx="8" cy="8" r="6.5"/><path d="M6.1 6.3a2 2 0 1 1 2.6 1.9c-.45.2-.7.55-.7 1v.5" stroke-linecap="round"/><circle cx="8" cy="11.6" r=".7" fill="#000" stroke="none"/>"##,
    Stop => r##"<path d="M3.5 3.5h9v9h-9z" fill="#000"/>"##,
    CheckOff => r##"<rect x="2.5" y="2.5" width="11" height="11" rx="2"/>"##,
    CheckOn => r##"<rect x="2.5" y="2.5" width="11" height="11" rx="2"/><path d="m5 8.2 2 2 4-4.2" stroke-width="1.5" stroke-linecap="round"/>"##,
    RadioOff => r##"<circle cx="8" cy="8" r="5.5"/>"##,
    RadioOn => r##"<circle cx="8" cy="8" r="5.5"/><circle cx="8" cy="8" r="2.5" fill="#000" stroke="none"/>"##,
}

/// An icon tinted with `color`, `size` pixels square.
pub fn icon<'a>(i: Icon, size: f32, color: Color) -> Svg<'a, Theme> {
    svg(i.handle(), size).style(move |_, _| svg::Style { color: Some(color) })
}

fn svg<'a>(handle: Handle, size: f32) -> Svg<'a, Theme> {
    Svg::new(handle).width(size).height(size)
}

/// The coloured emblems of message boxes and banners (not tinted).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emblem {
    Error,
    Warning,
    Info,
}

pub fn emblem<'a>(e: Emblem, size: f32) -> Svg<'a, Theme> {
    static ERROR: LazyLock<Handle> = LazyLock::new(|| {
        Handle::from_memory(format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="7" fill="{}"/><path d="m5.5 5.5 5 5m0-5-5 5" stroke="#fff" stroke-width="1.5" stroke-linecap="round"/></svg>"##,
            hex(theme::LIGHT.negative)
        )
        .into_bytes())
    });
    static WARNING: LazyLock<Handle> = LazyLock::new(|| {
        Handle::from_memory(format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M8 1.2 15.2 14.3H.8z" fill="{}" stroke="{0}" stroke-linejoin="round"/><path d="M8 5.8v4" stroke="#fff" stroke-width="1.5" stroke-linecap="round"/><circle cx="8" cy="12" r=".9" fill="#fff"/></svg>"##,
            hex(theme::LIGHT.neutral)
        )
        .into_bytes())
    });
    static INFO: LazyLock<Handle> = LazyLock::new(|| {
        Handle::from_memory(format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="7" fill="{}"/><path d="M8 7v4.5" stroke="#fff" stroke-width="1.5" stroke-linecap="round"/><circle cx="8" cy="4.6" r=".9" fill="#fff"/></svg>"##,
            hex(theme::LIGHT.link)
        )
        .into_bytes())
    });
    svg(
        match e {
            Emblem::Error => ERROR.clone(),
            Emblem::Warning => WARNING.clone(),
            Emblem::Info => INFO.clone(),
        },
        size,
    )
}

fn hex(c: Color) -> String {
    let [r, g, b, _] = c.into_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}
