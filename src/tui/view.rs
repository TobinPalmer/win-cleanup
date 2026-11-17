use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, Wrap},
};

use super::{App, Screen, Sort};
use crate::scan::{Item, Risk, size};

const PURPLE: Color = Color::Rgb(124, 58, 237);
const CYAN: Color = Color::Rgb(6, 182, 212);
const GREEN: Color = Color::Rgb(16, 185, 129);
const YELLOW: Color = Color::Rgb(245, 158, 11);
const RED: Color = Color::Rgb(239, 68, 68);
const MUTED: Color = Color::Rgb(115, 124, 140);
const BORDER: Color = Color::Rgb(55, 65, 81);

fn fg(color: Color) -> Style {
    Style::default().fg(color)
}

fn dot(risk: Risk) -> Span<'static> {
    Span::styled(
        "● ",
        fg(match risk {
            Risk::Safe => GREEN,
            Risk::Moderate => YELLOW,
            Risk::Risky => RED,
        }),
    )
}

fn cursor(active: bool) -> Span<'static> {
    Span::styled(if active { "▸ " } else { "  " }, fg(CYAN))
}

fn check(checked: bool) -> Span<'static> {
    Span::styled(
        if checked { "[✓] " } else { "[ ] " },
        if checked { fg(GREEN) } else { Style::default() },
    )
}

fn name(text: impl Into<String>, active: bool) -> Span<'static> {
    Span::styled(
        text.into(),
        if active {
            fg(PURPLE).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        },
    )
}

fn sized(bytes: u64) -> Cell<'static> {
    Cell::from(Line::styled(size(bytes), fg(CYAN)).alignment(Alignment::Right))
}

fn right(text: impl Into<String>) -> Cell<'static> {
    Cell::from(Line::styled(text.into(), fg(MUTED)).alignment(Alignment::Right))
}

fn title(text: &str) -> Line<'static> {
    Line::styled(
        format!("  {text}  "),
        fg(Color::White).bg(PURPLE).add_modifier(Modifier::BOLD),
    )
}

fn rule(width: u16) -> Line<'static> {
    Line::styled("─".repeat(usize::from(width.min(60))), fg(BORDER))
}

fn box_frame() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(fg(BORDER))
}

fn scroll_for(cursor: usize, scroll: &mut usize, visible: usize) -> usize {
    *scroll = (*scroll).clamp((cursor + 1).saturating_sub(visible), cursor);
    *scroll
}

fn table(rows: Vec<Row<'static>>, heading: &str, tail: &str) -> Table<'static> {
    Table::new(
        rows,
        [
            Constraint::Min(8),
            Constraint::Length(11),
            Constraint::Length(9),
        ],
    )
    .header(Row::new([
        Cell::from(format!("        {heading}")).style(fg(MUTED)),
        right("Size"),
        right(tail),
    ]))
}

fn warning_count(app: &App) -> usize {
    app.targets.iter().map(|target| target.warnings.len()).sum()
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.height < 10 || area.width < 32 {
        frame.render_widget(Paragraph::new("Resize terminal · q quit"), area);
        return;
    }
    let body = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(3),
    ])
    .split(area);
    let (title_text, keys) = match app.screen {
        Screen::List => (
            "Windows Cleanup",
            "↑/↓ move  •  space select  •  a/d all/none  •  enter review  •  ? help  •  q quit",
        ),
        Screen::Preview | Screen::Confirm => (
            "Cleanup Preview",
            "↑/↓ move  •  space toggle  •  →/l expand  •  enter inspect  •  y recycle  •  esc back  •  ? help",
        ),
        Screen::Cleaning => ("Recycling", ""),
        Screen::Report => (
            "Cleanup Report",
            "↑/↓ scroll  •  enter rescan  •  ? help  •  q quit",
        ),
    };
    frame.render_widget(Paragraph::new(title(title_text)), body[0]);
    let status = match app.screen {
        Screen::List if app.scan_pending() && app.status.is_empty() => {
            let names = app
                .scanning
                .iter()
                .map(|(_, name)| *name)
                .collect::<Vec<_>>();
            format!(
                "Scanning… {}/{} · {}",
                app.scanned,
                app.targets.len(),
                names.join(", ")
            )
        }
        Screen::List if warning_count(app) > 0 && app.status.is_empty() => {
            format!(
                "[!] {} scan warnings · w details · f full scan",
                warning_count(app)
            )
        }
        _ => app.status.clone(),
    };
    frame.render_widget(
        Paragraph::new(vec![
            rule(area.width),
            Line::styled(keys, fg(MUTED)),
            Line::styled(status, fg(YELLOW)),
        ]),
        body[2],
    );

    match app.screen {
        Screen::List => list(frame, body[1], app),
        Screen::Preview | Screen::Confirm => preview(frame, body[1], app),
        Screen::Cleaning => {
            frame.render_widget(
                Paragraph::new(
                    "\nChecking selected items and moving them to the Windows Recycle Bin… Cancel any Windows prompt to permanently delete.",
                )
                .wrap(Wrap { trim: false }),
                body[1],
            );
        }
        Screen::Report => report(frame, body[1], app),
    }
    if app.screen == Screen::Confirm {
        confirm(frame, app);
    }
    if app.help {
        help(frame, app);
    }
    if app.warnings {
        scan_warnings(frame, app);
    }
}

fn list(frame: &mut Frame, area: Rect, app: &mut App) {
    let available = size(app.targets.iter().map(|t| t.bytes()).sum());
    let selected = size(app.selected.iter().map(|i| app.targets[*i].bytes()).sum());
    let summary = Line::from(vec![
        Span::raw("Available  "),
        Span::styled(available.clone(), fg(CYAN)),
        Span::raw("  │  Selected  "),
        Span::styled(format!("{selected} ({})", app.selected.len()), fg(CYAN)),
    ]);
    let compact = area.height < 12;
    let wide = area.width >= 100 && !compact;
    let parts = Layout::vertical([
        Constraint::Length(if compact { 2 } else { 7 }),
        Constraint::Min(1),
    ])
    .split(area);
    let lines = if compact {
        vec![summary, rule(area.width)]
    } else {
        vec![
            Line::default(),
            Line::from(vec![
                dot(Risk::Safe),
                Span::raw("Safe      "),
                Span::styled("Auto-regenerated caches", fg(MUTED)),
            ]),
            Line::from(vec![
                dot(Risk::Moderate),
                Span::raw("Moderate  "),
                Span::styled("Temporary files and diagnostics", fg(MUTED)),
            ]),
            Line::from(vec![
                dot(Risk::Risky),
                Span::raw("Risky     "),
                Span::styled("May contain important data", fg(MUTED)),
            ]),
            Line::default(),
            summary,
            rule(area.width),
        ]
    };
    frame.render_widget(Paragraph::new(lines), parts[0]);
    let grid = Layout::horizontal([
        Constraint::Max(68),
        Constraint::Length(if wide { 8 } else { 0 }),
        Constraint::Length(if wide { 28 } else { 0 }),
        Constraint::Min(0),
    ])
    .split(parts[1]);
    let rows_area = if wide {
        grid[0]
    } else {
        Rect {
            width: parts[1].width.min(68),
            ..parts[1]
        }
    };
    let visible = usize::from(rows_area.height.saturating_sub(1)).max(1);
    let start = scroll_for(app.list_cursor, &mut app.list_scroll, visible);
    let rows = app
        .targets
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(i, target)| {
            let current = i == app.list_cursor;
            let label = Line::from(vec![
                cursor(current),
                check(app.selected.contains(&i)),
                dot(target.risk),
                name(target.name, current),
            ]);
            Row::new([
                Cell::from(label),
                sized(target.bytes()),
                right(target.count().to_string()),
            ])
        })
        .collect();
    frame.render_widget(table(rows, "Name", "Count"), rows_area);
    if wide {
        let mut summary = vec![
            title("Summary"),
            Line::default(),
            Line::raw(format!("Available  {available}")),
            Line::raw(format!("Selected   {selected}")),
        ];
        if !app.selected.is_empty() {
            summary.extend([Line::default(), Line::styled("Selected Items", fg(MUTED))]);
            summary.extend(
                app.targets
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| app.selected.contains(i))
                    .take(6)
                    .map(|(_, t)| Line::from(vec![dot(t.risk), Span::raw(t.name)])),
            );
            if app.selected.len() > 6 {
                summary.push(Line::styled(
                    format!("+{} more", app.selected.len() - 6),
                    fg(MUTED),
                ));
            }
        }
        let panel = Rect {
            height: (summary.len() as u16 + 2).min(grid[2].height),
            ..grid[2]
        };
        frame.render_widget(Paragraph::new(summary).block(box_frame()), panel);
    }
}

fn age(item: &Item) -> String {
    item.modified.and_then(|t| t.elapsed().ok()).map_or_else(
        || "—".into(),
        |duration| {
            let hours = duration.as_secs() / 3600;
            if hours >= 24 {
                format!("{}d", hours / 24)
            } else if hours > 0 {
                format!("{hours}h")
            } else {
                format!("{}m", duration.as_secs() / 60)
            }
        },
    )
}

// Escape control characters in names and OS errors before drawing them.
fn display(text: impl AsRef<str>) -> String {
    text.as_ref()
        .chars()
        .map(|c| if c.is_control() { '�' } else { c })
        .collect()
}

fn shorten_path(path: String, width: usize) -> String {
    if Span::raw(path.as_str()).width() <= width {
        return path;
    }
    let mut remaining = width.saturating_sub(1);
    let mut start = path.len();
    for (index, ch) in path.char_indices().rev() {
        let columns = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if columns > remaining {
            break;
        }
        remaining -= columns;
        start = index;
    }
    format!("…{}", &path[start..])
}

fn preview(frame: &mut Frame, area: Rect, app: &mut App) {
    let parts = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(area);
    let sort = match app.sort {
        Sort::Size => "Size",
        Sort::Name => "Name",
        Sort::Newest => "Newest",
    };
    let summary = if let Some(dir) = app.directories.last() {
        format!("Inspecting {}", dir.path.display())
    } else {
        format!(
            "Included  {} ({}) · Sort  {sort}",
            size(app.plan().map(|(_, item)| item.bytes).sum()),
            app.included.len()
        )
    };
    let detail = if app.search || !app.query.is_empty() {
        format!("/ {}{}", app.query, if app.search { "▏" } else { "" })
    } else if app.directories.is_empty() {
        "tab / shift+tab section  •  / search  •  s sort  •  o Explorer".to_owned()
    } else {
        "enter inspect folder  •  o Explorer  •  esc back".to_owned()
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(display(summary), fg(CYAN)),
            Line::styled(display(detail), fg(MUTED)),
            rule(area.width),
        ]),
        parts[0],
    );

    let visible = usize::from(parts[1].height.saturating_sub(1)).max(1);
    app.page = visible;
    let rows = if let Some(dir) = app.directories.last_mut() {
        let start = scroll_for(dir.cursor, &mut dir.scroll, visible);
        if dir.items.is_empty() {
            vec![Row::new([Cell::from("(empty)")])]
        } else {
            dir.items
                .iter()
                .enumerate()
                .skip(start)
                .take(visible)
                .map(|(i, item)| {
                    let active = i == dir.cursor;
                    let label = format!(
                        "{}{}",
                        item.path.file_name().unwrap_or_default().to_string_lossy(),
                        if item.directory { "/" } else { "" }
                    );
                    Row::new([
                        Cell::from(Line::from(vec![
                            cursor(active),
                            name(display(label), active),
                        ])),
                        if item.directory {
                            right("—")
                        } else {
                            sized(item.bytes)
                        },
                        right(age(item)),
                    ])
                })
                .collect()
        }
    } else {
        let start = scroll_for(app.preview_cursor, &mut app.preview_scroll, visible);
        app.rows
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .map(|(i, row)| {
                let active = i == app.preview_cursor;
                match *row {
                    super::Row::Category(index) => {
                        let target = &app.targets[index];
                        let expanded = app.expanded.contains(&index) || !app.query.is_empty();
                        let open = if expanded { "▾ " } else { "▸ " };
                        let label = Line::from(vec![
                            cursor(active),
                            Span::raw(open),
                            dot(target.risk),
                            name(target.name, active),
                        ]);
                        let included = target
                            .items
                            .iter()
                            .filter(|item| app.included.contains(&item.path))
                            .map(|item| item.bytes)
                            .sum();
                        Row::new([Cell::from(label), sized(included), right("")])
                    }
                    super::Row::Item(index, item) => {
                        let item = &app.targets[index].items[item];
                        let label = Line::from(vec![
                            cursor(active),
                            Span::raw("  "),
                            check(app.included.contains(&item.path)),
                            name(
                                shorten_path(
                                    display(item.path.to_string_lossy()),
                                    usize::from(parts[1].width.saturating_sub(30)),
                                ),
                                active,
                            ),
                        ]);
                        Row::new([Cell::from(label), sized(item.bytes), right(age(item))])
                    }
                }
            })
            .collect()
    };
    frame.render_widget(table(rows, "Path", "Age"), parts[1]);
}

fn modal(frame: &mut Frame, width: u16, height: u16, lines: Vec<Line<'_>>) {
    let outer = frame.area();
    let width = width.min(outer.width.saturating_sub(2));
    let height = height.min(outer.height.saturating_sub(2));
    let area = Rect::new(
        (outer.width - width) / 2,
        (outer.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(box_frame())
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn confirm(frame: &mut Frame, app: &App) {
    let total = size(app.plan().map(|(_, item)| item.bytes).sum());
    let cancel = if app.confirm_recycle {
        fg(MUTED)
    } else {
        fg(Color::White).bg(PURPLE)
    };
    let recycle = if app.confirm_recycle {
        fg(Color::White).bg(RED)
    } else {
        fg(RED)
    };
    let categories: Vec<_> = app
        .targets
        .iter()
        .enumerate()
        .filter(|(i, _)| app.selected.contains(i))
        .filter_map(|(_, target)| {
            let items: Vec<_> = target
                .items
                .iter()
                .filter(|i| app.included.contains(&i.path))
                .collect();
            if items.is_empty() {
                return None;
            }
            let bytes = items.iter().map(|item| item.bytes).sum();
            Some(Line::from(vec![
                dot(target.risk),
                Span::raw(format!("{}  ", target.name)),
                Span::styled(size(bytes), fg(CYAN)),
            ]))
        })
        .collect();
    let available = usize::from(frame.area().height.saturating_sub(12)).min(8);
    let start = app
        .confirm_scroll
        .min(categories.len().saturating_sub(available));
    let mut lines = vec![
        title("Confirm Cleanup"),
        Line::default(),
        Line::raw(format!("Recycle {} items ({total})?", app.included.len())),
    ];
    lines.extend(categories.into_iter().skip(start).take(available));
    lines.extend([
        Line::default(),
        Line::from(vec![
            Span::styled(" Cancel ", cancel),
            Span::raw("  "),
            Span::styled(" Recycle ", recycle),
        ]),
        Line::styled(
            "↑/↓ scroll  •  ←/→ choose  •  enter select  •  y recycle",
            fg(MUTED),
        ),
    ]);
    modal(frame, 64, (lines.len() + 2) as u16, lines);
}

fn report(frame: &mut Frame, area: Rect, app: &mut App) {
    let mut lines = vec![
        Line::default(),
        Line::styled(
            format!(
                "Moved {} items ({}) to the Recycle Bin",
                app.report.moved,
                size(app.report.bytes)
            ),
            fg(GREEN),
        ),
    ];
    if !app.report.errors.is_empty() {
        lines.push(Line::styled(
            format!("{} errors · ↑/↓ scroll", app.report.errors.len()),
            fg(YELLOW),
        ));
        lines.extend(
            app.report
                .errors
                .iter()
                .skip(app.report_scroll)
                .take(area.height as usize)
                .map(|error| Line::raw(display(error))),
        );
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn help(frame: &mut Frame, app: &App) {
    let entries: &[&str] = match app.screen {
        Screen::List => &[
            "↑/↓ or k/j    Move",
            "Space        Toggle category",
            "a / d        Select all eligible / clear",
            "Enter / p    Review selection",
            "w            Scan warnings",
            "f            Full rescan after quick scan (clears selection)",
            "q            Quit",
        ],
        Screen::Preview => &[
            "↑/↓ or k/j    Move",
            "Shift+↑/↓    Page (also K/J)",
            "Tab / ]      Next section",
            "Shift+Tab / [  Previous section",
            "←/h · →/l    Collapse / expand",
            "Space        Toggle inclusion",
            "Enter        Inspect a folder",
            "o            Open in Explorer",
            "/ · s        Search / cycle sort",
            "a / d        Include / exclude all shown",
            "y · Esc      Confirm cleanup / back",
            "q            Quit",
        ],
        Screen::Confirm => &[
            "←/→ or Tab   Choose",
            "y / Enter    Recycle / select",
            "n / Esc      Cancel",
        ],
        Screen::Report => &[
            "↑/↓ or k/j   Scroll errors",
            "Enter        Rescan",
            "q / Esc      Quit",
        ],
        Screen::Cleaning => &[],
    };
    let mut lines = vec![title("Keyboard Shortcuts"), Line::default()];
    lines.extend(entries.iter().map(|entry| Line::raw(*entry)));
    lines.push(Line::styled("? / esc close", fg(MUTED)));
    modal(frame, 55, (lines.len() + 2) as u16, lines);
}

fn scan_warnings(frame: &mut Frame, app: &App) {
    let mut lines = vec![
        title("Scan Warnings"),
        Line::styled("↑/↓ scroll  •  w/esc close", fg(MUTED)),
        Line::default(),
    ];
    lines.extend(
        app.targets
            .iter()
            .flat_map(|target| &target.warnings)
            .skip(app.warning_scroll)
            .take(frame.area().height as usize)
            .map(|warning| Line::raw(display(warning))),
    );
    modal(frame, 100, (lines.len() + 2) as u16, lines);
}
