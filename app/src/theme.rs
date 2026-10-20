//! Breeze look: the colours of the KDE Plasma colour schemes (light and
//! dark), flat panels separated by hairlines, Breeze-style buttons and
//! selections, the colours used for signal values, and the text size.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use iced::overlay::menu;
use iced::widget::{
    Space, button, checkbox, container, pick_list, row, scrollable, text, text_editor, text_input, tooltip,
};
use iced::{Alignment, Background, Border, Color, Element, Fill, Font, Shadow, Theme, Vector, color, font, theme};

use lb_core::netlist::Net;
use lb_core::value::Signal;

use crate::icons::{self, Emblem};

/// Embedded fonts (no dependency on system fonts): Noto Sans, the Plasma
/// default.
pub const FONT: Font = Font::with_name("Noto Sans");
pub const FONT_MEDIUM: Font = Font { weight: font::Weight::Medium, ..FONT };
pub const FONT_DATA: [&[u8]; 2] =
    [include_bytes!("../assets/fonts/NotoSans-Regular.ttf"), include_bytes!("../assets/fonts/NotoSans-Medium.ttf")];

/// Radius of buttons, fields, menus (Breeze frame radius).
pub const RADIUS: f32 = 5.0;
/// Radius of list items and small chips.
pub const ITEM_RADIUS: f32 = 3.0;

// ---- colour schemes ------------------------------------------------------------

/// The colours of a scheme.
pub struct Colors {
    /// Panels, docks, status bar.
    pub window: Color,
    /// Menu bar and toolbar; matches the Breeze title bar.
    pub header: Color,
    /// Content views: canvas, waveform, text fields.
    pub view: Color,
    pub view_alt: Color,
    pub button: Color,
    pub text: Color,
    pub muted: Color,
    pub disabled: Color,
    pub accent: Color,
    /// Accent used for text, adjusted for contrast.
    pub link: Color,
    pub negative: Color,
    pub neutral: Color,
    pub positive: Color,
    /// Hairlines between panels.
    pub separator: Color,
    /// Outline of buttons, fields and frames.
    pub frame: Color,
    /// Opaque accent tint, for fills drawn on the canvas.
    pub accent_soft: Color,
    /// Neutral fill drawn on the canvas (button components, chips).
    pub hover: Color,
    /// Strength of drop shadows.
    pub shadow: f32,

    pub canvas_bg: Color,
    pub grid: Color,
    pub stroke: Color,
    pub body: Color,
    pub annotation: Color,
    /// Components drawn as blocks at far zoom.
    pub lod_body: Color,
    /// Time grid of the waveform.
    pub wave_grid: Color,

    pub wire_low: Color,
    pub wire_high: Color,
    pub wire_x: Color,
    pub wire_z: Color,
    pub wire_bus: Color,
    pub wire_error: Color,
    pub wire_idle: Color,
    pub oscillation: Color,
    /// A 0 shown by LEDs and 1-bit pins.
    pub bit_low: Color,
}

/// Light scheme (Breeze Light colours).
pub static LIGHT: Colors = Colors {
    window: color!(0xeff0f1),
    header: color!(0xdee0e2),
    view: Color::WHITE,
    view_alt: color!(0xf7f7f7),
    button: color!(0xfcfcfc),
    text: color!(0x232629),
    muted: color!(0x707d8a),
    disabled: color!(0xa3a8ad),
    accent: color!(0x3daee9),
    link: color!(0x2980b9),
    negative: color!(0xda4453),
    neutral: color!(0xf67400),
    positive: color!(0x27ae60),
    separator: color!(0xc8cacc),
    frame: color!(0xbcbfc2),
    accent_soft: color!(0xd8eefa),
    hover: color!(0xeceef0),
    shadow: 1.0,

    canvas_bg: Color::WHITE,
    grid: color!(0xcbcfd3),
    stroke: color!(0x31363b),
    body: Color::WHITE,
    annotation: color!(0x8a949e),
    lod_body: color!(0x9ba3ab),
    wave_grid: color!(0xedeef0),

    wire_low: color!(0x1f7a35),
    wire_high: color!(0x2fc25b),
    wire_x: color!(0xda4453),
    wire_z: color!(0x98a2ab),
    wire_bus: color!(0x1d4f7c),
    wire_error: color!(0xf67400),
    wire_idle: color!(0x5d6670),
    oscillation: color!(0xff6a00),
    bit_low: color!(0x1d5f2c),
};

/// Dark scheme (Breeze Dark colours).
pub static DARK: Colors = Colors {
    window: color!(0x202326),
    header: color!(0x292c30),
    view: color!(0x141618),
    view_alt: color!(0x1d1f22),
    button: color!(0x292c30),
    text: color!(0xfcfcfc),
    muted: color!(0xa1a9b1),
    disabled: color!(0x656b71),
    accent: color!(0x3daee9),
    link: color!(0x1d99f3),
    negative: color!(0xda4453),
    neutral: color!(0xf67400),
    positive: color!(0x27ae60),
    separator: color!(0x3a3e43),
    frame: color!(0x4d5258),
    accent_soft: color!(0x1e5774),
    hover: color!(0x31363b),
    shadow: 2.2,

    canvas_bg: color!(0x17191c),
    grid: color!(0x3d4247),
    stroke: color!(0xc8ccd0),
    body: color!(0x232629),
    annotation: color!(0x7f8a95),
    lod_body: color!(0x5a626a),
    wave_grid: color!(0x25282c),

    wire_low: color!(0x2f7d3f),
    wire_high: color!(0x41dd70),
    wire_x: color!(0xe9535f),
    wire_z: color!(0x6f7882),
    wire_bus: color!(0x5a8dd6),
    wire_error: color!(0xf67400),
    wire_idle: color!(0x8a939c),
    oscillation: color!(0xff7a1a),
    bit_low: color!(0x245a30),
};

static DARK_MODE: AtomicBool = AtomicBool::new(false);
/// Text size in percent of the default.
static TEXT_SCALE: AtomicU32 = AtomicU32::new(100);

/// The colours of the active scheme.
pub fn colors() -> &'static Colors {
    if is_dark() { &DARK } else { &LIGHT }
}

pub fn is_dark() -> bool {
    DARK_MODE.load(Ordering::Relaxed)
}

pub fn set_dark(dark: bool) {
    DARK_MODE.store(dark, Ordering::Relaxed);
}

pub fn set_text_scale(percent: u32) {
    TEXT_SCALE.store(percent, Ordering::Relaxed);
}

/// `px`, the size of a text at the default size, scaled to the chosen text
/// size. Also used for the elements whose size follows the text (bars,
/// label columns, menus).
pub fn fs(px: f32) -> f32 {
    (px * TEXT_SCALE.load(Ordering::Relaxed) as f32 / 100.0).round()
}

fn iced_theme(name: &'static str, c: &Colors) -> Theme {
    Theme::custom(
        name,
        theme::Palette {
            background: c.window,
            text: c.text,
            primary: c.accent,
            success: c.positive,
            warning: c.neutral,
            danger: c.negative,
        },
    )
}

pub fn theme() -> Theme {
    static LIGHT_THEME: LazyLock<Theme> = LazyLock::new(|| iced_theme("Light", &LIGHT));
    static DARK_THEME: LazyLock<Theme> = LazyLock::new(|| iced_theme("Dark", &DARK));
    if is_dark() { DARK_THEME.clone() } else { LIGHT_THEME.clone() }
}

const fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

/// A shadow of opacity `a` in the light scheme, stronger in the dark one.
fn shade(a: f32) -> Color {
    alpha(Color::BLACK, (a * colors().shadow).min(1.0))
}

/// Colour of a wire carrying `value` on `net`.
pub fn wire_color(value: Signal, net: &Net) -> Color {
    let c = colors();
    if net.width_error {
        return c.wire_error;
    }
    if value.has_x() {
        return c.wire_x;
    }
    if value.is_all_z() {
        return c.wire_z;
    }
    if net.width > 1 {
        return c.wire_bus;
    }
    if value.is_one() { c.wire_high } else { c.wire_low }
}

/// Fill for value indicators (LEDs, 1-bit pins).
pub fn bit_fill(value: Signal) -> Color {
    let c = colors();
    if value.has_x() {
        c.wire_x
    } else if value.has_z() {
        c.wire_z
    } else if value.is_one() {
        c.wire_high
    } else {
        c.bit_low
    }
}

// ---- surfaces ------------------------------------------------------------------

fn fill(bg: Color) -> container::Style {
    container::Style { background: Some(Background::Color(bg)), ..Default::default() }
}

/// Docked panels and the status bar.
pub fn window(_: &Theme) -> container::Style {
    fill(colors().window)
}

/// Menu bar and toolbar.
pub fn header(_: &Theme) -> container::Style {
    fill(colors().header)
}

pub fn view(_: &Theme) -> container::Style {
    fill(colors().view)
}

/// A one pixel line across the parent.
pub fn hline<'a, M: 'a>() -> Element<'a, M> {
    container(Space::new()).width(Fill).height(1).style(|_| fill(colors().separator)).into()
}

/// A one pixel line along the parent.
pub fn vline<'a, M: 'a>() -> Element<'a, M> {
    container(Space::new()).width(1).height(Fill).style(|_| fill(colors().separator)).into()
}

/// Short vertical separator between toolbar groups.
pub fn tool_separator<'a, M: 'a>() -> Element<'a, M> {
    container(container(Space::new()).width(1).height(18).style(|_| fill(colors().separator))).padding([0, 4]).into()
}

fn popup_shadow() -> Shadow {
    Shadow { color: shade(0.18), offset: Vector::new(0.0, 3.0), blur_radius: 12.0 }
}

/// Popup menus.
pub fn menu(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(colors().button)),
        border: Border { color: colors().frame, width: 1.0, radius: RADIUS.into() },
        shadow: popup_shadow(),
        ..Default::default()
    }
}

/// Dialogs.
pub fn dialog(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(colors().window)),
        border: Border { color: colors().frame, width: 1.0, radius: (RADIUS + 1.0).into() },
        shadow: Shadow { color: shade(0.25), offset: Vector::new(0.0, 6.0), blur_radius: 24.0 },
        ..Default::default()
    }
}

pub fn tooltip_box(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(colors().view_alt)),
        text_color: Some(colors().text),
        border: Border { color: colors().frame, width: 1.0, radius: RADIUS.into() },
        shadow: Shadow { color: shade(0.12), offset: Vector::new(0.0, 2.0), blur_radius: 6.0 },
        ..Default::default()
    }
}

/// `content` with a Breeze tooltip below it.
pub fn tip<'a, M: 'a>(content: impl Into<Element<'a, M>>, label: impl ToString) -> Element<'a, M> {
    tooltip(
        content,
        container(text(label.to_string()).size(fs(12.0))).padding([4, 8]).style(tooltip_box),
        tooltip::Position::Bottom,
    )
    .gap(4)
    .into()
}

/// Inline message (KMessageWidget): tinted background, coloured outline
/// and emblem.
pub fn message<'a, M: 'a>(kind: Emblem, content: impl Into<Element<'a, M>>) -> container::Container<'a, M> {
    let c = if kind == Emblem::Error { colors().negative } else { colors().neutral };
    container(row![icons::emblem(kind, 16.0), content.into()].spacing(8).align_y(Alignment::Center))
        .padding([6, 10])
        .width(Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(alpha(c, 0.12))),
            text_color: Some(colors().text),
            border: Border { color: alpha(c, 0.7), width: 1.0, radius: RADIUS.into() },
            ..Default::default()
        })
}

/// Small coloured label (pin direction).
pub fn chip(c: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(alpha(c, 0.14))),
        border: Border { radius: ITEM_RADIUS.into(), ..Default::default() },
        ..Default::default()
    }
}

// ---- buttons --------------------------------------------------------------------

fn style(bg: Option<Color>, fg: Color, border: Color, radius: f32) -> button::Style {
    button::Style {
        background: bg.map(Background::Color),
        text_color: fg,
        border: Border { color: border, width: if border.a > 0.0 { 1.0 } else { 0.0 }, radius: radius.into() },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Push button: light face, grey outline that turns accent on hover.
pub fn push(_: &Theme, status: button::Status) -> button::Style {
    let mut s = match status {
        button::Status::Active => style(Some(colors().button), colors().text, colors().frame, RADIUS),
        button::Status::Hovered => style(Some(colors().button), colors().text, colors().accent, RADIUS),
        button::Status::Pressed => style(Some(alpha(colors().accent, 0.3)), colors().text, colors().accent, RADIUS),
        button::Status::Disabled => {
            style(Some(alpha(colors().button, 0.6)), colors().disabled, alpha(colors().frame, 0.6), RADIUS)
        }
    };
    if status != button::Status::Disabled && status != button::Status::Pressed {
        s.shadow = Shadow { color: shade(0.07), offset: Vector::new(0.0, 1.0), blur_radius: 1.0 };
    }
    s
}

/// The default button of a dialog: accent tinted.
pub fn default_push(t: &Theme, status: button::Status) -> button::Style {
    let mut s = push(t, status);
    s.border.color = colors().accent;
    s.background = Some(Background::Color(match status {
        button::Status::Hovered => alpha(colors().accent, 0.28),
        button::Status::Pressed => alpha(colors().accent, 0.4),
        _ => alpha(colors().accent, 0.2),
    }));
    s
}

/// Flat tool button: no frame until hovered.
pub fn tool(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Active => style(None, colors().text, Color::TRANSPARENT, RADIUS),
        button::Status::Hovered => {
            style(Some(alpha(colors().accent, 0.14)), colors().text, alpha(colors().accent, 0.55), RADIUS)
        }
        button::Status::Pressed => style(Some(alpha(colors().accent, 0.3)), colors().text, colors().accent, RADIUS),
        button::Status::Disabled => style(None, colors().disabled, Color::TRANSPARENT, RADIUS),
    }
}

/// Checkable tool button, tinted while checked.
pub fn toggle(checked: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |t, status| match (checked, status) {
        (true, button::Status::Hovered) => {
            style(Some(alpha(colors().accent, 0.32)), colors().text, colors().accent, RADIUS)
        }
        (true, button::Status::Active | button::Status::Pressed) => {
            style(Some(alpha(colors().accent, 0.24)), colors().text, alpha(colors().accent, 0.8), RADIUS)
        }
        _ => tool(t, status),
    }
}

/// An entry of a list or menu: highlighted on hover, tinted with an accent
/// outline when selected.
pub fn item(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let mut s = match (selected, status) {
            (_, button::Status::Disabled) => style(None, colors().disabled, Color::TRANSPARENT, ITEM_RADIUS),
            (true, button::Status::Hovered) => {
                style(Some(alpha(colors().accent, 0.32)), colors().text, colors().accent, ITEM_RADIUS)
            }
            (true, _) => {
                style(Some(alpha(colors().accent, 0.24)), colors().text, alpha(colors().accent, 0.8), ITEM_RADIUS)
            }
            (false, button::Status::Hovered) => {
                style(Some(alpha(colors().accent, 0.12)), colors().text, alpha(colors().accent, 0.4), ITEM_RADIUS)
            }
            (false, button::Status::Pressed) => {
                style(Some(alpha(colors().accent, 0.24)), colors().text, colors().accent, ITEM_RADIUS)
            }
            (false, button::Status::Active) => style(None, colors().text, Color::TRANSPARENT, ITEM_RADIUS),
        };
        s.snap = true;
        s
    }
}

/// Accent text, for an action inside a list.
pub fn link(t: &Theme, status: button::Status) -> button::Style {
    let mut s = tool(t, status);
    if status != button::Status::Disabled {
        s.text_color = colors().link;
    }
    s
}

/// The entry currently shown: accent text, no background.
pub fn current(_: &Theme, _: button::Status) -> button::Style {
    style(None, colors().link, Color::TRANSPARENT, ITEM_RADIUS)
}

/// Where a button sits in a segmented control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
    First,
    Middle,
    Last,
}

/// Frame around the buttons of a segmented control.
pub fn segmented(_: &Theme) -> container::Style {
    let c = colors();
    container::Style {
        background: Some(Background::Color(c.button)),
        border: Border { color: c.frame, width: 1.0, radius: RADIUS.into() },
        shadow: Shadow { color: shade(0.07), offset: Vector::new(0.0, 1.0), blur_radius: 1.0 },
        ..Default::default()
    }
}

/// A button of a segmented control: no outline of its own, tinted on hover,
/// rounded only on the outer corners of the group.
pub fn segment(at: Segment) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |t, status| {
        let r = RADIUS - 1.0;
        let mut s = tool(t, status);
        s.border = Border {
            radius: match at {
                Segment::First => iced::border::left(r),
                Segment::Middle => 0.0.into(),
                Segment::Last => iced::border::right(r),
            },
            ..Default::default()
        };
        s
    }
}

/// Content of a button: an icon, then a label when `label` is not empty.
pub fn icon_label<'a, M: 'a>(i: icons::Icon, label: &str, enabled: bool) -> Element<'a, M> {
    let color = if enabled { colors().text } else { colors().disabled };
    let mut r = row![icons::icon(i, 16.0, color)].spacing(6).align_y(Alignment::Center);
    if !label.is_empty() {
        r = r.push(text(label.to_string()).size(fs(13.0)));
    }
    r.into()
}

// ---- fields ---------------------------------------------------------------------

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let border = match status {
        text_input::Status::Focused { .. } => colors().accent,
        text_input::Status::Hovered => alpha(colors().accent, 0.6),
        text_input::Status::Active => colors().frame,
        text_input::Status::Disabled => alpha(colors().frame, 0.6),
    };
    text_input::Style {
        background: Background::Color(if matches!(status, text_input::Status::Disabled) {
            colors().window
        } else {
            colors().view
        }),
        border: Border { color: border, width: 1.0, radius: RADIUS.into() },
        icon: colors().muted,
        placeholder: colors().muted,
        value: colors().text,
        selection: alpha(colors().accent, 0.35),
    }
}

/// Multi-line text field, styled like [`input`].
pub fn text_area(theme: &Theme, status: text_editor::Status) -> text_editor::Style {
    let s = input(
        theme,
        match status {
            text_editor::Status::Active => text_input::Status::Active,
            text_editor::Status::Hovered => text_input::Status::Hovered,
            text_editor::Status::Focused { is_hovered } => text_input::Status::Focused { is_hovered },
            text_editor::Status::Disabled => text_input::Status::Disabled,
        },
    );
    text_editor::Style {
        background: s.background,
        border: s.border,
        placeholder: s.placeholder,
        value: s.value,
        selection: s.selection,
    }
}

/// Combo box, styled like a push button.
pub fn dropdown(_: &Theme, status: pick_list::Status) -> pick_list::Style {
    let border = match status {
        pick_list::Status::Active => colors().frame,
        pick_list::Status::Hovered | pick_list::Status::Opened { .. } => colors().accent,
    };
    pick_list::Style {
        text_color: colors().text,
        placeholder_color: colors().muted,
        handle_color: colors().text,
        background: Background::Color(colors().button),
        border: Border { color: border, width: 1.0, radius: RADIUS.into() },
    }
}

/// Combo box list, styled like the application menus.
pub fn dropdown_menu(_: &Theme) -> menu::Style {
    menu::Style {
        background: Background::Color(colors().button),
        border: Border { color: colors().frame, width: 1.0, radius: RADIUS.into() },
        text_color: colors().text,
        selected_text_color: colors().text,
        selected_background: Background::Color(alpha(colors().accent, 0.25)),
        shadow: popup_shadow(),
    }
}

/// A small, unobtrusive arrow.
pub fn dropdown_handle() -> pick_list::Handle<iced::Font> {
    pick_list::Handle::Arrow { size: Some(9.0.into()) }
}

pub fn check(_: &Theme, status: checkbox::Status) -> checkbox::Style {
    let (checked, hovered, disabled) = match status {
        checkbox::Status::Active { is_checked } => (is_checked, false, false),
        checkbox::Status::Hovered { is_checked } => (is_checked, true, false),
        checkbox::Status::Disabled { is_checked } => (is_checked, false, true),
    };
    let (bg, border) = match (checked, hovered, disabled) {
        (_, _, true) => (colors().window, alpha(colors().frame, 0.6)),
        (true, _, _) => (colors().accent, colors().accent),
        (false, true, _) => (colors().button, colors().accent),
        (false, false, _) => (colors().button, colors().frame),
    };
    checkbox::Style {
        background: Background::Color(bg),
        icon_color: Color::WHITE,
        border: Border { color: border, width: 1.0, radius: ITEM_RADIUS.into() },
        text_color: Some(if disabled { colors().disabled } else { colors().text }),
    }
}

/// A vertical scrollable with a thin scrollbar on a transparent rail; the
/// handle turns accent while hovered, as in Breeze.
pub fn scroll<'a, Message: 'a>(content: impl Into<iced::Element<'a, Message>>) -> scrollable::Scrollable<'a, Message> {
    let bar = scrollable::Scrollbar::new().width(6).scroller_width(6).margin(2);
    scrollable::Scrollable::new(content).direction(scrollable::Direction::Vertical(bar)).style(scrollbar)
}

pub fn scrollbar(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let mut s = scrollable::default(theme, status);
    let thumb = match status {
        scrollable::Status::Dragged { is_vertical_scrollbar_dragged: true, .. }
        | scrollable::Status::Hovered { is_vertical_scrollbar_hovered: true, .. } => colors().accent,
        _ => alpha(colors().text, 0.28),
    };
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller {
            background: Background::Color(thumb),
            border: Border { radius: 3.0.into(), ..Default::default() },
        },
    };
    s.vertical_rail = rail;
    s.horizontal_rail = rail;
    s
}
