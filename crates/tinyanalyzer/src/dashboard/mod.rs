//! The interactive terminal dashboard.
//!
//! This module owns the two things that cannot be tested without a terminal —
//! the raw-mode lifecycle and the event loop — and nothing else. What a
//! keystroke *means* is [`action_for`], a pure function; what it *does* is
//! [`state::Dashboard::apply`], a pure state transition; what the result looks
//! like is the renderer, a pure map to widgets. The loop below just moves
//! values between them.
//!
//! The terminal is restored on every exit path, including a panic: leaving a
//! user's shell in raw mode with the cursor hidden is the one failure this
//! program could cause that outlives the program.

mod render;
mod state;

pub use state::{Action, Dashboard, View};

use crate::error::{Error, Result};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use tinyanalyzer_core::{Report, StartView};

/// Opens the dashboard on `report` and runs until the operator leaves.
///
/// # Errors
///
/// Returns [`Error::Terminal`] if the terminal cannot be put into raw mode,
/// read from, or restored.
pub fn run(report: Report, start: StartView, hide_tests: bool) -> Result<()> {
    run_inner(report, start, hide_tests, true, None)
}

/// Opens a dashboard that can rebuild its report when ignore handling changes.
///
/// `reload` receives the new `respect_gitignore` value whenever the operator
/// presses `i`.
///
/// # Errors
///
/// Returns [`Error::Terminal`] for terminal failures, or forwards an analysis
/// error returned by `reload`.
pub fn run_with_reload(
    report: Report,
    start: StartView,
    hide_tests: bool,
    respect_gitignore: bool,
    reload: &mut dyn FnMut(bool) -> Result<Report>,
) -> Result<()> {
    run_inner(report, start, hide_tests, respect_gitignore, Some(reload))
}

fn run_inner(
    report: Report,
    start: StartView,
    hide_tests: bool,
    respect_gitignore: bool,
    mut reload: Option<&mut dyn FnMut(bool) -> Result<Report>>,
) -> Result<()> {
    let mut dashboard = Dashboard::new(report, start, hide_tests);
    dashboard.set_respect_gitignore(respect_gitignore);
    let mut terminal = ratatui::try_init().map_err(|source| Error::Terminal { source })?;

    let mouse = MouseCapture::enable().inspect_err(|_| {
        let _ = ratatui::try_restore();
    })?;

    let outcome = loop {
        if let Err(error) = drive(&mut terminal, &mut dashboard, &mut read_event) {
            break Err(error);
        }
        if dashboard.should_quit() {
            break Ok(());
        }
        let Some(respect) = dashboard.take_reload_request() else {
            break Ok(());
        };
        let Some(callback) = reload.as_deref_mut() else {
            break Ok(());
        };
        match callback(respect) {
            Ok(report) => dashboard.replace_report(report),
            Err(error) => break Err(error),
        }
    };

    // Restored before the outcome is inspected: a failure inside the loop must
    // not leave the terminal in raw mode on the way out.
    let mouse_restored = mouse.disable();
    let restored = ratatui::try_restore().map_err(|source| Error::Terminal { source });

    outcome.and(mouse_restored).and(restored)
}

/// Enables mouse reporting and disables it again on every exit path.
///
/// The explicit [`Self::disable`] call reports restoration failures. The drop
/// fallback exists for unwinding, when returning an error is no longer
/// possible but leaving the user's terminal in mouse-reporting mode would be
/// worse than losing the error.
struct MouseCapture {
    active: bool,
}

impl MouseCapture {
    /// Asks crossterm to begin reporting mouse events.
    fn enable() -> Result<Self> {
        ratatui::crossterm::execute!(std::io::stdout(), EnableMouseCapture)
            .map_err(|source| Error::Terminal { source })?;
        Ok(Self { active: true })
    }

    /// Stops mouse reporting, exposing a terminal failure to the caller.
    fn disable(mut self) -> Result<()> {
        self.active = false;
        ratatui::crossterm::execute!(std::io::stdout(), DisableMouseCapture)
            .map_err(|source| Error::Terminal { source })
    }
}

impl Drop for MouseCapture {
    fn drop(&mut self) {
        if self.active {
            let _ = ratatui::crossterm::execute!(std::io::stdout(), DisableMouseCapture);
        }
    }
}

/// Blocks until the terminal reports an event.
///
/// # Errors
///
/// Returns [`Error::Terminal`] if the terminal cannot be read.
fn read_event() -> Result<Event> {
    event::read().map_err(|source| Error::Terminal { source })
}

/// Draws and applies events until the dashboard is asked to close.
///
/// The event source is a parameter rather than a call to `event::read`, which
/// is what makes the loop testable: a test drives it with a scripted list of
/// key presses against an in-memory terminal, and asserts on the state that
/// comes out. A loop that could only be exercised by a human at a real
/// terminal would be the one part of this program nothing ever checked.
///
/// # Errors
///
/// Returns [`Error::Terminal`] if drawing or reading fails.
pub(crate) fn drive<B>(
    terminal: &mut Terminal<B>,
    dashboard: &mut Dashboard,
    events: &mut dyn FnMut() -> Result<Event>,
) -> Result<()>
where
    B: Backend,
    // Every real backend fails with an `io::Error`; the in-memory one the tests
    // draw into cannot fail at all. Naming the bound this way admits both
    // without the loop knowing which it is holding.
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let mut area = Rect::default();
    while !dashboard.should_quit() && !dashboard.reload_requested() {
        terminal
            .draw(|frame| {
                area = frame.area();
                render::draw(frame, dashboard);
            })
            .map_err(|source| Error::Terminal {
                source: std::io::Error::other(source),
            })?;

        if let Some(action) = action_for_event(&events()?, area, dashboard) {
            dashboard.apply(action);
        }
    }

    Ok(())
}

/// Maps a terminal event to a dashboard action.
fn action_for_event(event: &Event, area: Rect, dashboard: &Dashboard) -> Option<Action> {
    match event {
        Event::Key(key)
            if dashboard.view() == View::Dependencies && !dashboard.editing_filter() =>
        {
            match key.code {
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    Some(Action::EnterDependency)
                }
                KeyCode::Esc if !dashboard.dependency_at_root() => Some(Action::LeaveDependency),
                KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                    Some(Action::LeaveDependency)
                }
                KeyCode::Char('d') if dashboard.dependency_at_root() => {
                    Some(Action::SimulateRemoveDependency)
                }
                KeyCode::Char('r') => Some(Action::RestoreDependencies),
                KeyCode::Char('f') => Some(Action::ToggleFeature),
                KeyCode::Char('[') => Some(Action::PreviousFeature),
                KeyCode::Char(']') => Some(Action::NextFeature),
                KeyCode::Char('w') => Some(Action::ToggleFeatureTarget),
                _ => action_for(*key, false),
            }
        }
        Event::Key(key) if dashboard.view() == View::Directories && !dashboard.editing_filter() => {
            match key.code {
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    Some(Action::EnterDirectory)
                }
                KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                    Some(Action::LeaveDirectory)
                }
                KeyCode::Char('o') => Some(Action::ToggleDirectoriesOnly),
                _ => action_for(*key, false),
            }
        }
        Event::Key(key) => action_for(*key, dashboard.editing_filter()),
        Event::Mouse(mouse) if !dashboard.editing_filter() => {
            action_for_mouse(*mouse, area, dashboard)
        }
        _ => None,
    }
}

/// Maps mouse coordinates onto tabs and the active row list.
fn action_for_mouse(mouse: MouseEvent, area: Rect, dashboard: &Dashboard) -> Option<Action> {
    match mouse.kind {
        MouseEventKind::ScrollDown => {
            return Some(
                if render::detail_contains(area, dashboard.view(), mouse.column, mouse.row) {
                    Action::ScrollDetailDown
                } else {
                    Action::MoveDown
                },
            );
        }
        MouseEventKind::ScrollUp => {
            return Some(
                if render::detail_contains(area, dashboard.view(), mouse.column, mouse.row) {
                    Action::ScrollDetailUp
                } else {
                    Action::MoveUp
                },
            );
        }
        MouseEventKind::Down(MouseButton::Right) if dashboard.view() == View::Directories => {
            return Some(Action::LeaveDirectory);
        }
        MouseEventKind::Down(MouseButton::Left) => {}
        _ => return None,
    }

    if mouse.row == area.y.saturating_add(1) {
        let mut x = area.x;
        for (index, view) in View::ALL.iter().enumerate() {
            let width = u16::try_from(format!(" {}·{}  ", index + 1, view.title()).chars().count())
                .unwrap_or(u16::MAX);
            if mouse.column >= x && mouse.column < x.saturating_add(width) {
                return Some(Action::SelectView(index));
            }
            x = x.saturating_add(width);
        }
    }

    let row = render::row_at(area, dashboard.view(), mouse.column, mouse.row)?;
    if row >= dashboard.row_count() {
        return None;
    }
    if dashboard.view() == View::Directories {
        Some(Action::EnterDirectoryAt(row))
    } else {
        Some(Action::SelectRow(row))
    }
}

/// What a key press means, given whether a filter is being typed.
///
/// Returns `None` for a key with no meaning in the current mode, which the loop
/// treats as "redraw and wait" rather than as an error.
///
/// While a filter is being typed almost every key is a character rather than a
/// command — otherwise a path containing a `q` would close the dashboard
/// mid-search.
#[must_use]
pub fn action_for(key: KeyEvent, editing_filter: bool) -> Option<Action> {
    // Terminals that report both press and release would otherwise apply every
    // action twice.
    if key.kind == KeyEventKind::Release {
        return None;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        return Some(Action::Quit);
    }

    if editing_filter {
        return match key.code {
            KeyCode::Esc => Some(Action::CancelFilter),
            KeyCode::Enter => Some(Action::CommitFilter),
            KeyCode::Backspace => Some(Action::FilterPop),
            KeyCode::Char(character) => Some(Action::FilterPush(character)),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        KeyCode::Tab => Some(Action::NextView),
        KeyCode::BackTab => Some(Action::PreviousView),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::MoveDown),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::MoveUp),
        KeyCode::PageDown | KeyCode::Char('d') => Some(Action::PageDown),
        KeyCode::PageUp | KeyCode::Char('u') => Some(Action::PageUp),
        KeyCode::Home | KeyCode::Char('g') => Some(Action::First),
        KeyCode::End | KeyCode::Char('G') => Some(Action::Last),
        KeyCode::Char('t') => Some(Action::ToggleTests),
        KeyCode::Char('i') => Some(Action::ToggleGitignore),
        KeyCode::Char('s') => Some(Action::NextSort),
        KeyCode::Char('/') => Some(Action::StartFilter),
        KeyCode::Char(digit @ '1'..='9') => {
            let position = digit.to_digit(10).unwrap_or(1).saturating_sub(1);
            Some(Action::SelectView(position as usize))
        }
        _ => None,
    }
}

#[cfg(test)]
mod test;
