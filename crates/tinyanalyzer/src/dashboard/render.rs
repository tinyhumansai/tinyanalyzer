//! Drawing the dashboard.
//!
//! Every function here is a pure map from [`Dashboard`] to widgets: nothing
//! decides anything, nothing mutates anything except the scroll state a
//! stateful widget needs. All of the decisions were made in
//! [`state`](super::state).
//!
//! The layout is the same in every view — a tab bar, a body, and a status line —
//! so that switching views moves the content and not the furniture.

use super::state::{Dashboard, View};
use crate::summary::{human_bytes, truncate_path};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Bar, BarChart, BarGroup, Block, Borders, Cell, List, ListItem, Paragraph, Row, Table,
    TableState, Wrap,
};
use tinyanalyzer_core::{Finding, Severity};

/// Shared palette. Standard ANSI colors remain legible without assuming a
/// particular terminal background, while the bright variants provide enough
/// contrast for structure and warnings.
const ACCENT: Color = Color::LightCyan;
const DIRECTORY: Color = Color::LightBlue;
const METRIC: Color = Color::Cyan;
const DOCUMENTATION: Color = Color::Green;
const SIZE: Color = Color::LightMagenta;
const MUTED: Color = Color::DarkGray;
const WARNING: Color = Color::Yellow;

/// Product wordmark shown above the overview totals when the terminal has room.
pub(super) const WORDMARK: [&str; 7] = [
    "                                        ▄▄",
    " ██   ▀▀                                ██",
    "▀██▀▀ ██  ████▄ ██ ██  ▀▀█▄ ████▄  ▀▀█▄ ██ ██ ██ ▀▀▀██ ▄█▀█▄ ████▄",
    " ██   ██  ██ ██ ██▄██ ▄█▀██ ██ ██ ▄█▀██ ██ ██▄██   ▄█▀ ██▄█▀ ██ ▀▀",
    " ██   ██▄ ██ ██  ▀██▀ ▀█▄██ ██ ██ ▀█▄██ ██  ▀██▀ ▄██▄▄ ▀█▄▄▄ ██",
    "                  ██                         ██",
    "                ▀▀▀                        ▀▀▀",
];

const MIN_OVERVIEW_WITH_WORDMARK_HEIGHT: u16 = 36;
const WORDMARK_HEIGHT: u16 = 7;

/// Draws the whole dashboard.
pub(super) fn draw(frame: &mut Frame<'_>, dashboard: &Dashboard) {
    let areas = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(frame.area());

    title(frame, areas[0], dashboard);
    tabs(frame, areas[1], dashboard);
    body(frame, areas[2], dashboard);
    status(frame, areas[3], dashboard);
}

/// Returns the row under a mouse coordinate in the active view's primary list.
pub(super) fn row_at(area: Rect, view: View, column: u16, row: u16) -> Option<usize> {
    let body = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area)[2];

    let (list, header) = match view {
        View::Overview => {
            let lower = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(overview_content(body))[1];
            (lower, false)
        }
        View::Files => (
            Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
                .split(body)[0],
            true,
        ),
        View::Directories | View::DeadCode => (body, true),
        View::Dependencies => (
            Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                .split(body)[0],
            true,
        ),
        View::Findings => (
            Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                .split(body)[0],
            false,
        ),
    };

    if column <= list.x || column >= list.right().saturating_sub(1) {
        return None;
    }
    let first = list.y.saturating_add(if header { 2 } else { 1 });
    (row >= first && row < list.bottom().saturating_sub(1))
        .then(|| usize::from(row.saturating_sub(first)))
}

/// Whether a mouse coordinate is inside the active view's detail pane.
pub(super) fn detail_contains(area: Rect, view: View, column: u16, row: u16) -> bool {
    let body = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area)[2];
    let percentage = match view {
        View::Files => 62,
        View::Dependencies | View::Findings => 55,
        View::Overview | View::Directories | View::DeadCode => return false,
    };
    let detail = Layout::horizontal([
        Constraint::Percentage(percentage),
        Constraint::Percentage(100 - percentage),
    ])
    .split(body)[1];

    column >= detail.x && column < detail.right() && row >= detail.y && row < detail.bottom()
}

/// The project name and the headline totals.
fn title(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let totals = dashboard.totals();
    let project = &dashboard.report().project;

    let mut spans = vec![Span::styled(
        format!(" {} ", project.name),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )];
    if dashboard.view() == View::Dependencies {
        spans.extend([
            Span::styled(
                dashboard.direct_dependency_count().to_string(),
                Style::default().fg(METRIC),
            ),
            Span::raw(" direct deps · "),
            Span::styled(
                dashboard
                    .report()
                    .dependencies
                    .external_packages
                    .to_string(),
                Style::default().fg(METRIC),
            ),
            Span::raw(" total deps · "),
            Span::styled(
                human_bytes(dashboard.simulated_build_source_bytes()),
                Style::default().fg(SIZE),
            ),
            Span::raw(" dependency source · "),
            Span::styled(
                dashboard.simulated_build_dependency_count().to_string(),
                Style::default().fg(SIZE),
            ),
            Span::raw(" crates if built"),
        ]);
    } else {
        spans.extend([
            Span::styled(totals.files.to_string(), Style::default().fg(METRIC)),
            Span::raw(" files · "),
            Span::styled(totals.lines.code.to_string(), Style::default().fg(METRIC)),
            Span::raw(" loc · "),
            Span::styled(totals.functions.to_string(), Style::default().fg(METRIC)),
            Span::raw(" functions · "),
            Span::styled(
                totals.external_packages.to_string(),
                Style::default().fg(METRIC),
            ),
            Span::raw(" crates · "),
            Span::styled(human_bytes(totals.bytes), Style::default().fg(SIZE)),
        ]);
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The tab bar.
///
/// Rendered by hand rather than with the `Tabs` widget so the count of rows in
/// each view can sit next to its name; knowing a view is empty before opening it
/// saves a keystroke every time.
fn tabs(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let mut spans = Vec::new();

    for (index, view) in View::ALL.iter().enumerate() {
        let selected = *view == dashboard.view();
        let style = if selected {
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(MUTED)
        };

        spans.push(Span::styled(
            format!(" {}·{} ", index.saturating_add(1), view.title()),
            style,
        ));
        spans.push(Span::raw(" "));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The status line: filters, and the keys that change them.
fn status(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let line = if dashboard.editing_filter() {
        Line::from(vec![
            Span::styled(
                " filter: ",
                Style::default().fg(Color::Black).bg(Color::Yellow),
            ),
            Span::styled(
                format!("{}█", dashboard.filter()),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("  enter to keep · esc to clear", Style::default().fg(MUTED)),
        ])
    } else {
        let tests = if dashboard.hide_tests() {
            Span::styled("tests hidden", Style::default().fg(Color::Yellow))
        } else {
            Span::styled("tests shown", Style::default().fg(MUTED))
        };

        let mut spans = vec![
            Span::styled(" q", Style::default().fg(ACCENT)),
            Span::raw(" quit · "),
            Span::styled("tab/1-6", Style::default().fg(ACCENT)),
            Span::raw(" view · "),
            Span::styled("↑↓", Style::default().fg(ACCENT)),
            Span::raw(" move · "),
            Span::styled("t", Style::default().fg(ACCENT)),
            Span::raw(" tests · "),
            Span::styled("/", Style::default().fg(ACCENT)),
            Span::raw(" filter · "),
            Span::styled("s", Style::default().fg(ACCENT)),
            Span::raw(format!(" sort:{} · ", dashboard.sort_label())),
            Span::styled("i", Style::default().fg(ACCENT)),
            Span::raw(if dashboard.respect_gitignore() {
                " gitignore:on · "
            } else {
                " gitignore:off · "
            }),
            tests,
        ];

        if !dashboard.filter().is_empty() {
            spans.push(Span::styled(
                format!(
                    " · {} “{}”",
                    if dashboard.filter_regex_valid() {
                        "regex"
                    } else {
                        "literal (invalid regex)"
                    },
                    dashboard.filter()
                ),
                Style::default().fg(WARNING),
            ));
        }

        if dashboard.view() == View::Dependencies {
            spans.push(Span::styled(" · d", Style::default().fg(Color::LightRed)));
            spans.push(Span::raw(" toggle mock remove · "));
            spans.push(Span::styled("r", Style::default().fg(Color::LightGreen)));
            spans.push(Span::raw(" restore · "));
            spans.push(Span::styled(
                "[]/f/w",
                Style::default().fg(Color::LightMagenta),
            ));
            spans.push(Span::raw(" features"));
        } else if dashboard.view() == View::Directories {
            spans.push(Span::styled(" · o", Style::default().fg(ACCENT)));
            spans.push(Span::raw(if dashboard.directories_only() {
                " dirs only"
            } else {
                " dirs + files"
            }));
        }

        Line::from(spans)
    };

    frame.render_widget(Paragraph::new(line), area);
}

/// The body of whichever view is open.
fn body(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    match dashboard.view() {
        View::Overview => overview(frame, area, dashboard),
        View::Files => files(frame, area, dashboard),
        View::Directories => directories(frame, area, dashboard),
        View::Dependencies => dependencies(frame, area, dashboard),
        View::DeadCode => dead_code(frame, area, dashboard),
        View::Findings => findings(frame, area, dashboard),
    }
}

/// A bordered block with a title.
fn panel(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(MUTED))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
}

/// The color a severity is drawn in.
const fn severity_color(severity: Severity) -> Color {
    match severity {
        Severity::Critical => Color::LightRed,
        Severity::High => Color::Red,
        Severity::Medium => WARNING,
        Severity::Low => Color::Blue,
    }
}

/// Renders a table with the cursor on `selected`.
fn table(frame: &mut Frame<'_>, area: Rect, table: Table<'_>, selected: usize) {
    let mut state = TableState::default().with_selected(Some(selected));

    frame.render_stateful_widget(
        table.row_highlight_style(
            Style::default()
                .fg(Color::White)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        area,
        &mut state,
    );
}

/// Totals, the language mix, and the top findings.
fn overview(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let content = overview_content(area);
    if content != area {
        let logo = Rect {
            height: WORDMARK_HEIGHT,
            ..area
        };
        wordmark(frame, logo);
    }

    let rows =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(content);
    let top =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).split(rows[0]);

    totals_panel(frame, top[0], dashboard);
    languages_panel(frame, top[1], dashboard);
    findings_list(frame, rows[1], dashboard, "Top findings");
}

/// Leaves room for the wordmark without sacrificing the useful panels in a
/// short or narrow terminal.
fn overview_content(area: Rect) -> Rect {
    let wordmark_width = WORDMARK
        .iter()
        .map(|row| row.chars().count())
        .max()
        .unwrap_or_default();
    let fits = area.height >= MIN_OVERVIEW_WITH_WORDMARK_HEIGHT
        && usize::from(area.width) >= wordmark_width;

    if fits {
        Rect {
            y: area.y.saturating_add(WORDMARK_HEIGHT),
            height: area.height.saturating_sub(WORDMARK_HEIGHT),
            ..area
        }
    } else {
        area
    }
}

/// Draws the product wordmark in the same accent as the rest of the dashboard
/// chrome.
fn wordmark(frame: &mut Frame<'_>, area: Rect) {
    let lines: Vec<Line<'_>> = WORDMARK
        .iter()
        .map(|row| {
            Line::from(Span::styled(
                *row,
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// The numbers, spelled out.
fn totals_panel(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let totals = dashboard.totals();
    let report = dashboard.report();

    let pair = |label: &str, value: String| {
        Line::from(vec![
            Span::styled(format!("{label:<22}"), Style::default().fg(MUTED)),
            Span::styled(
                value,
                Style::default().fg(METRIC).add_modifier(Modifier::BOLD),
            ),
        ])
    };

    let lines = vec![
        pair("files", totals.files.to_string()),
        pair("directories", totals.directories.to_string()),
        pair("lines of code", totals.lines.code.to_string()),
        pair("comment lines", totals.lines.comment.to_string()),
        pair("functions", totals.functions.to_string()),
        pair("items", totals.items.total().to_string()),
        pair("on disk", human_bytes(totals.bytes)),
        pair("test files", report.totals.test_files.to_string()),
        pair("external crates", totals.external_packages.to_string()),
        pair(
            "duplicated crates",
            report.dependencies.duplicates.len().to_string(),
        ),
        pair("unreferenced items", report.dead_code.len().to_string()),
        pair("clones", totals.performance.clones.to_string()),
        pair(
            "allocations in loops",
            totals.performance.allocations_in_loops.to_string(),
        ),
        pair("panic paths", totals.performance.unwraps.to_string()),
    ];

    frame.render_widget(Paragraph::new(lines).block(panel("Totals")), area);
}

/// The language mix as a bar chart.
fn languages_panel(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let bars: Vec<Bar<'_>> = dashboard
        .report()
        .languages
        .iter()
        .take(8)
        .enumerate()
        .map(|(index, language)| {
            const LANGUAGE_COLORS: [Color; 6] = [
                Color::LightBlue,
                Color::LightGreen,
                Color::LightMagenta,
                Color::LightCyan,
                Color::Yellow,
                Color::LightRed,
            ];
            Bar::default()
                .label(Line::from(language.language.label()))
                .value(language.lines.code as u64)
                .style(Style::default().fg(LANGUAGE_COLORS[index % LANGUAGE_COLORS.len()]))
        })
        .collect();

    if bars.is_empty() {
        frame.render_widget(
            Paragraph::new("Nothing analyzed.").block(panel("Languages")),
            area,
        );
        return;
    }

    frame.render_widget(
        BarChart::default()
            .block(panel("Languages by lines of code"))
            .data(BarGroup::default().bars(&bars))
            .bar_width(10)
            .bar_gap(1),
        area,
    );
}

/// Files, ranked.
fn files(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let panes =
        Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)]).split(area);

    let rows: Vec<Row<'_>> = dashboard
        .files()
        .iter()
        .map(|file| {
            let file_lines = dashboard.file_lines(file);
            let complexity: u32 = file
                .rust
                .as_ref()
                .map(|rust| {
                    rust.functions
                        .iter()
                        .filter(|function| !(dashboard.hide_tests() && function.is_test))
                        .map(|function| function.complexity)
                        .sum()
                })
                .unwrap_or_default();
            let functions = dashboard.file_function_count(file);

            let name_style = if file.is_test {
                Style::default().fg(MUTED)
            } else if let Some(severity) = severity_for_path(dashboard, &file.path, false) {
                Style::default()
                    .fg(severity_color(severity))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::LightGreen)
            };
            let name = Span::styled(truncate_path(&file.path, 48), name_style);

            Row::new(vec![
                Cell::from(name),
                metric_cell(&file_lines.code, METRIC),
                metric_cell(&file_lines.comment, DOCUMENTATION),
                metric_cell(&functions, Color::LightMagenta),
                metric_cell(&complexity, if complexity >= 15 { WARNING } else { METRIC }),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(20),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(6),
        Constraint::Length(6),
    ];

    table(
        frame,
        panes[0],
        Table::new(rows, widths)
            .header(header_row(&["file", "code", "docs", "fns", "cx"]))
            .block(panel(&format!("Files ({})", dashboard.row_count()))),
        dashboard.cursor(),
    );

    file_detail(frame, panes[1], dashboard);
}

/// Everything known about the selected file.
fn file_detail(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let Some(file) = dashboard.selected_file() else {
        frame.render_widget(
            Paragraph::new("No file selected.").block(panel("Detail")),
            area,
        );
        return;
    };

    let file_lines = dashboard.file_lines(file);
    let mut lines = vec![
        Line::from(Span::styled(
            file.path.clone(),
            Style::default()
                .fg(severity_for_path(dashboard, &file.path, false)
                    .map_or(Color::LightGreen, severity_color))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(
                "{} · {} · {}{}",
                file.language.label(),
                human_bytes(file.bytes),
                file.crate_name.as_deref().unwrap_or("no crate"),
                if file.is_test { " · test" } else { "" }
            ),
            Style::default().fg(MUTED),
        )),
        Line::from(""),
        Line::from(format!(
            "{} code · {} comment · {} blank",
            file_lines.code, file_lines.comment, file_lines.blank
        )),
    ];

    if let Some(rust) = &file.rust {
        lines.push(Line::from(format!(
            "{} items · {} public · nesting {}",
            rust.items.total(),
            rust.public_items,
            rust.max_nesting
        )));
        lines.push(Line::from(format!(
            "{} clones · {} allocs · {} in loops · {} panics",
            rust.performance.clones,
            rust.performance.allocating_conversions,
            rust.performance.allocations_in_loops,
            rust.performance.unwraps
        )));

        let mut heaviest: Vec<_> = rust
            .functions
            .iter()
            .filter(|function| !(dashboard.hide_tests() && function.is_test))
            .collect();
        heaviest.sort_by(|left, right| {
            right
                .complexity
                .cmp(&left.complexity)
                .then_with(|| right.lines().cmp(&left.lines()))
        });

        if !heaviest.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Heaviest functions",
                Style::default().fg(ACCENT),
            )));
            for function in heaviest.iter().take(8) {
                lines.push(Line::from(format!(
                    "  {:<28} L{:<5} {:>3} lines  cx {}",
                    truncate_path(&function.qualified_name, 28),
                    function.start_line,
                    function.lines(),
                    function.complexity
                )));
            }
        }
    }

    for note in &file.notes {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("note ({}): {}", note.level.label(), note.note),
            Style::default().fg(WARNING),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("Detail"))
            .scroll((dashboard.detail_scroll(), 0))
            .wrap(Wrap { trim: true }),
        area,
    );
}

/// Immediate child directories and files at the current browser level.
fn directories(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let rows: Vec<Row<'_>> = dashboard
        .browser_entries()
        .iter()
        .map(|entry| {
            let is_directory = entry.is_directory();
            let path = entry.path();
            let name = if path == ".." {
                "../".to_owned()
            } else if is_directory {
                format!("{}/", path.rsplit('/').next().unwrap_or("."))
            } else {
                path.rsplit('/').next().unwrap_or(path).to_owned()
            };
            let name_color = severity_for_path(dashboard, path, is_directory).map_or(
                if is_directory {
                    DIRECTORY
                } else {
                    Color::White
                },
                severity_color,
            );
            let lines = entry.lines(dashboard);
            Row::new(vec![
                Cell::from(Span::styled(
                    name,
                    Style::default()
                        .fg(if entry.is_test_only() {
                            MUTED
                        } else {
                            name_color
                        })
                        .add_modifier(if is_directory {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                )),
                metric_cell(&entry.file_count(), Color::LightMagenta),
                metric_cell(&lines.code, METRIC),
                metric_cell(&lines.comment, DOCUMENTATION),
                Cell::from(Span::styled(
                    human_bytes(entry.bytes()),
                    Style::default().fg(SIZE),
                )),
                Cell::from(Span::styled(
                    if entry.is_test_only() { "tests" } else { "" },
                    Style::default().fg(MUTED),
                )),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(20),
        Constraint::Length(7),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(6),
    ];

    table(
        frame,
        area,
        Table::new(rows, widths)
            .header(header_row(&["name", "files", "code", "docs", "size", ""]))
            .block(panel(&format!(
                "Directories · {} ({})",
                dashboard.directory_path(),
                dashboard.row_count()
            ))),
        dashboard.cursor(),
    );
}

/// Direct dependencies and the subtree of the selected one.
fn dependencies(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    if dashboard.report().dependencies.packages.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "No dependency graph.\n\nEither this is not a cargo workspace, cargo could not resolve it, or the pass was turned off with --no-deps.",
            )
            .wrap(Wrap { trim: true })
            .block(panel("Dependencies")),
            area,
        );
        return;
    }

    let panes =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(area);

    let rows: Vec<Row<'_>> = dashboard
        .packages()
        .iter()
        .map(|package| {
            let removed = dashboard.dependency_is_removed(&package.id);
            let (exclusive, reaches) = dashboard.dependency_counts(&package.id);
            Row::new(vec![
                Cell::from(Span::styled(
                    if removed {
                        format!("✕ {}", truncate_path(&package.name, 30))
                    } else {
                        truncate_path(&package.name, 32)
                    },
                    Style::default()
                        .fg(if removed { Color::LightRed } else { DIRECTORY })
                        .add_modifier(Modifier::BOLD),
                )),
                Cell::from(Span::styled(
                    if removed {
                        "mock removed".to_owned()
                    } else {
                        package.version.clone()
                    },
                    Style::default().fg(if removed { Color::LightRed } else { MUTED }),
                )),
                Cell::from(Span::styled(
                    human_bytes(package.source_bytes),
                    Style::default().fg(SIZE),
                )),
                Cell::from(Span::styled(
                    exclusive.to_string(),
                    Style::default().fg(if exclusive >= 20 {
                        Color::LightRed
                    } else {
                        METRIC
                    }),
                )),
                metric_cell(&reaches, Color::LightMagenta),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(16),
        Constraint::Length(12),
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Length(9),
    ];

    table(
        frame,
        panes[0],
        Table::new(rows, widths)
            .header(header_row(&[
                "crate",
                "version",
                "source size",
                "exclusive",
                "reaches",
            ]))
            .block(panel(&if dashboard.removed_dependency_count() == 0 {
                format!("Direct dependencies ({})", dashboard.row_count())
            } else {
                format!(
                    "Simulation · {} removed · {} crates reclaimed",
                    dashboard.removed_dependency_count(),
                    dashboard.simulated_reclaimed_packages()
                )
            })),
        dashboard.cursor(),
    );

    dependency_detail(frame, panes[1], dashboard);
}

/// The subtree beneath the selected dependency.
fn dependency_detail(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let mut simulation = simulation_lines(dashboard);
    let features = feature_lines(dashboard);
    if !simulation.is_empty() && !features.is_empty() {
        simulation.push(Line::from(""));
    }
    simulation.extend(features);
    let Some(package) = dashboard.selected_package() else {
        frame.render_widget(
            Paragraph::new(if simulation.is_empty() {
                vec![Line::from("No dependency selected.")]
            } else {
                simulation
            })
            .block(panel("Graph"))
            .wrap(Wrap { trim: true }),
            area,
        );
        return;
    };

    let mut lines = simulation;
    if !lines.is_empty() {
        lines.push(Line::from(""));
    }
    let (exclusive, reaches) = dashboard.dependency_counts(&package.id);
    lines.extend([
        Line::from(Span::styled(
            format!("{} v{}", package.name, package.version),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(
                "{} crates leave the build with it · {} reachable · depth {}",
                exclusive, reaches, package.depth
            ),
            Style::default().fg(MUTED),
        )),
    ]);

    if !package.features.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("features: {}", package.features.join(", ")),
            Style::default().fg(MUTED),
        )));
    }

    lines.push(Line::from(""));

    let subtree = dashboard.subtree(&package.id, 3);
    if subtree.is_empty() {
        lines.push(Line::from("Depends on nothing else."));
    } else {
        for (depth, child) in subtree.iter().take(60) {
            let child_count = dashboard.dependency_child_count(&child.id);
            let child_label = if child_count == 1 {
                "child dep"
            } else {
                "child deps"
            };
            lines.push(Line::from(vec![
                Span::raw("  ".repeat(depth.saturating_add(1))),
                Span::styled(
                    format!("{} v{}", child.name, child.version),
                    Style::default().fg(DIRECTORY),
                ),
                Span::raw(" · "),
                Span::styled(human_bytes(child.source_bytes), Style::default().fg(SIZE)),
                Span::raw(" · "),
                Span::styled(child_count.to_string(), Style::default().fg(METRIC)),
                Span::raw(format!(" {child_label}")),
            ]));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("Graph"))
            .scroll((dashboard.detail_scroll(), 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// Lines explaining the current dependency-removal simulation.
fn simulation_lines(dashboard: &Dashboard) -> Vec<Line<'static>> {
    if dashboard.removed_dependency_count() == 0 {
        return Vec::new();
    }

    let mut lines = vec![Line::from(Span::styled(
        format!(
            "mock removed: {}",
            dashboard.removed_dependency_names().join(", ")
        ),
        Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD),
    ))];
    let unreachable = dashboard.simulated_unreachable_packages();
    lines.push(Line::from(Span::styled(
        format!("unreachable crates ({})", unreachable.len()),
        Style::default().fg(WARNING),
    )));
    for package in unreachable.iter().take(12) {
        lines.push(Line::from(Span::styled(
            format!("  {} v{}", package.name, package.version),
            Style::default().fg(MUTED),
        )));
    }
    if unreachable.len() > 12 {
        lines.push(Line::from(format!(
            "  … and {} more",
            unreachable.len() - 12
        )));
    }
    lines
}

/// Cargo feature controls for either the selected dependency or workspace root.
fn feature_lines(dashboard: &Dashboard) -> Vec<Line<'static>> {
    let Some(package) = dashboard.feature_target_package() else {
        return Vec::new();
    };
    let features = dashboard.simulated_features();
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "features · {} · {}",
            if dashboard.feature_root_target() {
                if package.is_root_package {
                    "root package"
                } else {
                    "workspace package"
                }
            } else {
                "dependency"
            },
            package.name
        ),
        Style::default()
            .fg(Color::LightMagenta)
            .add_modifier(Modifier::BOLD),
    ))];
    if features.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no declared features",
            Style::default().fg(MUTED),
        )));
        return lines;
    }
    for (index, (feature, enabled)) in features.iter().enumerate().take(12) {
        lines.push(Line::from(vec![
            Span::styled(
                if index == dashboard.feature_cursor() {
                    "> "
                } else {
                    "  "
                },
                Style::default().fg(ACCENT),
            ),
            Span::styled(
                if *enabled { "[x] " } else { "[ ] " },
                Style::default().fg(if *enabled { Color::LightGreen } else { MUTED }),
            ),
            Span::raw((*feature).to_owned()),
        ]));
    }
    lines.push(Line::from(Span::styled(
        "  simulated feature state; rerun analysis to resolve Cargo graph changes",
        Style::default().fg(WARNING),
    )));
    lines
}

/// Unreferenced items.
fn dead_code(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let candidates = dashboard.dead_code();

    if candidates.is_empty() {
        frame.render_widget(
            Paragraph::new("Nothing unreferenced. Every item here has a caller.")
                .block(panel("Dead code")),
            area,
        );
        return;
    }

    let rows: Vec<Row<'_>> = candidates
        .iter()
        .map(|candidate| {
            let confidence = Span::styled(
                candidate.confidence.label().to_owned(),
                Style::default().fg(match candidate.confidence {
                    tinyanalyzer_core::Confidence::High => Color::LightRed,
                    tinyanalyzer_core::Confidence::Medium => Color::Yellow,
                }),
            );

            Row::new(vec![
                Cell::from(confidence),
                Cell::from(Span::styled(
                    candidate.kind.label(),
                    Style::default().fg(Color::LightMagenta),
                )),
                Cell::from(Span::styled(
                    candidate.name.clone(),
                    Style::default().fg(Color::LightGreen),
                )),
                Cell::from(Span::styled(
                    format!("{}:{}", truncate_path(&candidate.file, 40), candidate.line),
                    Style::default().fg(DIRECTORY),
                )),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(9),
        Constraint::Length(11),
        Constraint::Min(16),
        Constraint::Length(46),
    ];

    table(
        frame,
        area,
        Table::new(rows, widths)
            .header(header_row(&["sure", "kind", "name", "where"]))
            .block(panel(&format!(
                "Unreferenced items ({})",
                dashboard.row_count()
            ))),
        dashboard.cursor(),
    );
}

/// Every finding, with the selected one spelled out.
fn findings(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let panes =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(area);

    findings_list(frame, panes[0], dashboard, "Findings");
    finding_detail(frame, panes[1], dashboard);
}

/// The ranked list of findings.
fn findings_list(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard, title: &str) {
    let entries = dashboard.findings();

    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new("Nothing to report.").block(panel(title)),
            area,
        );
        return;
    }

    let items: Vec<ListItem<'_>> = entries
        .iter()
        .map(|finding| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<9}", finding.severity.label()),
                    Style::default().fg(severity_color(finding.severity)),
                ),
                Span::styled(finding.title.clone(), Style::default().fg(Color::Reset)),
            ]))
        })
        .collect();

    let mut state = ratatui::widgets::ListState::default();
    if dashboard.view() == View::Findings {
        state.select(Some(dashboard.cursor()));
    }

    frame.render_stateful_widget(
        List::new(items)
            .block(panel(&format!("{title} ({})", entries.len())))
            .highlight_style(Style::default().fg(Color::White).bg(Color::Blue).bold()),
        area,
        &mut state,
    );
}

/// What the selected finding says, in full.
fn finding_detail(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let Some(finding) = dashboard.selected_finding() else {
        frame.render_widget(
            Paragraph::new("No finding selected.").block(panel("Detail")),
            area,
        );
        return;
    };

    frame.render_widget(
        Paragraph::new(detail_lines(finding))
            .block(panel("Detail"))
            .scroll((dashboard.detail_scroll(), 0))
            .wrap(Wrap { trim: true }),
        area,
    );
}

/// The body of a finding's detail pane.
fn detail_lines(finding: &Finding) -> Vec<Line<'_>> {
    let mut lines = vec![
        Line::from(Span::styled(
            finding.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("{} · {}", finding.severity.label(), finding.rule.id()),
            Style::default().fg(severity_color(finding.severity)),
        )),
    ];

    if let Some(location) = &finding.location {
        lines.push(Line::from(Span::styled(
            match location.line {
                Some(line) => format!("{}:{line}", location.file),
                None => location.file.clone(),
            },
            Style::default().fg(DIRECTORY),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(finding.detail.clone()));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "What to do",
        Style::default().fg(ACCENT),
    )));
    lines.push(Line::from(finding.suggestion.clone()));

    lines
}

/// A styled header row for a table.
fn header_row<'a>(labels: &[&'a str]) -> Row<'a> {
    Row::new(labels.iter().map(|label| Cell::from(*label))).style(
        Style::default()
            .fg(Color::Black)
            .bg(ACCENT)
            .add_modifier(Modifier::BOLD),
    )
}

/// A consistently colored numeric table cell.
fn metric_cell(value: &impl ToString, color: Color) -> Cell<'static> {
    Cell::from(Span::styled(value.to_string(), Style::default().fg(color)))
}

/// Most severe finding attached to a file or anywhere below a directory.
fn severity_for_path(dashboard: &Dashboard, path: &str, directory: bool) -> Option<Severity> {
    dashboard
        .report()
        .findings
        .iter()
        .filter_map(|finding| {
            let file = finding.location.as_ref()?.file.as_str();
            let matches = file == path
                || (directory
                    && file
                        .strip_prefix(path)
                        .is_some_and(|rest| rest.starts_with('/')));
            matches.then_some(finding.severity)
        })
        .min_by_key(|severity| match severity {
            Severity::Critical => 0,
            Severity::High => 1,
            Severity::Medium => 2,
            Severity::Low => 3,
        })
}
