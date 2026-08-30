//! The interactive terminal dashboard.
//!
//! This module owns the two things that cannot be tested without a terminal —
//! the raw-mode lifecycle and the event loop — and nothing else. What a
//! keystroke *means* is [`action_for`], a pure function; what it *does* is
//! [`state::Dashboard::apply`], a pure state transition; what the result looks
//! like is [`render::draw`], a pure map to widgets. The loop below just moves
//! values between them.
//!
//! The terminal is restored on every exit path, including a panic: leaving a
//! user's shell in raw mode with the cursor hidden is the one failure this
//! program could cause that outlives the program.

mod render;
mod state;

pub use state::{Action, Dashboard, View};

use crate::error::{Error, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tinyanalyzer_core::{Report, StartView};

/// Opens the dashboard on `report` and runs until the operator leaves.
///
/// # Errors
///
/// Returns [`Error::Terminal`] if the terminal cannot be put into raw mode,
/// read from, or restored.
pub fn run(report: Report, start: StartView, hide_tests: bool) -> Result<()> {
    let mut dashboard = Dashboard::new(report, start, hide_tests);

    let mut terminal = ratatui::try_init().map_err(|source| Error::Terminal { source })?;

    let outcome = event_loop(&mut terminal, &mut dashboard);

    // Restored before the outcome is inspected: a failure inside the loop must
    // not leave the terminal in raw mode on the way out.
    let restored = ratatui::try_restore().map_err(|source| Error::Terminal { source });

    outcome.and(restored)
}

/// Draws and reads events until the dashboard is asked to close.
fn event_loop(terminal: &mut ratatui::DefaultTerminal, dashboard: &mut Dashboard) -> Result<()> {
    while !dashboard.should_quit() {
        terminal
            .draw(|frame| render::draw(frame, dashboard))
            .map_err(|source| Error::Terminal { source })?;

        let event = event::read().map_err(|source| Error::Terminal { source })?;

        if let Event::Key(key) = event
            && let Some(action) = action_for(key, dashboard.editing_filter())
        {
            dashboard.apply(action);
        }
    }

    Ok(())
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
        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => Some(Action::NextView),
        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => Some(Action::PreviousView),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::MoveDown),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::MoveUp),
        KeyCode::PageDown | KeyCode::Char('d') => Some(Action::PageDown),
        KeyCode::PageUp | KeyCode::Char('u') => Some(Action::PageUp),
        KeyCode::Home | KeyCode::Char('g') => Some(Action::First),
        KeyCode::End | KeyCode::Char('G') => Some(Action::Last),
        KeyCode::Char('t') => Some(Action::ToggleTests),
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
