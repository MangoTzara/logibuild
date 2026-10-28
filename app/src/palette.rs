//! Component library on the left: search, categories and project circuits.

use iced::widget::{Space, button, column, container, row, stack, text, text_input, tooltip};
use iced::{Alignment, Element, Fill, Padding};

use lb_core::component::{Category, Kind};

use crate::app::{App, Message};
use crate::canvas::Tool;
use crate::elide::elide;
use crate::icons::{self, Icon};
use crate::theme;

fn matches(kind: Kind, query: &str) -> bool {
    query.is_empty()
        || kind.name().to_lowercase().contains(query)
        || format!("{kind:?}").to_lowercase().contains(query)
        || kind.category().name().to_lowercase().contains(query)
}

/// Library sections, top to bottom.
const ORDER: [Category; 7] = [
    Category::Annotations,
    Category::Io,
    Category::Displays,
    Category::Gates,
    Category::Combinational,
    Category::Sequential,
    Category::Memory,
];

pub const CIRCUITS_SECTION: &str = "Project circuits";

/// Clickable section header with a disclosure arrow; `count` is shown when
/// collapsed.
fn header(label: &str, open: bool, count: usize) -> Element<'_, Message> {
    let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
    let mut line = row![
        icons::icon(chevron, 12.0, theme::colors().muted),
        text(label.to_string()).size(theme::fs(theme::SMALL)).font(theme::FONT_MEDIUM).color(theme::colors().muted),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    if !open {
        line = line
            .push(Space::new().width(Fill))
            .push(text(format!("{count}")).size(theme::fs(theme::TINY)).color(theme::colors().muted));
    }
    button(line)
        .on_press(Message::ToggleSection(label.to_string()))
        .width(Fill)
        .padding([6, 6])
        .style(theme::item(false))
        .into()
}

/// Padding of list entries: aligned with the section titles.
const ENTRY: Padding = Padding { top: 4.0, right: 8.0, bottom: 4.0, left: 24.0 };

pub fn view(app: &App) -> Element<'_, Message> {
    let search = text_input("Search components…", &app.search)
        .on_input(Message::Search)
        .size(theme::fs(theme::BODY))
        .padding(Padding { top: 6.0, right: 8.0, bottom: 6.0, left: 28.0 })
        .style(theme::input);
    let search = stack![
        search,
        container(icons::icon(Icon::Search, 14.0, theme::colors().muted)).height(Fill).padding([0, 8]).center_y(Fill)
    ];
    let query = app.search.trim().to_lowercase();
    let searching = !query.is_empty();
    // While searching every section is shown expanded with its matches.
    let is_open = |section: &str| searching || !app.collapsed.contains(section);
    let current = match &app.tool {
        Tool::Place { kind, props, .. } => Some((*kind, props.circuit.clone())),
        Tool::Select => None,
    };

    let mut list = column![].spacing(1);

    // Circuits of the project: insert as subcircuit or open.
    let here = app.circuit().name.clone();
    let circuits: Vec<(usize, &lb_core::circuit::Circuit)> = app
        .project
        .circuits
        .iter()
        .enumerate()
        .filter(|(_, c)| !searching || c.name.to_lowercase().contains(&query))
        .collect();
    if !searching || !circuits.is_empty() {
        let open = is_open(CIRCUITS_SECTION);
        list = list.push(header(CIRCUITS_SECTION, open, app.project.circuits.len()));
        if open {
            for (i, c) in circuits {
                let name = c.name.clone();
                let refusal = app.placement_refusal(&name);
                let placeable = refusal.is_none();
                let shown = name == here;
                let selected = current.as_ref().is_some_and(|(k, n)| *k == Kind::Subcircuit && *n == name);
                // The main circuit (★) is never placeable; others only when
                // they would not contain the shown circuit.
                let tip = if !shown && i > 0 && !placeable { "(recursive)" } else { "" };
                // Circuits synthesized from HDL, orange when out of date.
                let (tag, tag_color) = match &c.hdl {
                    Some(h) if h.stale => ("HDL", theme::colors().neutral),
                    Some(_) => ("HDL", theme::colors().muted),
                    None => ("", theme::colors().muted),
                };
                // Right column, right aligned: "Open", or "Editing" for the shown
                // circuit. Inside the row, so that the whole row lights up.
                let side: Element<Message> = if shown {
                    // A state, not an action: grey and without a badge.
                    text("Editing").size(theme::fs(theme::SMALL)).color(theme::colors().muted).into()
                } else {
                    button(text("Open").size(theme::fs(theme::SMALL)))
                        .on_press(Message::OpenCircuit(name.clone()))
                        .padding([0, 2])
                        .style(theme::bare(theme::colors().link, theme::colors().accent))
                        .into()
                };
                // The shown circuit cannot be placed in itself: its name is only
                // coloured, unlike the highlight of the subcircuit being placed.
                // The name is cut to keep the right column in view.
                let mut left =
                    row![elide(name.clone()).size(theme::fs(theme::BODY))].spacing(6).align_y(Alignment::Center);
                if i == 0 {
                    left = left.push(text("★").size(theme::fs(theme::BODY)));
                }
                // The HDL tag sits right aligned, before the right column.
                let mut content =
                    row![left.push(text(tip).size(theme::fs(theme::SMALL)).color(theme::colors().muted)).width(Fill)]
                        .spacing(6)
                        .align_y(Alignment::Center);
                if !tag.is_empty() {
                    content = content.push(text(tag).size(theme::fs(theme::TINY)).color(tag_color));
                }
                let content = content.push(side);
                let pick: Element<Message> = if placeable {
                    button(content)
                        .on_press(Message::PickSubcircuit(name.clone()))
                        .width(Fill)
                        .padding(ENTRY)
                        .style(theme::item(selected))
                        .into()
                } else {
                    // Not a disabled button: a button shows the hand only for
                    // itself, never for the “Open” inside it.
                    let color = if shown { theme::colors().link } else { theme::colors().disabled };
                    container(content)
                        .width(Fill)
                        .padding(ENTRY)
                        .style(move |_| container::Style { text_color: Some(color), ..Default::default() })
                        .into()
                };
                // Say why a circuit cannot be placed here.
                let pick = match refusal {
                    Some(why) if !shown => tooltip(
                        pick,
                        container(text(why).size(theme::fs(theme::SMALL)))
                            .padding([4, 8])
                            .max_width(260)
                            .style(theme::tooltip_box),
                        tooltip::Position::Bottom,
                    )
                    .into(),
                    _ => pick,
                };
                list = list.push(pick);
            }
        }
    }

    let mut any = false;
    for cat in ORDER {
        let kinds: Vec<Kind> = Kind::ALL
            .iter()
            .copied()
            .filter(|k| k.category() == cat && *k != Kind::Subcircuit && matches(*k, &query))
            .collect();
        if kinds.is_empty() {
            continue;
        }
        any = true;
        let open = is_open(cat.name());
        list = list.push(header(cat.name(), open, kinds.len()));
        if !open {
            continue;
        }
        for k in kinds {
            let selected = current.as_ref().is_some_and(|(c, _)| *c == k);
            list = list.push(
                button(text(k.name()).size(theme::fs(theme::BODY)))
                    .on_press(Message::PickTool(k))
                    .width(Fill)
                    .padding(ENTRY)
                    .style(theme::item(selected)),
            );
        }
    }
    if !any && searching {
        list = list.push(
            container(text("No components found").size(theme::fs(theme::SMALL)).color(theme::colors().muted))
                .padding(8),
        );
    }

    let new = theme::tip(
        button(theme::icon_label(Icon::Add, "", true)).on_press(Message::NewCircuit).padding(6).style(theme::tool),
        "New circuit",
    );

    column![
        container(row![search, new].spacing(4).align_y(Alignment::Center)).padding(Padding {
            top: 8.0,
            right: 8.0,
            bottom: 6.0,
            left: 8.0
        }),
        theme::scroll(container(list).padding(Padding { top: 0.0, right: 10.0, bottom: 10.0, left: 6.0 })).height(Fill),
    ]
    .into()
}
