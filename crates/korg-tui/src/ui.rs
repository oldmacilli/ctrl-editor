//! Draw the app state.  Pure: reads `App`, writes the frame, and returns the
//! clickable areas (`Hits`) for the mouse.

use crate::app::{App, Focus, Hits, Modal, Screen, MENUS};
use korg_app::assign::MessageKind;
use korg_app::panel::{Binding, Control, ControlKind, Panel, TreeLine, TreeRow};
use korg_core::{Clock, TransferState};
use korg_midi::MidiTransport;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Circle, Line as CanvasLine, Rectangle};
use ratatui::widgets::{
    Block, BorderType, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Table, TableState, Wrap,
};
use ratatui::Frame;

const ACCENT: Color = Color::Cyan;
/// The scene the device is on.
const ACTIVE_SCENE: Color = Color::Rgb(0x6f, 0xcf, 0x7f);
const WARN: Color = Color::Yellow;
/// A control that just sent a message (MIDI monitor).
const LIT: Color = Color::Rgb(0xff, 0x9f, 0x43);
/// The second shade of the pulsing selection.
const ACCENT_DIM: Color = Color::Rgb(0x00, 0x7f, 0x8f);
/// MIDI monitor rows under the parameters.
const MONITOR_HEIGHT: u16 = 10;
/// The control tree column of the lower band.
const TREE_WIDTH: u16 = 30;

pub fn draw<T: MidiTransport, C: Clock>(frame: &mut Frame, app: &App<T, C>) -> Hits {
    let mut hits = Hits::default();
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .areas(frame.area());
    draw_header(frame, header, app, &mut hits);
    match app.screen {
        Screen::Devices => draw_devices(frame, body, app, &mut hits),
        Screen::Editor => draw_editor(frame, body, app, &mut hits),
        Screen::Browse => draw_browse(frame, body, app, &mut hits),
    }
    draw_footer(frame, footer, app);
    match &app.modal {
        Modal::None => {}
        Modal::Help => draw_help(frame, app),
        Modal::Menu { menu, item } => draw_menu(frame, app, *menu, *item, &mut hits),
        Modal::Prompt { kind, input } => {
            let area = popup(frame.area(), 70, 3);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(Line::from(vec![Span::raw(input.as_str()), "█".fg(ACCENT)])).block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title(format!(" {} ", kind.title()))
                        .title_bottom(" Enter accept · Esc cancel "),
                ),
                area,
            );
        }
        Modal::Confirm { text, .. } => {
            let area = popup(frame.area(), 70, 7);
            frame.render_widget(Clear, area);
            let block = Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(WARN))
                .title(" Write to device ");
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let [body, buttons] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
            frame.render_widget(
                Paragraph::new(text.as_str()).wrap(Wrap { trim: true }),
                body,
            );
            let yes = Rect::new(buttons.x + 1, buttons.y, 11, 1);
            let no = Rect::new(yes.right() + 2, buttons.y, 12, 1);
            frame.render_widget(
                Line::from(" y  Write ").fg(Color::Black).bg(WARN).bold(),
                yes,
            );
            frame.render_widget(Line::from(" n  Cancel ").reversed(), no);
            hits.confirm_yes = Some(yes);
            hits.confirm_no = Some(no);
        }
    }
    hits
}

fn draw_header<T: MidiTransport, C: Clock>(
    frame: &mut Frame,
    area: Rect,
    app: &App<T, C>,
    hits: &mut Hits,
) {
    let view = app.view();
    let open = match app.modal {
        Modal::Menu { menu, .. } => Some(menu),
        _ => None,
    };
    let mut spans = Vec::new();
    let mut x = area.x;
    for (index, (title, _)) in MENUS.iter().enumerate() {
        let text = format!(" {title} ");
        let width = text.chars().count() as u16;
        hits.menu_titles.push(Rect::new(x, area.y, width, 1));
        x += width;
        spans.push(if open == Some(index) {
            text.fg(Color::Black).bg(ACCENT).bold()
        } else {
            text.into()
        });
    }
    spans.push(" │ ".dim());
    spans.push("KORG Kontrol Editor ".fg(ACCENT).bold());
    if app.screen == Screen::Editor {
        spans.push(app.device_name().bold());
        if let Some(path) = &app.path {
            spans.push(format!("  {}", path.display()).dim());
        }
        if view.dirty {
            spans.push("  ● modified".fg(WARN));
        }
        if let Some(class) = &app.class {
            if korg_app::wireless::toggle_instruction(class).is_some() {
                let state = korg_app::wireless::WirelessState::of_links(&app.links(class));
                spans.push(format!("  Wireless {}", state.label()).dim());
            }
        }
    } else {
        spans.push("Devices".bold());
    }
    if let Some(port) = &app.port_override {
        spans.push(format!("  port {port:?}").dim());
    }
    let midi = format!("  {}", app.midi.button_label());
    spans.push(if app.midi.current().problem.is_some() {
        format!("{midi} (not available)").fg(WARN)
    } else {
        midi.dim()
    });
    frame.render_widget(Line::from(spans), area);
}

fn draw_footer<T: MidiTransport, C: Clock>(frame: &mut Frame, area: Rect, app: &App<T, C>) {
    let [status, keys] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
    let line = match &app.busy {
        Some(busy) => {
            let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
            let elapsed = busy.started.elapsed();
            let glyph = spinner[(elapsed.as_millis() / 100) as usize % spinner.len()];
            Line::from(vec![
                format!(" {glyph} {}: ", busy.label).fg(ACCENT).bold(),
                busy.step.clone().into(),
                format!("  {:.0} s", elapsed.as_secs_f32()).dim(),
            ])
        }
        None => Line::from(format!(" {}", app.status)),
    };
    frame.render_widget(line, status);
    let hints = match app.screen {
        Screen::Devices => "↑↓ select · Enter open · / search · c connected only · r receive · l link · o open · p port · M midi · F10 menu · ? help · q quit",
        Screen::Editor => "↑↓ move · ←→ change · [ ] scene · Tab device/list/params · u/U undo/redo · s save · r receive · w/W write · v bindings · F10 menu · ? help",
        Screen::Browse => "↑↓ select · Enter open · Esc back · o open a path · F10 menu · ? help",
    };
    frame.render_widget(Line::from(format!(" {hints}")).dim(), keys);
}

fn draw_menu<T: MidiTransport, C: Clock>(
    frame: &mut Frame,
    app: &App<T, C>,
    menu: usize,
    selected: usize,
    hits: &mut Hits,
) {
    let items = MENUS[menu].1;
    let Some(title) = hits.menu_titles.get(menu).copied() else {
        return;
    };
    let label_width = items
        .iter()
        .map(|i| i.label.chars().count())
        .max()
        .unwrap_or(0);
    let width = (label_width + 8) as u16;
    let screen = frame.area();
    let area = Rect::new(title.x, title.y + 1, width, items.len() as u16 + 2).intersection(screen);
    frame.render_widget(Clear, area);
    let rows: Vec<ListItem> = items
        .iter()
        .map(|item| {
            let text = format!("{:<label_width$}  {:>2}", item.label, item.keys);
            if app.unavailable(item.command).is_some() {
                ListItem::new(text.dim())
            } else {
                ListItem::new(text)
            }
        })
        .collect();
    let list = List::new(rows)
        .block(Block::bordered().border_type(BorderType::Rounded))
        .highlight_style(Style::new().fg(Color::Black).bg(ACCENT));
    let mut state = ListState::default().with_selected(Some(selected));
    frame.render_stateful_widget(list, area, &mut state);
    for index in 0..items.len() as u16 {
        let row = Rect::new(
            area.x + 1,
            area.y + 1 + index,
            area.width.saturating_sub(2),
            1,
        );
        hits.menu_items.push(row.intersection(screen));
    }
    if let Some(reason) = app.unavailable(items[selected].command) {
        let hint = Rect::new(
            area.x,
            area.bottom(),
            screen.width.saturating_sub(area.x),
            1,
        )
        .intersection(screen);
        frame.render_widget(Clear, hint);
        frame.render_widget(Line::from(format!(" {reason} ")).fg(WARN), hint);
    }
}

fn draw_devices<T: MidiTransport, C: Clock>(
    frame: &mut Frame,
    area: Rect,
    app: &App<T, C>,
    hits: &mut Hits,
) {
    let connected = app.connected();
    let rows = app.device_rows();
    let [list_area, detail_area] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area);
    let items: Vec<ListItem> = rows
        .iter()
        .map(|&index| {
            let device = &app.catalog.devices[index];
            let editable = device.scenes.is_some() && device.scene_payload_size.is_some();
            let is_connected = connected.contains(&device.class);
            let mark = if is_connected {
                "● ".fg(Color::Green)
            } else {
                "  ".into()
            };
            let name = if editable {
                Span::raw(device.device.clone())
            } else {
                device.device.clone().dim()
            };
            let mut spans = vec![mark, name, format!("  {}", device.class).dim()];
            if is_connected {
                let labels: Vec<&str> = app
                    .links(&device.class)
                    .iter()
                    .map(|link| link.label())
                    .collect();
                if !labels.is_empty() {
                    spans.push(format!("  {}", labels.join(" + ")).dim());
                }
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let filter = if app.search.is_empty() {
        String::new()
    } else {
        format!(" · search {:?}", app.search)
    };
    let title = if app.connected_only {
        format!(" Connected devices ({}){filter} ", rows.len())
    } else {
        format!(
            " Devices ({}, {} connected){filter} ",
            rows.len(),
            connected.len()
        )
    };
    let list = List::new(items)
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(title),
        )
        .highlight_style(
            Style::new()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    let mut state =
        ListState::default().with_selected((!rows.is_empty()).then_some(app.device_cursor));
    frame.render_stateful_widget(list, list_area, &mut state);
    hits.devices = Some((inner(list_area), state.offset()));

    let device = rows
        .get(app.device_cursor)
        .map(|&index| &app.catalog.devices[index]);
    let detail = device
        .map(|device| {
            let mut lines = vec![
                Line::from(device.device.clone().bold()),
                Line::from(format!("Class      {}", device.class)),
                Line::from(format!(
                    "File key   {}",
                    device.file_model_key.as_deref().unwrap_or("—")
                )),
                Line::from(format!(
                    "Scenes     {}",
                    device.scenes.map(|n| n.to_string()).unwrap_or("—".into())
                )),
                Line::from(format!(
                    "Scene size {}",
                    device
                        .scene_payload_size
                        .map(|n| format!("{n} bytes"))
                        .unwrap_or("—".into())
                )),
                Line::from(format!("Files      {}", device.extensions.join(", "))),
                Line::default(),
            ];
            let measured = device
                .file_model_key
                .as_deref()
                .and_then(korg_devices::measured_layout_for_key)
                .is_some();
            lines.push(if measured {
                Line::from("Receive and write: measured on hardware.".fg(Color::Green))
            } else {
                Line::from("Receive and write: not measured for this model.".dim())
            });
            if let Some(found) = korg_app::detect::detected_class(&device.class) {
                lines.push(Line::from(format!(
                    "Firmware   {} · device id {}",
                    found.reply.firmware(),
                    found.reply.device_id
                )));
            }
            let links = app.links(&device.class);
            if let Some(port) = &app.port_override {
                lines.push(Line::from(vec![
                    format!("Link       {port}").into(),
                    "  (set with p)".dim(),
                ]));
            } else {
                match korg_app::choose_link(&links, app.link) {
                    Some(link) => {
                        let mut spans =
                            vec![
                                format!("Link       {} · {}", link.label(), link.ports_text())
                                    .into(),
                            ];
                        if let Some(next) = korg_app::next_link(&links, Some(link.kind)) {
                            spans.push(format!("  (l: {})", next.label()).dim());
                        }
                        lines.push(Line::from(spans));
                    }
                    None => lines.push(Line::from("Link       not connected".dim())),
                }
            }
            // Wireless is set by buttons on the device; the editor can only
            // see whether a Bluetooth link exists here.
            if !links.is_empty() {
                if let Some(instruction) = korg_app::wireless::toggle_instruction(&device.class) {
                    let state = korg_app::wireless::WirelessState::of_links(&links);
                    lines.push(Line::from(format!("Wireless   {}", state.label())));
                    lines.push(Line::from(format!("  on device: {instruction}")).dim());
                }
            }
            if device.scenes.is_none() || device.scene_payload_size.is_none() {
                lines.push(Line::from(
                    "Data model not extracted. Open a file with o.".fg(WARN),
                ));
            }
            lines
        })
        .unwrap_or_default();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(" Details ");
    let detail_inner = block.inner(detail_area);
    frame.render_widget(block, detail_area);
    let text_height = (detail.len() as u16 + 1).min(detail_inner.height);
    let [text_area, picture_area] =
        Layout::vertical([Constraint::Length(text_height), Constraint::Min(0)]).areas(detail_inner);
    frame.render_widget(Paragraph::new(detail).wrap(Wrap { trim: false }), text_area);
    if let Some(panel) = device.and_then(|d| korg_app::panel::panel(&d.class)) {
        if picture_area.height >= 4 {
            let area = fit(picture_area, &panel, picture_area.height);
            draw_panel(frame, area, &panel, None, &[], false, " Schematic ");
        }
    } else if device.is_some() && picture_area.height >= 1 {
        frame.render_widget(
            Line::from("No schematic for this device yet.").dim(),
            picture_area,
        );
    }
}

/// The largest rect inside `area`, at most `max_height` tall, that shows
/// `panel` undistorted (terminal cells are about twice as tall as wide).
fn fit(area: Rect, panel: &Panel, max_height: u16) -> Rect {
    let ratio = panel.height / panel.width / 2.0;
    let mut width = area.width;
    let mut height = ((f64::from(width.saturating_sub(2)) * ratio).round() as u16 + 2)
        .min(max_height)
        .min(area.height);
    let fitted_width = (f64::from(height.saturating_sub(2)) / ratio).round() as u16 + 2;
    if fitted_width < width {
        width = fitted_width;
    }
    height = height.max(3);
    let [row] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    Rect::new(row.x, area.y, row.width, height.min(area.height))
}

/// Draw `panel` as a schematic. Returns the drawing area inside the border.
fn draw_panel(
    frame: &mut Frame,
    area: Rect,
    panel: &Panel,
    selected: Option<usize>,
    lit: &[usize],
    focused: bool,
    title: &str,
) -> Rect {
    let label = selected
        .and_then(|index| panel.controls.get(index))
        .map(|c| format!(" {} · {} ", c.label, c.group));
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if focused {
            Style::new().fg(ACCENT)
        } else {
            Style::new()
        })
        .title(title.to_owned());
    if let Some(label) = label {
        block = block.title_bottom(Line::from(label).fg(ACCENT).bold());
    }
    let drawing = block.inner(area);
    let (w, h) = (panel.width, panel.height);
    let painted = lit.to_vec();
    // Braille dots: 2 across and 4 down per cell.
    let dot = h / (f64::from(drawing.height.max(1)) * 4.0);
    let canvas = Canvas::default()
        .block(block)
        .marker(Marker::Braille)
        .x_bounds([0.0, w])
        .y_bounds([0.0, h])
        .paint(move |ctx| {
            for (index, c) in panel.controls.iter().enumerate() {
                let sending = painted.contains(&index);
                // A sending control is filled like the selected one.
                let chosen = Some(index) == selected || sending;
                let color = if sending {
                    LIT
                } else if chosen {
                    ACCENT
                } else {
                    match (c.kind, c.binding) {
                        (ControlKind::Key { black: true }, _) => Color::DarkGray,
                        (_, Binding::None) => Color::Gray,
                        _ => Color::White,
                    }
                };
                // Canvas y grows upward.
                let y = h - c.y - c.h;
                match c.kind {
                    ControlKind::Knob | ControlKind::Jog => {
                        let (cx, _) = c.center();
                        let cy = y + c.h / 2.0;
                        ctx.draw(&Circle {
                            x: cx,
                            y: cy,
                            radius: c.w / 2.0,
                            color,
                        });
                        if chosen {
                            ctx.draw(&Circle {
                                x: cx,
                                y: cy,
                                radius: c.w / 4.0,
                                color,
                            });
                        }
                    }
                    ControlKind::Fader => {
                        let cx = c.x + c.w / 2.0;
                        ctx.draw(&CanvasLine::new(cx, y, cx, y + c.h, color));
                        ctx.draw(&Rectangle {
                            x: c.x,
                            y: y + c.h * 0.45,
                            width: c.w,
                            height: c.h * 0.1,
                            color,
                        });
                    }
                    _ => ctx.draw(&Rectangle {
                        x: c.x,
                        y,
                        width: c.w,
                        height: c.h,
                        color,
                    }),
                }
                if chosen && !matches!(c.kind, ControlKind::Knob | ControlKind::Jog) {
                    let mut line = y;
                    while line < y + c.h {
                        ctx.draw(&CanvasLine::new(c.x, line, c.x + c.w, line, color));
                        line += dot.max(0.5);
                    }
                }
            }
        });
    frame.render_widget(canvas, area);
    // Braille dots alone are hard to tell apart on small controls: mark the
    // cells of a sending control, and of the selected one, with a solid
    // background.  The selection pulses between two shades and gets ▶ ◀.
    let pulse = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() / 500 % 2);
    let selected_bg = if pulse == 0 { ACCENT } else { ACCENT_DIM };
    let marks = lit
        .iter()
        .map(|&index| (index, LIT, false))
        .chain(selected.map(|index| (index, selected_bg, true)));
    for (index, bg, arrows) in marks {
        if let Some(control) = panel.controls.get(index) {
            mark_cells(frame, drawing, panel, control, bg, arrows);
        }
    }
    drawing
}

/// The cells `control` covers in `drawing`, by the Braille grid's own
/// rounding (2 dots across and 4 down per cell), so the marks sit on the
/// dots the canvas drew.
fn control_cells(drawing: Rect, panel: &Panel, control: &Control) -> Rect {
    let dots_x = f64::from(drawing.width) * 2.0 - 1.0;
    let dots_y = f64::from(drawing.height) * 4.0 - 1.0;
    let col = |x: f64| ((x / panel.width * dots_x) as u16 / 2).min(drawing.width - 1);
    let row = |y: f64| ((y / panel.height * dots_y) as u16 / 4).min(drawing.height - 1);
    let (x0, x1) = (col(control.x), col(control.x + control.w));
    let (y0, y1) = (row(control.y), row(control.y + control.h));
    Rect::new(drawing.x + x0, drawing.y + y0, x1 - x0 + 1, y1 - y0 + 1)
}

/// Paint `control`'s cells with `bg` (dots in black), and with `arrows` put
/// ▶ ◀ in the empty cells beside it.
fn mark_cells(
    frame: &mut Frame,
    drawing: Rect,
    panel: &Panel,
    control: &Control,
    bg: Color,
    arrows: bool,
) {
    let cells = control_cells(drawing, panel, control);
    let buffer = frame.buffer_mut();
    for y in cells.top()..cells.bottom() {
        for x in cells.left()..cells.right() {
            buffer[(x, y)].set_bg(bg).set_fg(Color::Black);
        }
    }
    if !arrows {
        return;
    }
    let middle = cells.y + cells.height / 2;
    let empty = |symbol: &str| matches!(symbol, " " | "\u{2800}");
    if cells.left() > drawing.left() {
        let cell = &mut buffer[(cells.left() - 1, middle)];
        if empty(cell.symbol()) {
            cell.set_symbol("▶").set_fg(ACCENT);
        }
    }
    if cells.right() < drawing.right() {
        let cell = &mut buffer[(cells.right(), middle)];
        if empty(cell.symbol()) {
            cell.set_symbol("◀").set_fg(ACCENT);
        }
    }
}

fn draw_editor<T: MidiTransport, C: Clock>(
    frame: &mut Frame,
    area: Rect,
    app: &App<T, C>,
    hits: &mut Hits,
) {
    let view = app.view();
    let panel = app.panel();
    let [tabs_area, rest] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).areas(area);
    if view.scene_count > 0 {
        // Scene buttons, one clickable span each.
        let mut spans = Vec::new();
        let mut x = tabs_area.x;
        for (index, label) in view.scene_labels.iter().enumerate() {
            let text = format!(" {} {label} ", index + 1);
            // The device marks the scene it is on; it is not the scene the
            // editor shows.
            let active = app.monitor.active == Some(index as u8);
            let width = (text.chars().count() as u16 + u16::from(active))
                .min(tabs_area.right().saturating_sub(x));
            hits.tabs.push(Rect::new(x, tabs_area.y, width, 1));
            x += width + 1;
            spans.push(if index == view.selected_scene {
                text.fg(Color::Black).bg(ACCENT).bold()
            } else {
                text.into()
            });
            if active {
                spans.push("●".fg(ACTIVE_SCENE));
            }
            spans.push("│".dim());
        }
        frame.render_widget(Line::from(spans), tabs_area);
    } else if app.controller.session().document().global.is_some() {
        frame.render_widget(Line::from(" Global data").bold(), tabs_area);
    }

    // The schematic takes up to half the height, like the device picture
    // in the original editor.
    let main = match &panel {
        Some(panel) if rest.height >= 16 => {
            let area = fit(rest, panel, rest.height / 2);
            let drawing = draw_panel(
                frame,
                area,
                panel,
                app.control,
                &app.lit_controls(),
                app.focus == Focus::Panel,
                " Device ",
            );
            hits.panel = Some(drawing);
            // One line under the schematic: what the control sends.
            let status = Rect::new(rest.x, area.bottom(), rest.width, 1);
            draw_control_status(frame, status, app, panel);
            let mut bottom = status.bottom();
            if app.show_bindings && bottom < rest.bottom() {
                draw_bindings(frame, Rect::new(rest.x, bottom, rest.width, 1), app, panel);
                bottom += 1;
            }
            Rect::new(
                rest.x,
                bottom,
                rest.width,
                rest.bottom().saturating_sub(bottom),
            )
        }
        _ => rest,
    };

    let binding = app.selected_binding();
    // Lower band: the control tree on the left, the parameter table on the right.
    let params_area = match &panel {
        Some(panel) => {
            let [tree, work] =
                Layout::horizontal([Constraint::Length(TREE_WIDTH), Constraint::Min(20)])
                    .areas(main);
            draw_control_tree(frame, tree, app, panel, hits);
            work
        }
        None => main,
    };

    let params_area = if app.monitor_shown && params_area.height >= MONITOR_HEIGHT + 5 {
        let [params, monitor] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(MONITOR_HEIGHT)])
                .areas(params_area);
        draw_monitor(frame, monitor, app);
        params
    } else {
        params_area
    };
    let focused = |focus: Focus| {
        if app.focus == focus {
            Style::new().fg(ACCENT)
        } else {
            Style::new()
        }
    };
    // nanoKONTROL Studio: the selected control's record fields.  Other
    // devices: the schema rows.
    let fields = app.control_fields();
    let row_count = fields.as_ref().map_or(view.params.len(), Vec::len);
    let cursor = app.param_cursor.min(row_count.saturating_sub(1));
    let rows: Vec<Row> = match &fields {
        Some(fields) => fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let shown = field.display();
                let value = if index == cursor && app.focus == Focus::Params {
                    format!("◀ {shown} ▶")
                } else {
                    shown
                };
                // Page headings, as the GUI draws them above a block.
                let label = match field.section {
                    Some(section) => format!("{section} · {}", field.label),
                    None => field.label.to_owned(),
                };
                let value = if field.enabled {
                    Cell::from(value).bold()
                } else {
                    Cell::from(value).dim()
                };
                Row::new(vec![label.into(), value, Cell::from(field.range()).dim()])
            })
            .collect(),
        None => view
            .params
            .iter()
            .enumerate()
            .map(|(index, param)| {
                let range = crate::app::param_range(param);
                let shown = crate::app::param_display(param);
                let value = if index == cursor && app.focus == Focus::Params {
                    format!("◀ {shown} ▶")
                } else {
                    shown
                };
                // The label the GUI shows for the same row.
                let label = korg_app::pages::schema_label(&param.label);
                let label = if binding == Binding::Element(param.element) {
                    Cell::from(label).fg(ACCENT)
                } else {
                    Cell::from(label)
                };
                Row::new(vec![
                    label,
                    Cell::from(value).bold(),
                    Cell::from(range).dim(),
                ])
            })
            .collect(),
    };
    let title = match &fields {
        Some(fields) if fields.is_empty() => " Parameters: no fields for this control ".to_owned(),
        Some(fields) => format!(" Parameters ({}) ", fields.len()),
        None if app.global_page && view.params.is_empty() => {
            " Global: none loaded (Send → Load Global Data) ".to_owned()
        }
        None if app.global_page => format!(" Global ({}) ", view.params.len()),
        None if view.params.is_empty() => " Parameters: no schema for this device ".to_owned(),
        None => format!(" Parameters ({}) ", view.params.len()),
    };
    let widths = [
        Constraint::Fill(2),
        Constraint::Fill(1),
        Constraint::Length(13),
    ];
    let table = Table::new(rows, widths)
        .header(Row::new(["Parameter", "Value", "Range"]).dim())
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(focused(Focus::Params))
                .title(title),
        )
        .row_highlight_style(if app.focus == Focus::Params {
            Style::new().bg(Color::DarkGray)
        } else {
            Style::new()
        })
        .highlight_symbol("▶ ");
    let mut state = TableState::default().with_selected((row_count > 0).then_some(cursor));
    frame.render_stateful_widget(table, params_area, &mut state);
    let table_inner = inner(params_area);
    let rows_area = Rect {
        y: table_inner.y + 1,
        height: table_inner.height.saturating_sub(1),
        ..table_inner
    };
    if row_count > 0 {
        // Same split as the table: highlight column, then the three columns
        // with one cell of spacing.
        let [_, columns] =
            Layout::horizontal([Constraint::Length(2), Constraint::Min(0)]).areas(rows_area);
        let [_, value, _] = Layout::horizontal(widths).spacing(1).areas(columns);
        hits.params = Some((rows_area, state.offset(), (value.x, value.right())));
        if row_count > rows_area.height as usize {
            let mut scroll = ScrollbarState::new(row_count).position(cursor);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                params_area.inner(ratatui::layout::Margin::new(0, 1)),
                &mut scroll,
            );
        }
    }
    if matches!(
        view.transfer,
        TransferState::Sending(_) | TransferState::Waiting(_)
    ) {
        let area = popup(main, 40, 3);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new("Transfer running…").block(Block::bordered()),
            area,
        );
    }
}

/// The control list: group headers, single controls and the chips of the
/// open group.  The list takes the whole box.
fn draw_control_tree<T: MidiTransport, C: Clock>(
    frame: &mut Frame,
    area: Rect,
    app: &App<T, C>,
    panel: &Panel,
    hits: &mut Hits,
) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if app.focus == Focus::List {
            Style::new().fg(ACCENT)
        } else {
            Style::new()
        })
        .title(Line::from(vec![
            " Controls ".into(),
            " ".into(),
            "Note".fg(rgb_color(MessageKind::Note)),
            " ".into(),
            "CC".fg(rgb_color(MessageKind::ControlChange)),
            " ".into(),
            "PC".fg(rgb_color(MessageKind::ProgramChange)),
            " ".into(),
        ]));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let chars = inner.width as usize;
    let lines = panel.tree_lines(app.open_group.as_deref(), app.control, chars);
    if lines.is_empty() {
        return;
    }
    let selected_line = |line: &TreeLine| match line {
        TreeLine::Row(TreeRow::Group { selected, .. })
        | TreeLine::Row(TreeRow::Control { selected, .. }) => *selected,
        TreeLine::Chips(chips) => chips.iter().any(|chip| chip.selected),
    };
    let cursor = if app.focus == Focus::List {
        Some(app.list_cursor)
    } else {
        lines.iter().position(selected_line)
    };
    let height = inner.height as usize;
    let offset = cursor
        .filter(|&index| height > 0 && index >= height)
        .map_or(0, |index| index + 1 - height)
        .min(lines.len().saturating_sub(height));
    let scene = app.scene_bytes();
    let message = |index: usize| -> Option<Color> {
        let control = panel.controls.get(index)?;
        let message = korg_app::assign::control_message(&panel.class, control, scene)?;
        Some(rgb_color(message.kind))
    };
    let lit = app.lit_controls();
    let mut rendered = Vec::new();
    for (index, line) in lines.iter().enumerate().skip(offset).take(height) {
        let y = inner.y + (index - offset) as u16;
        let focused = app.focus == Focus::List && index == app.list_cursor;
        match line {
            TreeLine::Row(row) => {
                rendered.push(match row {
                    TreeRow::Group {
                        name,
                        count,
                        open,
                        selected,
                    } => {
                        let marker = if *open { "▾ " } else { "▸ " };
                        let mut line = Line::from(format!("{marker}{name} ({count})"));
                        if *selected {
                            line = line.bold();
                        }
                        if focused {
                            line = line.bg(Color::DarkGray);
                        }
                        line
                    }
                    TreeRow::Control {
                        index: control,
                        label,
                        selected,
                        ..
                    } => {
                        let line = Line::from(label.clone());
                        if lit.contains(control) {
                            line.fg(Color::Black).bg(LIT)
                        } else if *selected {
                            line.fg(Color::Black).bg(ACCENT)
                        } else if focused {
                            line.bg(Color::DarkGray)
                        } else {
                            line
                        }
                    }
                });
            }
            TreeLine::Chips(chips) => {
                let mut spans = vec![Span::raw("  ")];
                let mut x = inner.x + 2;
                for (at, chip) in chips.iter().enumerate() {
                    if at > 0 {
                        spans.push(Span::raw(" "));
                        x += 1;
                    }
                    let width = chip.label.chars().count() as u16;
                    let mut span = Span::raw(chip.label.clone());
                    if let Some(color) = message(chip.index) {
                        span = span.fg(color);
                    }
                    if chip.selected {
                        span = span.fg(Color::Black).bg(ACCENT).bold();
                    }
                    if lit.contains(&chip.index) {
                        span = span.fg(Color::Black).bg(LIT).bold();
                    }
                    hits.chips.push((Rect::new(x, y, width, 1), chip.index));
                    spans.push(span);
                    x += width;
                }
                rendered.push(Line::from(spans));
            }
        }
    }
    frame.render_widget(Paragraph::new(rendered), inner);
    hits.tree = Some((inner, offset, chars));
}

/// The MIDI monitor: the newest messages the device sent, last at the
/// bottom, coloured like the control list.
fn draw_monitor<T: MidiTransport, C: Clock>(frame: &mut Frame, area: Rect, app: &App<T, C>) {
    let title = match &app.monitor.ports {
        Some((input, _)) => format!(" MIDI Monitor · {input} (i hides) "),
        None => " MIDI Monitor · not connected (i hides) ".to_owned(),
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = inner.height as usize;
    let skip = app.monitor.log.len().saturating_sub(rows);
    let mut lines: Vec<Line> = app
        .monitor
        .log
        .iter()
        .skip(skip)
        .map(|event| {
            let text = format!("{:<28}", event.message.describe());
            let text = match event.message.kind() {
                Some(kind) => text.fg(rgb_color(kind)),
                None => text.into(),
            };
            Line::from(vec![
                format!("{:>9}  ", event.time_text()).dim(),
                text,
                format!("  {}", event.message.hex()).dim(),
            ])
        })
        .collect();
    if lines.is_empty() {
        lines.push(Line::from(
            "Move a control on the device; its messages appear here.".dim(),
        ));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn rgb_color(kind: MessageKind) -> Color {
    let (r, g, b) = kind.rgb();
    Color::Rgb(r, g, b)
}

/// What the selected control sends in this scene, one line under the
/// schematic.
fn draw_control_status<T: MidiTransport, C: Clock>(
    frame: &mut Frame,
    area: Rect,
    app: &App<T, C>,
    panel: &Panel,
) {
    let text = app.control_details().unwrap_or_else(|| {
        "Click a control on the device, or press Tab and use the arrows.".into()
    });
    let mut spans = vec![format!(" {text}").into()];
    if app.control.is_some() && panel.binding_inferred {
        spans.push(" · Control-to-element order is inferred".fg(WARN));
    }
    frame.render_widget(Line::from(spans), area);
}

/// The binding of each control of the open group, on one line.
fn draw_bindings<T: MidiTransport, C: Clock>(
    frame: &mut Frame,
    area: Rect,
    app: &App<T, C>,
    panel: &Panel,
) {
    let group = app.open_group.as_deref();
    let scene = app.scene_bytes();
    let mut text = String::new();
    for control in &panel.controls {
        if group.is_some_and(|group| control.group != group) {
            continue;
        }
        let detail = korg_app::assign::details(&panel.class, control, scene);
        let item = format!("{}: {detail}", control.label);
        let extra = if text.is_empty() { 0 } else { 3 };
        if text.chars().count() + item.chars().count() + extra + 2 > usize::from(area.width) {
            if !text.is_empty() {
                text.push_str(" · …");
            }
            break;
        }
        if extra > 0 {
            text.push_str(" · ");
        }
        text.push_str(&item);
    }
    if text.is_empty() {
        text = match group {
            Some(group) => format!("No control in {group}."),
            None => "No control has a decoded binding.".into(),
        };
    }
    frame.render_widget(Line::from(format!(" {text}").dim()), area);
}

/// The files of the current device under the product folder.
fn draw_browse<T: MidiTransport, C: Clock>(
    frame: &mut Frame,
    area: Rect,
    app: &App<T, C>,
    hits: &mut Hits,
) {
    let files = app.browse_files();
    let [list_area, detail_area] =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area);
    let items: Vec<ListItem> = files
        .iter()
        .map(|(label, _)| ListItem::new(Line::from(label.clone())))
        .collect();
    let list = List::new(items)
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(format!(" {} files ({}) ", app.device_name(), files.len())),
        )
        .highlight_style(
            Style::new()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    let mut state =
        ListState::default().with_selected((!files.is_empty()).then_some(app.browse_cursor));
    frame.render_stateful_widget(list, list_area, &mut state);
    hits.browse = Some((inner(list_area), state.offset()));

    let mut lines = vec![
        Line::from(app.device_name().bold()),
        Line::from(format!("Folder  {}", app.products.display())),
        Line::default(),
    ];
    match files.get(app.browse_cursor) {
        Some((_, path)) => {
            lines.push(Line::from(path.display().to_string()));
            lines.push(Line::from("Enter opens this file.".dim()));
        }
        None => {
            lines.push(Line::from("No file for this device."));
            lines.push(Line::from("Press o to open a path.".dim()));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" File "),
        ),
        detail_area,
    );
}

fn draw_help<T: MidiTransport, C: Clock>(frame: &mut Frame, app: &App<T, C>) {
    let mut text = String::from(
        "\
Devices
  ↑↓ / j k     select a device
  Enter        open it offline (factory scenes)
  r            receive its scene set over MIDI
  c            show only connected devices
  l            switch link (USB / Bluetooth)
  /            search the list (Esc clears it)
  b            browse the device's files
Editor
  ↑↓ / j k     move; g / G first / last
  ← → / h l    change the value; Shift or H / L: by 10
  Enter        type a value (a number, a note name such as
               C#4 for a note, or a choice label)
  [ ] / 1-9    previous / next / numbered scene
  Tab          device → list → parameters;
               on the device the arrows move between controls
  u / U        undo / redo (also Ctrl-R)
  s            save to a file
  r            receive the scene set from the device
  w / W        write this scene / every scene (asks first,
               then reads the data back and compares)
  v            show the binding of each control
  Esc          back to the device list
Browse
  ↑↓ / j k     select a file    Enter  open it
  Esc          back
Control list (Tab)
  ↑↓ / j k     move between rows; ← → between chips
  Enter/Space  open or close a group; select a chip
Anywhere
  F10 / m      menus            o   open a file
  p            MIDI port        ?   this help
  M            MIDI backend (Linux: ALSA sequencer / JACK)
  i            MIDI monitor: what the device sends; the
               control that sent it lights up
  x            cancel the running job (it ends after its
               current step, because a request cannot stop)
  q            quit (twice with unsaved edits)
Mouse
  click        menus, devices (double-click opens), scenes,
               controls on the device, list rows and chips,
               rows; ◀ ▶ step a value
  wheel        scroll; over a value it changes the value

Receive and write use the exchange measured on hardware for
nanoKEY Studio (1480) and nanoKONTROL Studio (1482) only.",
    );
    if let Some(note) = app
        .class
        .as_deref()
        .and_then(korg_app::wireless::toggle_note)
    {
        text.push_str("\n\nWireless (buttons on the device only)\n");
        for line in wrap_text(&note, 60) {
            text.push_str("  ");
            text.push_str(&line);
            text.push('\n');
        }
    }
    text.push_str("\nPress any key to close.");
    let area = popup(frame.area(), 72, 50);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" Help "),
        ),
        area,
    );
}

/// Greedy word wrap of `text` to at most `width` columns.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// `area` without its border.
fn inner(area: Rect) -> Rect {
    area.inner(ratatui::layout::Margin::new(1, 1))
}

fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let [row] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    let [cell] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(row);
    cell
}
