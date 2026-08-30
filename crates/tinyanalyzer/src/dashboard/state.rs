//! What the dashboard is showing, and what a keystroke does to it.
//!
//! All of the dashboard's behavior lives here as a plain state machine over a
//! [`Report`]: which view is open, where the cursor is, whether tests are
//! hidden, what the filter says. Nothing in this file knows what a terminal is.
//!
//! That separation is what makes the interactive half testable. A key press
//! becomes an [`Action`] at the edge of the program, and every question worth
//! asking — does `t` really remove test files from the ranking, does the cursor
//! stay in range when a filter shrinks the list under it — is a question about
//! this struct, answerable without drawing anything.

use tinyanalyzer_core::{
    DeadCodeCandidate, DirectoryMetrics, FileMetrics, Finding, PackageNode, Report, StartView,
    Totals,
};

/// A pane of the dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {
    /// Totals, language mix, and the headline findings.
    Overview,
    /// Files ranked by weight.
    Files,
    /// Directories ranked by size.
    Directories,
    /// The dependency graph and the heaviest crates in it.
    Dependencies,
    /// Unreferenced items.
    DeadCode,
    /// Every finding, ranked by severity.
    Findings,
}

impl View {
    /// Every view, in the order the tab bar shows them.
    pub const ALL: [Self; 6] = [
        Self::Overview,
        Self::Files,
        Self::Directories,
        Self::Dependencies,
        Self::DeadCode,
        Self::Findings,
    ];

    /// The label on the tab bar.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Files => "Files",
            Self::Directories => "Directories",
            Self::Dependencies => "Dependencies",
            Self::DeadCode => "Dead code",
            Self::Findings => "Findings",
        }
    }

    /// Position in [`Self::ALL`].
    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|view| *view == self)
            .unwrap_or_default()
    }

    /// The view at `index`, wrapping.
    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self::ALL[index % Self::ALL.len()]
    }

    /// The view a configured start position names.
    #[must_use]
    pub const fn from_start(start: StartView) -> Self {
        match start {
            StartView::Overview => Self::Overview,
            StartView::Files => Self::Files,
            StartView::Dependencies => Self::Dependencies,
            StartView::DeadCode => Self::DeadCode,
            StartView::Findings => Self::Findings,
        }
    }
}

/// Something the operator asked the dashboard to do.
///
/// Keys are translated into these at the edge of the program, so the same
/// behavior can be driven by a test without inventing key events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Leave.
    Quit,
    /// Move to the next view.
    NextView,
    /// Move to the previous view.
    PreviousView,
    /// Jump to a view by its position on the tab bar.
    SelectView(usize),
    /// Move the cursor down one row.
    MoveDown,
    /// Move the cursor up one row.
    MoveUp,
    /// Move the cursor down a screenful.
    PageDown,
    /// Move the cursor up a screenful.
    PageUp,
    /// Move the cursor to the first row.
    First,
    /// Move the cursor to the last row.
    Last,
    /// Select a row directly, as when it is clicked.
    SelectRow(usize),
    /// Enter the selected directory.
    EnterDirectory,
    /// Enter a directory row directly, as when it is clicked.
    EnterDirectoryAt(usize),
    /// Return to the parent directory.
    LeaveDirectory,
    /// Show or hide test code.
    ToggleTests,
    /// Start typing a filter.
    StartFilter,
    /// Stop typing, keeping what was typed.
    CommitFilter,
    /// Stop typing and discard the filter.
    CancelFilter,
    /// Append a character to the filter.
    FilterPush(char),
    /// Remove the last character from the filter.
    FilterPop,
}

/// How many rows a page key moves.
const PAGE: usize = 10;

/// Everything the dashboard is showing.
#[derive(Debug, Clone)]
pub struct Dashboard {
    report: Report,
    view: View,
    hide_tests: bool,
    cursors: [usize; View::ALL.len()],
    filter: String,
    editing_filter: bool,
    directory_path: String,
    directory_cursors: Vec<usize>,
    quit: bool,
}

impl Dashboard {
    /// Opens a dashboard on `report`, honoring the configured start state.
    #[must_use]
    pub fn new(report: Report, start: StartView, hide_tests: bool) -> Self {
        Self {
            report,
            view: View::from_start(start),
            hide_tests,
            cursors: [0; View::ALL.len()],
            filter: String::new(),
            editing_filter: false,
            directory_path: ".".to_owned(),
            directory_cursors: Vec::new(),
            quit: false,
        }
    }

    /// The report being shown.
    #[must_use]
    pub const fn report(&self) -> &Report {
        &self.report
    }

    /// The view currently open.
    #[must_use]
    pub const fn view(&self) -> View {
        self.view
    }

    /// Whether test code is hidden.
    #[must_use]
    pub const fn hide_tests(&self) -> bool {
        self.hide_tests
    }

    /// The current filter text.
    #[must_use]
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Whether the operator is typing a filter.
    #[must_use]
    pub const fn editing_filter(&self) -> bool {
        self.editing_filter
    }

    /// Whether the dashboard has been asked to close.
    #[must_use]
    pub const fn should_quit(&self) -> bool {
        self.quit
    }

    /// The cursor position in the current view.
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursors[self.view.index()]
    }

    /// Totals for the current test filter.
    #[must_use]
    pub fn totals(&self) -> Totals {
        if self.hide_tests {
            self.report.production_totals()
        } else {
            self.report.totals
        }
    }

    /// Files matching the current filters, in report order.
    #[must_use]
    pub fn files(&self) -> Vec<&FileMetrics> {
        self.report
            .files
            .iter()
            .filter(|file| !(self.hide_tests && file.is_test))
            .filter(|file| self.matches(&file.path))
            .collect()
    }

    /// Immediate child directories at the current browser level.
    ///
    /// Metrics include every matching file below the child, like `ncdu`, so a
    /// parent row honestly represents the whole subtree rather than only files
    /// stored directly inside it.
    #[must_use]
    pub fn directories(&self) -> Vec<DirectoryMetrics> {
        let mut children = std::collections::BTreeMap::<String, DirectoryMetrics>::new();

        for file in self.files() {
            let Some(child_path) = immediate_child(&self.directory_path, &file.directory) else {
                continue;
            };

            let entry = children
                .entry(child_path.clone())
                .or_insert_with(|| DirectoryMetrics {
                    path: child_path,
                    files: 0,
                    bytes: 0,
                    lines: Default::default(),
                    is_test_only: true,
                });
            entry.files = entry.files.saturating_add(1);
            entry.bytes = entry.bytes.saturating_add(file.bytes);
            entry.lines.add(file.lines);
            entry.is_test_only = entry.is_test_only && file.is_test;
        }

        let mut rows: Vec<_> = children.into_values().collect();
        rows.sort_by(|left, right| {
            right
                .bytes
                .cmp(&left.bytes)
                .then_with(|| left.path.cmp(&right.path))
        });
        rows
    }

    /// Directory currently open in the level-by-level browser.
    #[must_use]
    pub fn directory_path(&self) -> &str {
        &self.directory_path
    }

    /// Direct dependencies matching the current filter, heaviest first.
    #[must_use]
    pub fn packages(&self) -> Vec<&PackageNode> {
        self.report
            .dependencies
            .heaviest_direct()
            .into_iter()
            .filter(|package| self.matches(&package.name))
            .collect()
    }

    /// Unreferenced items matching the current filters.
    #[must_use]
    pub fn dead_code(&self) -> Vec<&DeadCodeCandidate> {
        self.report
            .dead_code
            .iter()
            .filter(|candidate| !(self.hide_tests && candidate.is_test))
            .filter(|candidate| self.matches(&candidate.name) || self.matches(&candidate.file))
            .collect()
    }

    /// Findings matching the current filter.
    #[must_use]
    pub fn findings(&self) -> Vec<&Finding> {
        self.report
            .findings
            .iter()
            .filter(|finding| self.matches(&finding.title) || self.matches(finding.rule.id()))
            .collect()
    }

    /// How many rows the current view has.
    #[must_use]
    pub fn row_count(&self) -> usize {
        match self.view {
            View::Overview => self.report.findings.len(),
            View::Files => self.files().len(),
            View::Directories => self.directories().len(),
            View::Dependencies => self.packages().len(),
            View::DeadCode => self.dead_code().len(),
            View::Findings => self.findings().len(),
        }
    }

    /// The file the cursor is on, when the files view is open.
    #[must_use]
    pub fn selected_file(&self) -> Option<&FileMetrics> {
        let files = self.files();
        files.get(self.cursor()).copied()
    }

    /// The package the cursor is on, when the dependencies view is open.
    #[must_use]
    pub fn selected_package(&self) -> Option<&PackageNode> {
        let packages = self.packages();
        packages.get(self.cursor()).copied()
    }

    /// The finding the cursor is on, when the findings view is open.
    #[must_use]
    pub fn selected_finding(&self) -> Option<&Finding> {
        let findings = self.findings();
        findings.get(self.cursor()).copied()
    }

    /// The packages reached from `id`, as an indented tree.
    ///
    /// Each entry is the depth below `id` and the package. Depth is capped so a
    /// dependency with a thousand-crate subtree does not lock the renderer up
    /// drawing something nobody can read; the count in the header is the honest
    /// total either way.
    #[must_use]
    pub fn subtree(&self, id: &str, max_depth: usize) -> Vec<(usize, &PackageNode)> {
        let mut out = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut stack = vec![(id.to_owned(), 0usize)];

        while let Some((current, depth)) = stack.pop() {
            if depth > max_depth {
                continue;
            }

            let mut children: Vec<&str> = self
                .report
                .dependencies
                .edges
                .iter()
                .filter(|edge| edge.from == current)
                .map(|edge| edge.to.as_str())
                .collect();
            children.sort_unstable();
            children.dedup();

            // Reversed, because the stack pops last-in first and the tree
            // should read in the order the children are sorted.
            for child in children.into_iter().rev() {
                if !seen.insert(child.to_owned()) {
                    continue;
                }
                if let Some(package) = self.report.dependencies.package(child) {
                    out.push((depth, package));
                    stack.push((child.to_owned(), depth.saturating_add(1)));
                }
            }
        }

        out.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.name.cmp(&right.1.name))
        });
        out
    }

    /// Whether `text` passes the current filter.
    ///
    /// Case-insensitive substring matching. Not a glob and not a regex: the
    /// filter is typed one character at a time while looking at the result, and
    /// a syntax that can be half-written is a syntax that spends most of its
    /// life invalid.
    fn matches(&self, text: &str) -> bool {
        if self.filter.is_empty() {
            return true;
        }

        text.to_lowercase().contains(&self.filter.to_lowercase())
    }

    /// Applies one action.
    pub fn apply(&mut self, action: Action) {
        match action {
            Action::Quit => self.quit = true,
            Action::NextView => {
                self.view = View::from_index(self.view.index().saturating_add(1));
            }
            Action::PreviousView => {
                let last = View::ALL.len().saturating_sub(1);
                let index = if self.view.index() == 0 {
                    last
                } else {
                    self.view.index().saturating_sub(1)
                };
                self.view = View::from_index(index);
            }
            Action::SelectView(index) => {
                if index < View::ALL.len() {
                    self.view = View::from_index(index);
                }
            }
            Action::MoveDown => self.move_cursor(1),
            Action::MoveUp => self.move_cursor(-1),
            Action::PageDown => self.move_cursor(PAGE.try_into().unwrap_or(i64::MAX)),
            Action::PageUp => self.move_cursor(-PAGE.try_into().unwrap_or(i64::MAX)),
            Action::First => self.set_cursor(0),
            Action::Last => self.set_cursor(self.row_count().saturating_sub(1)),
            Action::SelectRow(position) => self.set_cursor(position),
            Action::EnterDirectory => self.enter_directory(self.cursor()),
            Action::EnterDirectoryAt(position) => self.enter_directory(position),
            Action::LeaveDirectory => self.leave_directory(),
            Action::ToggleTests => {
                self.hide_tests = !self.hide_tests;
                self.clamp_cursor();
            }
            Action::StartFilter => self.editing_filter = true,
            Action::CommitFilter => self.editing_filter = false,
            Action::CancelFilter => {
                self.editing_filter = false;
                self.filter.clear();
                self.clamp_cursor();
            }
            Action::FilterPush(character) => {
                self.filter.push(character);
                self.clamp_cursor();
            }
            Action::FilterPop => {
                self.filter.pop();
                self.clamp_cursor();
            }
        }
    }

    /// Moves the cursor by `delta`, clamped to the current view's rows.
    fn move_cursor(&mut self, delta: i64) {
        let current = i64::try_from(self.cursor()).unwrap_or(i64::MAX);
        let next = current.saturating_add(delta).max(0);

        self.set_cursor(usize::try_from(next).unwrap_or(0));
    }

    /// Puts the cursor at `position`, clamped to the current view's rows.
    fn set_cursor(&mut self, position: usize) {
        let last = self.row_count().saturating_sub(1);
        self.cursors[self.view.index()] = position.min(last);
    }

    /// Pulls the cursor back into range after the row set shrinks.
    ///
    /// A filter that removes everything below the cursor would otherwise leave
    /// it pointing past the end, and every `selected_*` accessor would return
    /// nothing while the list plainly has rows in it.
    fn clamp_cursor(&mut self) {
        let last = self.row_count().saturating_sub(1);
        let cursor = &mut self.cursors[self.view.index()];
        *cursor = (*cursor).min(last);
    }

    /// Opens the directory at `position` when it contains another level.
    fn enter_directory(&mut self, position: usize) {
        if self.view != View::Directories {
            return;
        }

        let rows = self.directories();
        let Some(selected) = rows.get(position) else {
            return;
        };
        let selected_path = selected.path.clone();
        let has_children = self.report.files.iter().any(|file| {
            file.directory
                .strip_prefix(&selected_path)
                .is_some_and(|rest| rest.starts_with('/'))
        });
        if !has_children {
            self.set_cursor(position);
            return;
        }

        self.directory_cursors.push(position);
        self.directory_path = selected_path;
        self.set_cursor(0);
    }

    /// Returns to the parent directory and restores its previous cursor.
    fn leave_directory(&mut self) {
        if self.view != View::Directories || self.directory_path == "." {
            return;
        }

        self.directory_path = self
            .directory_path
            .rsplit_once('/')
            .map_or_else(|| ".".to_owned(), |(parent, _)| parent.to_owned());
        let cursor = self.directory_cursors.pop().unwrap_or_default();
        self.set_cursor(cursor);
    }
}

/// Returns the immediate child of `current` containing `directory`.
fn immediate_child(current: &str, directory: &str) -> Option<String> {
    let remainder = if current == "." {
        (directory != ".").then_some(directory)?
    } else {
        directory.strip_prefix(current)?.strip_prefix('/')?
    };
    let child = remainder.split('/').next()?;

    Some(if current == "." {
        child.to_owned()
    } else {
        format!("{current}/{child}")
    })
}
