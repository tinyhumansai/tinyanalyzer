//! Unit tests for the dashboard.
//!
//! Two things are tested here, and the split matters. The state machine is
//! tested directly — filters, cursors, view switching — because that is where
//! the behavior is. The renderer is tested through ratatui's `TestBackend`,
//! which draws into an in-memory buffer, so "does every view draw without
//! panicking, at a realistic size and at an absurdly small one" is an assertion
//! rather than something discovered in front of a user.
//!
//! The event loop is tested too, by handing it a scripted list of key presses
//! instead of a terminal to read from. Without that it would be the one part of
//! this program nothing ever checked.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::action_for;
use super::render;
use super::state::{Action, Dashboard, View};
use crate::error::{Error, Result};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::path::Path;
use tempfile::TempDir;
use tinyanalyzer_core::{Config, DependencyConfig, Report, StartView, analyze_with};

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the fixture is writable");
    }
    std::fs::write(path, contents).expect("the fixture is writable");
}

fn fixture() -> (TempDir, Report) {
    let root = TempDir::new().expect("a temporary directory for the fixture");
    write(
        root.path(),
        "src/lib.rs",
        &format!(
            "//! docs\npub fn a() {{\n{}}}\n",
            "    let _ = 1;\n".repeat(80)
        ),
    );
    write(root.path(), "src/small.rs", "pub fn unreferenced() {}\n");
    write(root.path(), "tests/api.rs", "#[test]\nfn t() {}\n");
    write(root.path(), "README.md", "# Title\n");

    let config = Config {
        dependencies: DependencyConfig {
            enabled: false,
            ..DependencyConfig::default()
        },
        ..Config::default()
    };
    let report = analyze_with(root.path(), &config).expect("a walkable tree");

    (root, report)
}

fn dashboard() -> (TempDir, Dashboard) {
    let (root, report) = fixture();
    let dashboard = Dashboard::new(report, StartView::Overview, false);

    (root, dashboard)
}

/// A fixture with a resolved dependency graph and an operator note, so the
/// dependency views and the note rendering have something to show.
fn graph_dashboard() -> (TempDir, Dashboard) {
    let root = TempDir::new().expect("a temporary directory for the fixture");

    write(
        root.path(),
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"crates/*\"]\n",
    );
    write(
        root.path(),
        "crates/engine/Cargo.toml",
        "[package]\nname = \"engine\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(
        root.path(),
        "crates/engine/src/lib.rs",
        "//! Engine.\n\n/// Adds.\npub fn add(a: u8, b: u8) -> u8 { a.saturating_add(b) }\n\nfn orphan() {}\n",
    );
    write(
        root.path(),
        "crates/app/Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nengine = { path = \"../engine\" }\n",
    );
    write(
        root.path(),
        "crates/app/src/lib.rs",
        "//! App.\n\n/// Runs.\npub fn run() -> u8 { engine::add(1, 2) }\n",
    );

    let mut config = Config::default();
    config.notes = vec![tinyanalyzer_core::Note {
        path: "crates/engine/src/lib.rs".to_owned(),
        note: "the hot path lives here".to_owned(),
        level: tinyanalyzer_core::NoteLevel::Warning,
    }];

    let mut report = analyze_with(root.path(), &config).expect("a resolvable workspace");

    // A workspace whose only dependencies are path dependencies has no
    // *external* graph, and `heaviest_direct` reports external cost. Rather
    // than make the fixture reach the registry — which would fail on a machine
    // with no network — the external half of the graph is written in directly.
    // These tests are about the views, not about cargo; the resolver itself is
    // exercised in the engine's own integration suite.
    let workspace_root = report
        .dependencies
        .packages
        .iter()
        .find(|package| package.is_workspace_member)
        .map(|package| package.id.clone())
        .expect("the fixture resolves to at least one member");

    report.dependencies.packages.push(external("heavy", "1.2.3", 24, 40, true));
    report.dependencies.packages.push(external("leaf", "0.1.0", 1, 0, true));
    report.dependencies.packages.push(external("deep", "2.0.0", 1, 0, false));
    report.dependencies.edges.extend([
        edge(&workspace_root, "heavy@1.2.3"),
        edge(&workspace_root, "leaf@0.1.0"),
        edge("heavy@1.2.3", "deep@2.0.0"),
    ]);
    report.dependencies.external_packages = 3;

    (root, Dashboard::new(report, StartView::Overview, false))
}

fn external(
    name: &str,
    version: &str,
    exclusive: usize,
    transitive: usize,
    direct: bool,
) -> tinyanalyzer_core::PackageNode {
    tinyanalyzer_core::PackageNode {
        id: format!("{name}@{version}"),
        name: name.to_owned(),
        version: version.to_owned(),
        is_workspace_member: false,
        is_direct: direct,
        kinds: vec![tinyanalyzer_core::DependencyKind::Normal],
        features: vec!["default".to_owned()],
        transitive_count: transitive,
        exclusive_count: exclusive,
        depth: 1,
    }
}

fn edge(from: &str, to: &str) -> tinyanalyzer_core::DependencyEdge {
    tinyanalyzer_core::DependencyEdge {
        from: from.to_owned(),
        to: to.to_owned(),
        kind: tinyanalyzer_core::DependencyKind::Normal,
    }
}

fn rendered(dashboard: &Dashboard) -> String {
    let backend = TestBackend::new(180, 50);
    let mut terminal = Terminal::new(backend).expect("an in-memory terminal");

    terminal
        .draw(|frame| render::draw(frame, dashboard))
        .expect("the view draws");

    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn it_opens_on_the_configured_view() {
    let (_root, report) = fixture();

    let dashboard = Dashboard::new(report, StartView::Findings, false);

    assert_eq!(dashboard.view(), View::Findings);
    assert!(!dashboard.should_quit());
}

#[test]
fn every_view_has_a_title_and_a_stable_position() {
    for (index, view) in View::ALL.iter().enumerate() {
        assert!(!view.title().is_empty());
        assert_eq!(view.index(), index);
        assert_eq!(View::from_index(index), *view);
    }
}

#[test]
fn view_indexes_wrap_rather_than_panicking() {
    assert_eq!(View::from_index(View::ALL.len()), View::Overview);
}

#[test]
fn every_start_view_maps_onto_a_pane() {
    assert_eq!(View::from_start(StartView::Overview), View::Overview);
    assert_eq!(View::from_start(StartView::Files), View::Files);
    assert_eq!(
        View::from_start(StartView::Dependencies),
        View::Dependencies
    );
    assert_eq!(View::from_start(StartView::DeadCode), View::DeadCode);
    assert_eq!(View::from_start(StartView::Findings), View::Findings);
}

#[test]
fn quitting_sets_the_flag_the_loop_reads() {
    let (_root, mut dashboard) = dashboard();

    dashboard.apply(Action::Quit);

    assert!(dashboard.should_quit());
}

#[test]
fn views_cycle_in_both_directions() {
    let (_root, mut dashboard) = dashboard();

    dashboard.apply(Action::NextView);
    assert_eq!(dashboard.view(), View::Files);

    dashboard.apply(Action::PreviousView);
    assert_eq!(dashboard.view(), View::Overview);

    dashboard.apply(Action::PreviousView);
    assert_eq!(
        dashboard.view(),
        View::Findings,
        "moving back from the first view wraps to the last"
    );
}

#[test]
fn a_view_can_be_selected_by_position() {
    let (_root, mut dashboard) = dashboard();

    dashboard.apply(Action::SelectView(2));
    assert_eq!(dashboard.view(), View::Directories);

    dashboard.apply(Action::SelectView(99));
    assert_eq!(
        dashboard.view(),
        View::Directories,
        "a position past the end changes nothing"
    );
}

#[test]
fn the_cursor_moves_and_stops_at_both_ends() {
    let (_root, mut dashboard) = dashboard();
    dashboard.apply(Action::SelectView(View::Files.index()));

    assert_eq!(dashboard.cursor(), 0);

    dashboard.apply(Action::MoveUp);
    assert_eq!(dashboard.cursor(), 0, "it does not go negative");

    dashboard.apply(Action::MoveDown);
    assert_eq!(dashboard.cursor(), 1);

    dashboard.apply(Action::Last);
    assert_eq!(dashboard.cursor(), dashboard.row_count() - 1);

    dashboard.apply(Action::MoveDown);
    assert_eq!(
        dashboard.cursor(),
        dashboard.row_count() - 1,
        "it does not go past the end"
    );

    dashboard.apply(Action::First);
    assert_eq!(dashboard.cursor(), 0);
}

#[test]
fn paging_moves_further_than_a_single_step() {
    let (_root, mut dashboard) = dashboard();
    dashboard.apply(Action::SelectView(View::Files.index()));

    dashboard.apply(Action::PageDown);
    let paged = dashboard.cursor();
    dashboard.apply(Action::PageUp);

    assert_eq!(dashboard.cursor(), 0);
    assert!(paged > 0);
}

#[test]
fn each_view_keeps_its_own_cursor() {
    let (_root, mut dashboard) = dashboard();

    dashboard.apply(Action::SelectView(View::Files.index()));
    dashboard.apply(Action::MoveDown);
    let files_cursor = dashboard.cursor();

    dashboard.apply(Action::SelectView(View::Findings.index()));
    assert_eq!(dashboard.cursor(), 0);

    dashboard.apply(Action::SelectView(View::Files.index()));
    assert_eq!(dashboard.cursor(), files_cursor);
}

#[test]
fn hiding_tests_removes_them_from_the_rows_and_the_totals() {
    let (_root, mut dashboard) = dashboard();
    dashboard.apply(Action::SelectView(View::Files.index()));

    let with_tests = dashboard.row_count();
    let total_lines = dashboard.totals().lines.total;
    assert!(
        dashboard.files().iter().any(|file| file.is_test),
        "the fixture has a test file"
    );

    dashboard.apply(Action::ToggleTests);

    assert!(dashboard.hide_tests());
    assert!(dashboard.row_count() < with_tests);
    assert!(dashboard.totals().lines.total < total_lines);
    assert!(!dashboard.files().iter().any(|file| file.is_test));
}

#[test]
fn toggling_tests_twice_restores_the_original_view() {
    let (_root, mut dashboard) = dashboard();
    dashboard.apply(Action::SelectView(View::Files.index()));
    let before = dashboard.row_count();

    dashboard.apply(Action::ToggleTests);
    dashboard.apply(Action::ToggleTests);

    assert_eq!(dashboard.row_count(), before);
}

#[test]
fn a_filter_narrows_the_rows_and_is_case_insensitive() {
    let (_root, mut dashboard) = dashboard();
    dashboard.apply(Action::SelectView(View::Files.index()));

    dashboard.apply(Action::StartFilter);
    assert!(dashboard.editing_filter());

    for character in "SMALL".chars() {
        dashboard.apply(Action::FilterPush(character));
    }
    dashboard.apply(Action::CommitFilter);

    assert!(!dashboard.editing_filter());
    assert_eq!(dashboard.filter(), "SMALL");
    assert_eq!(dashboard.row_count(), 1);
    assert_eq!(
        dashboard.selected_file().map(|file| file.path.as_str()),
        Some("src/small.rs")
    );
}

#[test]
fn backspace_widens_the_filter_again() {
    let (_root, mut dashboard) = dashboard();
    dashboard.apply(Action::SelectView(View::Files.index()));
    let unfiltered = dashboard.row_count();

    dashboard.apply(Action::StartFilter);
    dashboard.apply(Action::FilterPush('s'));
    dashboard.apply(Action::FilterPush('m'));
    dashboard.apply(Action::FilterPop);
    dashboard.apply(Action::FilterPop);

    assert_eq!(dashboard.filter(), "");
    assert_eq!(dashboard.row_count(), unfiltered);
}

#[test]
fn cancelling_a_filter_discards_it() {
    let (_root, mut dashboard) = dashboard();
    dashboard.apply(Action::SelectView(View::Files.index()));
    let unfiltered = dashboard.row_count();

    dashboard.apply(Action::StartFilter);
    dashboard.apply(Action::FilterPush('z'));
    dashboard.apply(Action::CancelFilter);

    assert!(!dashboard.editing_filter());
    assert!(dashboard.filter().is_empty());
    assert_eq!(dashboard.row_count(), unfiltered);
}

#[test]
fn a_filter_that_shrinks_the_list_pulls_the_cursor_back_into_range() {
    let (_root, mut dashboard) = dashboard();
    dashboard.apply(Action::SelectView(View::Files.index()));
    dashboard.apply(Action::Last);
    assert!(dashboard.cursor() > 0);

    dashboard.apply(Action::StartFilter);
    for character in "small".chars() {
        dashboard.apply(Action::FilterPush(character));
    }

    assert_eq!(dashboard.cursor(), 0);
    assert!(
        dashboard.selected_file().is_some(),
        "the cursor must still point at a row"
    );
}

#[test]
fn a_filter_matching_nothing_leaves_no_selection_and_does_not_panic() {
    let (_root, mut dashboard) = dashboard();
    dashboard.apply(Action::SelectView(View::Files.index()));

    dashboard.apply(Action::StartFilter);
    for character in "zzzznothing".chars() {
        dashboard.apply(Action::FilterPush(character));
    }

    assert_eq!(dashboard.row_count(), 0);
    assert!(dashboard.selected_file().is_none());
    assert_eq!(dashboard.cursor(), 0);
}

#[test]
fn dead_code_rows_match_on_the_name_or_the_file() {
    let (_root, mut dashboard) = dashboard();
    dashboard.apply(Action::SelectView(View::DeadCode.index()));
    assert!(dashboard.row_count() > 0, "the fixture has an orphan");

    dashboard.apply(Action::StartFilter);
    for character in "unreferenced".chars() {
        dashboard.apply(Action::FilterPush(character));
    }

    assert!(
        dashboard
            .dead_code()
            .iter()
            .any(|item| item.name == "unreferenced")
    );
}

#[test]
fn findings_can_be_filtered_by_rule_identifier() {
    let (_root, mut dashboard) = dashboard();
    dashboard.apply(Action::SelectView(View::Findings.index()));
    assert!(dashboard.row_count() > 0, "the fixture provokes findings");

    dashboard.apply(Action::StartFilter);
    for character in "long_function".chars() {
        dashboard.apply(Action::FilterPush(character));
    }

    assert!(
        dashboard
            .findings()
            .iter()
            .all(|finding| finding.rule.id() == "long_function")
    );
}

#[test]
fn a_subtree_of_an_empty_graph_is_empty() {
    let (_root, dashboard) = dashboard();

    assert!(dashboard.subtree("anything", 3).is_empty());
}

#[test]
fn keys_map_onto_the_actions_they_describe() {
    assert_eq!(
        action_for(key(KeyCode::Char('q')), false),
        Some(Action::Quit)
    );
    assert_eq!(action_for(key(KeyCode::Esc), false), Some(Action::Quit));
    assert_eq!(action_for(key(KeyCode::Tab), false), Some(Action::NextView));
    assert_eq!(
        action_for(key(KeyCode::BackTab), false),
        Some(Action::PreviousView)
    );
    assert_eq!(
        action_for(key(KeyCode::Down), false),
        Some(Action::MoveDown)
    );
    assert_eq!(
        action_for(key(KeyCode::Char('j')), false),
        Some(Action::MoveDown)
    );
    assert_eq!(action_for(key(KeyCode::Up), false), Some(Action::MoveUp));
    assert_eq!(
        action_for(key(KeyCode::Char('t')), false),
        Some(Action::ToggleTests)
    );
    assert_eq!(
        action_for(key(KeyCode::Char('/')), false),
        Some(Action::StartFilter)
    );
    assert_eq!(
        action_for(key(KeyCode::Char('3')), false),
        Some(Action::SelectView(2))
    );
    assert_eq!(action_for(key(KeyCode::Char('%')), false), None);
}

#[test]
fn control_c_quits_from_either_mode() {
    let interrupt = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

    assert_eq!(action_for(interrupt, false), Some(Action::Quit));
    assert_eq!(action_for(interrupt, true), Some(Action::Quit));
}

#[test]
fn while_filtering_ordinary_keys_are_characters_rather_than_commands() {
    assert_eq!(
        action_for(key(KeyCode::Char('q')), true),
        Some(Action::FilterPush('q')),
        "a path containing a q must not close the dashboard"
    );
    assert_eq!(
        action_for(key(KeyCode::Char('t')), true),
        Some(Action::FilterPush('t'))
    );
    assert_eq!(
        action_for(key(KeyCode::Esc), true),
        Some(Action::CancelFilter)
    );
    assert_eq!(
        action_for(key(KeyCode::Enter), true),
        Some(Action::CommitFilter)
    );
    assert_eq!(
        action_for(key(KeyCode::Backspace), true),
        Some(Action::FilterPop)
    );
    assert_eq!(action_for(key(KeyCode::Down), true), None);
}

#[test]
fn a_key_release_does_nothing() {
    let release = KeyEvent::new_with_kind(
        KeyCode::Char('q'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    );

    assert_eq!(
        action_for(release, false),
        None,
        "terminals that report both press and release must not act twice"
    );
}

#[test]
fn every_view_draws_at_a_realistic_size() {
    let (_root, mut dashboard) = dashboard();

    for index in 0..View::ALL.len() {
        dashboard.apply(Action::SelectView(index));

        let backend = TestBackend::new(160, 48);
        let mut terminal = Terminal::new(backend).expect("an in-memory terminal");

        terminal
            .draw(|frame| render::draw(frame, &dashboard))
            .expect("every view draws");
    }
}

#[test]
fn every_view_draws_in_a_terminal_far_too_small_for_it() {
    let (_root, mut dashboard) = dashboard();

    for index in 0..View::ALL.len() {
        dashboard.apply(Action::SelectView(index));

        let backend = TestBackend::new(12, 6);
        let mut terminal = Terminal::new(backend).expect("an in-memory terminal");

        terminal
            .draw(|frame| render::draw(frame, &dashboard))
            .expect("a cramped terminal must clip, not panic");
    }
}

#[test]
fn every_view_draws_with_nothing_to_show() {
    let root = TempDir::new().expect("a temporary directory");
    let config = Config {
        dependencies: DependencyConfig {
            enabled: false,
            ..DependencyConfig::default()
        },
        ..Config::default()
    };
    let report = analyze_with(root.path(), &config).expect("an empty tree");
    let mut dashboard = Dashboard::new(report, StartView::Overview, false);

    for index in 0..View::ALL.len() {
        dashboard.apply(Action::SelectView(index));

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("an in-memory terminal");

        terminal
            .draw(|frame| render::draw(frame, &dashboard))
            .expect("an empty report must still draw");
    }
}

#[test]
fn the_rendered_dashboard_shows_the_project_and_the_tab_bar() {
    let (_root, dashboard) = dashboard();
    let backend = TestBackend::new(160, 48);
    let mut terminal = Terminal::new(backend).expect("an in-memory terminal");

    terminal
        .draw(|frame| render::draw(frame, &dashboard))
        .expect("the overview draws");

    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();

    assert!(rendered.contains(&dashboard.report().project.name));
    assert!(rendered.contains("Overview"));
    assert!(rendered.contains("Findings"));
    assert!(rendered.contains("quit"));
}

#[test]
fn the_filter_prompt_appears_while_typing() {
    let (_root, mut dashboard) = dashboard();
    dashboard.apply(Action::StartFilter);
    dashboard.apply(Action::FilterPush('x'));

    let backend = TestBackend::new(160, 48);
    let mut terminal = Terminal::new(backend).expect("an in-memory terminal");
    terminal
        .draw(|frame| render::draw(frame, &dashboard))
        .expect("the overview draws");

    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();

    assert!(rendered.contains("filter:"));
}


/// Drives the loop with a scripted list of key presses.
///
/// The last event must quit, or the loop would never return. A script that ran
/// out is a test that would hang, so it is turned into a terminal error rather
/// than being allowed to block the suite.
fn drive_with(dashboard: &mut Dashboard, keys: &[KeyEvent]) -> Result<()> {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("an in-memory terminal");

    let mut remaining = keys.iter().copied();
    let mut next = move || -> Result<Event> {
        remaining.next().map(Event::Key).ok_or_else(|| Error::Terminal {
            source: std::io::Error::other("the scripted events ran out"),
        })
    };

    super::drive(&mut terminal, dashboard, &mut next)
}

#[test]
fn the_loop_draws_applies_events_and_returns_when_asked_to_quit() {
    let (_root, mut dashboard) = dashboard();

    drive_with(
        &mut dashboard,
        &[
            key(KeyCode::Char('2')),
            key(KeyCode::Down),
            key(KeyCode::Char('t')),
            key(KeyCode::Char('q')),
        ],
    )
    .expect("the script quits, so the loop returns");

    assert_eq!(dashboard.view(), View::Files);
    assert!(dashboard.hide_tests());
    assert!(dashboard.should_quit());
}

#[test]
fn the_loop_returns_the_error_when_the_event_source_fails() {
    let (_root, mut dashboard) = dashboard();

    let error = drive_with(&mut dashboard, &[]).expect_err("the script is empty");

    assert!(matches!(error, Error::Terminal { .. }));
    assert!(!dashboard.should_quit());
}

#[test]
fn the_loop_ignores_an_event_that_means_nothing_here() {
    let (_root, mut dashboard) = dashboard();

    drive_with(
        &mut dashboard,
        &[key(KeyCode::Char('%')), key(KeyCode::F(4)), key(KeyCode::Char('q'))],
    )
    .expect("an unmapped key is not an error");

    assert_eq!(dashboard.view(), View::Overview);
}

#[test]
fn a_dashboard_that_is_already_quitting_draws_nothing() {
    let (_root, mut dashboard) = dashboard();
    dashboard.apply(Action::Quit);

    drive_with(&mut dashboard, &[]).expect("the loop returns without reading an event");
}

#[test]
fn a_filter_typed_through_the_loop_narrows_the_rows() {
    let (_root, mut dashboard) = dashboard();

    drive_with(
        &mut dashboard,
        &[
            key(KeyCode::Char('2')),
            key(KeyCode::Char('/')),
            key(KeyCode::Char('s')),
            key(KeyCode::Char('m')),
            key(KeyCode::Enter),
            key(KeyCode::Char('q')),
        ],
    )
    .expect("the script quits");

    assert_eq!(dashboard.filter(), "sm");
    assert!(!dashboard.editing_filter());
    assert!(dashboard.row_count() < dashboard.report().files.len());
}


#[test]
fn the_dependency_view_ranks_direct_dependencies_and_shows_the_subtree() {
    let (_root, mut dashboard) = graph_dashboard();
    dashboard.apply(Action::SelectView(View::Dependencies.index()));

    assert!(dashboard.row_count() > 0, "the fixture has direct dependencies");

    let selected = dashboard
        .selected_package()
        .expect("the cursor is on a package");
    assert!(!selected.name.is_empty());

    assert_eq!(
        selected.name, "heavy",
        "the heaviest direct dependency comes first"
    );

    let text = rendered(&dashboard);
    assert!(text.contains("Direct dependencies"));
    assert!(text.contains("heavy"));
    assert!(text.contains("exclusive"));
    assert!(text.contains("features: default"));
    assert!(text.contains("deep"), "the subtree is drawn beneath it");
}

#[test]
fn a_dependency_that_reaches_nothing_says_so() {
    let (_root, mut dashboard) = graph_dashboard();
    dashboard.apply(Action::SelectView(View::Dependencies.index()));
    dashboard.apply(Action::MoveDown);

    assert_eq!(
        dashboard.selected_package().map(|package| package.name.as_str()),
        Some("leaf")
    );
    assert!(rendered(&dashboard).contains("Depends on nothing else."));
}

#[test]
fn a_subtree_walks_the_resolved_graph() {
    let (_root, mut dashboard) = graph_dashboard();
    dashboard.apply(Action::SelectView(View::Dependencies.index()));

    let members: Vec<&str> = dashboard
        .report()
        .dependencies
        .packages
        .iter()
        .filter(|package| package.is_workspace_member)
        .map(|package| package.id.as_str())
        .collect();
    assert!(!members.is_empty());

    let reached: usize = members
        .iter()
        .map(|id| dashboard.subtree(id, 3).len())
        .sum();

    assert!(reached > 0, "the workspace reaches its dependencies");

    let names: Vec<&str> = members
        .iter()
        .flat_map(|id| dashboard.subtree(id, 3))
        .map(|(_, package)| package.name.as_str())
        .collect();
    assert!(names.contains(&"heavy"));
    assert!(names.contains(&"deep"), "the walk goes deeper than one level");
}

#[test]
fn a_subtree_depth_limit_is_honored() {
    let (_root, dashboard) = graph_dashboard();
    let root_id = dashboard
        .report()
        .dependencies
        .packages
        .iter()
        .find(|package| package.is_workspace_member)
        .map(|package| package.id.clone())
        .expect("the fixture resolves to at least one member");

    let deep = dashboard.subtree(&root_id, 3);
    let shallow = dashboard.subtree(&root_id, 0);

    assert!(shallow.len() <= deep.len());
    assert!(shallow.iter().all(|(depth, _)| *depth == 0));
}

#[test]
fn the_dependency_view_can_be_filtered_to_nothing_and_still_draws() {
    let (_root, mut dashboard) = graph_dashboard();
    dashboard.apply(Action::SelectView(View::Dependencies.index()));
    dashboard.apply(Action::StartFilter);
    for character in "zzznothing".chars() {
        dashboard.apply(Action::FilterPush(character));
    }

    assert_eq!(dashboard.row_count(), 0);
    assert!(dashboard.selected_package().is_none());
    assert!(rendered(&dashboard).contains("No dependency selected."));
}

#[test]
fn the_dead_code_view_lists_candidates_with_their_confidence() {
    let (_root, mut dashboard) = graph_dashboard();
    dashboard.apply(Action::SelectView(View::DeadCode.index()));

    assert!(dashboard.row_count() > 0, "the fixture has an orphan");

    let text = rendered(&dashboard);
    assert!(text.contains("Unreferenced items"));
    assert!(text.contains("orphan"));
}

#[test]
fn an_operator_note_is_shown_on_the_file_it_is_about() {
    let (_root, mut dashboard) = graph_dashboard();
    dashboard.apply(Action::SelectView(View::Files.index()));

    let position = dashboard
        .files()
        .iter()
        .position(|file| file.path == "crates/engine/src/lib.rs")
        .expect("the fixture writes it");
    for _ in 0..position {
        dashboard.apply(Action::MoveDown);
    }

    let text = rendered(&dashboard);
    assert!(text.contains("the hot path lives here"));
    assert!(text.contains("warning"));
}

#[test]
fn the_file_detail_names_the_owning_crate_and_the_heaviest_functions() {
    let (_root, mut dashboard) = graph_dashboard();
    dashboard.apply(Action::SelectView(View::Files.index()));

    let position = dashboard
        .files()
        .iter()
        .position(|file| file.path == "crates/engine/src/lib.rs")
        .expect("the fixture writes it");
    for _ in 0..position {
        dashboard.apply(Action::MoveDown);
    }

    let text = rendered(&dashboard);

    assert!(text.contains("Heaviest functions"));
    assert!(text.contains("engine"), "the owning crate is named");
    assert!(text.contains("items"));
}

#[test]
fn the_directories_view_lists_directories_with_their_sizes() {
    let (_root, mut dashboard) = graph_dashboard();
    dashboard.apply(Action::SelectView(View::Directories.index()));

    assert!(dashboard.row_count() > 0);
    assert!(rendered(&dashboard).contains("Directories"));
}

#[test]
fn the_findings_view_spells_out_the_selected_finding() {
    let (_root, mut dashboard) = graph_dashboard();
    dashboard.apply(Action::SelectView(View::Findings.index()));

    let finding = dashboard
        .selected_finding()
        .expect("the fixture provokes findings");
    let text = rendered(&dashboard);

    assert!(text.contains("What to do"));
    assert!(text.contains(finding.rule.id()));
}

#[test]
fn a_filter_that_empties_the_findings_view_still_draws() {
    let (_root, mut dashboard) = graph_dashboard();
    dashboard.apply(Action::SelectView(View::Findings.index()));
    dashboard.apply(Action::StartFilter);
    for character in "zzznothing".chars() {
        dashboard.apply(Action::FilterPush(character));
    }

    assert!(dashboard.selected_finding().is_none());
    assert!(rendered(&dashboard).contains("Nothing to report."));
}

#[test]
fn hiding_tests_removes_a_test_only_directory_from_the_list() {
    let (_root, mut dashboard) = dashboard();
    dashboard.apply(Action::SelectView(View::Directories.index()));
    let shown = dashboard.row_count();

    dashboard.apply(Action::ToggleTests);

    assert!(dashboard.directories().iter().all(|entry| !entry.is_test_only));
    assert!(dashboard.row_count() <= shown);
}

#[test]
fn every_view_of_a_report_with_a_graph_draws() {
    let (_root, mut dashboard) = graph_dashboard();

    for index in 0..View::ALL.len() {
        dashboard.apply(Action::SelectView(index));
        assert!(!rendered(&dashboard).is_empty());
    }
}
