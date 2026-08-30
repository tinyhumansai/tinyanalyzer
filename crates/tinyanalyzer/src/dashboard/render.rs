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
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Bar, BarChart, BarGroup, Block, Borders, Cell, List, ListItem, Paragraph, Row, Table,
    TableState, Wrap,
};
use tinyanalyzer_core::{Finding, Severity};

/// The accent used for headings and the selected tab.
const ACCENT: Color = Color::Cyan;

/// Draws the whole dashboard.
pub fn draw(frame: &mut Frame<'_>, dashboard: &Dashboard) {
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

/// The project name and the headline totals.
fn title(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let totals = dashboard.totals();
    let project = &dashboard.report().project;

    let line = Line::from(vec![
        Span::styled(
            format!(" {} ", project.name),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "{} files · {} loc · {} functions · {} crates · {}",
            totals.files,
            totals.lines.code,
            totals.functions,
            totals.external_packages,
            human_bytes(totals.bytes)
        )),
    ]);

    frame.render_widget(Paragraph::new(line), area);
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
            Style::default().fg(Color::Gray)
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
            Span::styled(" filter: ", Style::default().fg(Color::Black).bg(Color::Yellow)),
            Span::styled(
                format!("{}█", dashboard.filter()),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("  enter to keep · esc to clear", Style::default().fg(Color::DarkGray)),
        ])
    } else {
        let tests = if dashboard.hide_tests() {
            Span::styled("tests hidden", Style::default().fg(Color::Yellow))
        } else {
            Span::styled("tests shown", Style::default().fg(Color::DarkGray))
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
            tests,
        ];

        if !dashboard.filter().is_empty() {
            spans.push(Span::styled(
                format!(" · filter “{}”", dashboard.filter()),
                Style::default().fg(Color::Yellow),
            ));
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
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(ACCENT),
        ))
}

/// The color a severity is drawn in.
const fn severity_color(severity: Severity) -> Color {
    match severity {
        Severity::Critical => Color::Red,
        Severity::High => Color::LightRed,
        Severity::Medium => Color::Yellow,
        Severity::Low => Color::DarkGray,
    }
}

/// Renders a table with the cursor on `selected`.
fn table(frame: &mut Frame<'_>, area: Rect, table: Table<'_>, selected: usize) {
    let mut state = TableState::default().with_selected(Some(selected));

    frame.render_stateful_widget(
        table.row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        area,
        &mut state,
    );
}

/// Totals, the language mix, and the top findings.
fn overview(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let rows = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);
    let top = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(rows[0]);

    totals_panel(frame, top[0], dashboard);
    languages_panel(frame, top[1], dashboard);
    findings_list(frame, rows[1], dashboard, "Top findings");
}

/// The numbers, spelled out.
fn totals_panel(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let totals = dashboard.totals();
    let report = dashboard.report();

    let pair = |label: &str, value: String| {
        Line::from(vec![
            Span::styled(format!("{label:<20}"), Style::default().fg(Color::Gray)),
            Span::styled(value, Style::default().add_modifier(Modifier::BOLD)),
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
        pair("duplicated crates", report.dependencies.duplicates.len().to_string()),
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
        .map(|language| {
            Bar::default()
                .label(Line::from(language.language.label()))
                .value(language.lines.code as u64)
                .style(Style::default().fg(ACCENT))
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
    let panes = Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(area);

    let rows: Vec<Row<'_>> = dashboard
        .files()
        .iter()
        .map(|file| {
            let complexity: u32 = file
                .rust
                .as_ref()
                .map(|rust| rust.functions.iter().map(|function| function.complexity).sum())
                .unwrap_or_default();
            let functions = file.rust.as_ref().map_or(0, |rust| rust.functions.len());

            let name = if file.is_test {
                Span::styled(
                    truncate_path(&file.path, 48),
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::raw(truncate_path(&file.path, 48))
            };

            Row::new(vec![
                Cell::from(name),
                Cell::from(file.lines.code.to_string()),
                Cell::from(file.lines.comment.to_string()),
                Cell::from(functions.to_string()),
                Cell::from(complexity.to_string()),
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

    let mut lines = vec![
        Line::from(Span::styled(
            file.path.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(
                "{} · {} · {}{}",
                file.language.label(),
                human_bytes(file.bytes),
                file.crate_name.as_deref().unwrap_or("no crate"),
                if file.is_test { " · test" } else { "" }
            ),
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
        Line::from(format!(
            "{} code · {} comment · {} blank",
            file.lines.code, file.lines.comment, file.lines.blank
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

        let mut heaviest: Vec<_> = rust.functions.iter().collect();
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
            format!("note ({}): {}", note.level_label(), note.note),
            Style::default().fg(Color::Yellow),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("Detail"))
            .wrap(Wrap { trim: true }),
        area,
    );
}

/// Directories, ranked.
fn directories(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let rows: Vec<Row<'_>> = dashboard
        .directories()
        .iter()
        .map(|directory| {
            Row::new(vec![
                Cell::from(truncate_path(&directory.path, 60)),
                Cell::from(directory.files.to_string()),
                Cell::from(directory.lines.code.to_string()),
                Cell::from(directory.lines.comment.to_string()),
                Cell::from(human_bytes(directory.bytes)),
                Cell::from(if directory.is_test_only { "tests" } else { "" }),
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
            .header(header_row(&["directory", "files", "code", "docs", "size", ""]))
            .block(panel(&format!("Directories ({})", dashboard.row_count()))),
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

    let panes = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    let rows: Vec<Row<'_>> = dashboard
        .packages()
        .iter()
        .map(|package| {
            Row::new(vec![
                Cell::from(truncate_path(&package.name, 32)),
                Cell::from(package.version.clone()),
                Cell::from(Span::styled(
                    package.exclusive_count.to_string(),
                    Style::default().fg(if package.exclusive_count >= 20 {
                        Color::LightRed
                    } else {
                        Color::Reset
                    }),
                )),
                Cell::from(package.transitive_count.to_string()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(16),
        Constraint::Length(12),
        Constraint::Length(11),
        Constraint::Length(9),
    ];

    table(
        frame,
        panes[0],
        Table::new(rows, widths)
            .header(header_row(&["crate", "version", "exclusive", "reaches"]))
            .block(panel(&format!(
                "Direct dependencies ({})",
                dashboard.row_count()
            ))),
        dashboard.cursor(),
    );

    dependency_detail(frame, panes[1], dashboard);
}

/// The subtree beneath the selected dependency.
fn dependency_detail(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let Some(package) = dashboard.selected_package() else {
        frame.render_widget(
            Paragraph::new("No dependency selected.").block(panel("Graph")),
            area,
        );
        return;
    };

    let mut lines = vec![
        Line::from(Span::styled(
            format!("{} v{}", package.name, package.version),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(
                "{} crates leave the build with it · {} reachable · depth {}",
                package.exclusive_count, package.transitive_count, package.depth
            ),
            Style::default().fg(Color::Gray),
        )),
    ];

    if !package.features.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("features: {}", package.features.join(", ")),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));

    let subtree = dashboard.subtree(&package.id, 3);
    if subtree.is_empty() {
        lines.push(Line::from("Depends on nothing else."));
    } else {
        for (depth, child) in subtree.iter().take(60) {
            lines.push(Line::from(format!(
                "{}{} v{}",
                "  ".repeat(depth.saturating_add(1)),
                child.name,
                child.version
            )));
        }
    }

    frame.render_widget(
        Paragraph::new(lines).block(panel("Graph")).wrap(Wrap { trim: false }),
        area,
    );
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
                Cell::from(candidate.kind.label()),
                Cell::from(candidate.name.clone()),
                Cell::from(format!(
                    "{}:{}",
                    truncate_path(&candidate.file, 40),
                    candidate.line
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
            .block(panel(&format!("Unreferenced items ({})", dashboard.row_count()))),
        dashboard.cursor(),
    );
}

/// Every finding, with the selected one spelled out.
fn findings(frame: &mut Frame<'_>, area: Rect, dashboard: &Dashboard) {
    let panes = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

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
                Span::raw(finding.title.clone()),
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
            .highlight_style(Style::default().bg(Color::DarkGray).bold()),
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
            Style::default().fg(Color::Gray),
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
    Row::new(labels.iter().map(|label| Cell::from(*label)))
        .style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD))
}
