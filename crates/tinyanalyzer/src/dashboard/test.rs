//! Unit tests for the dashboard.
//!
//! Two things are tested here, and the split matters. The state machine is
//! tested directly — filters, cursors, view switching — because that is where
//! the behavior is. The renderer is tested through ratatui's `TestBackend`,
//! which draws into an in-memory buffer, so "does every view draw without
//! panicking, at a realistic size and at an absurdly small one" is an assertion
//! rather than something discovered in front of a user.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::action_for;
use super::render;
use super::state::{Action, Dashboard, View};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
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
