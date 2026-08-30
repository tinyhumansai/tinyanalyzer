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

use regex::{Regex, RegexBuilder};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use tinyanalyzer_core::{
    DeadCodeCandidate, DirectoryMetrics, FileMetrics, Finding, LineCounts, PackageNode, Report,
    StartView, Totals,
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
    /// Show or hide files in the directory browser.
    ToggleDirectoriesOnly,
    /// Scroll the active detail pane down.
    ScrollDetailDown,
    /// Scroll the active detail pane up.
    ScrollDetailUp,
    /// Cycle to the next sort for the active view.
    NextSort,
    /// Remove the selected dependency from the simulated graph.
    SimulateRemoveDependency,
    /// Restore every dependency removed from the simulation.
    RestoreDependencies,
    /// Select the next Cargo feature in the dependency detail pane.
    NextFeature,
    /// Select the previous Cargo feature in the dependency detail pane.
    PreviousFeature,
    /// Toggle the selected Cargo feature in the simulation.
    ToggleFeature,
    /// Switch feature simulation between the selected dependency and workspace root.
    ToggleFeatureTarget,
    /// Toggle whether filesystem discovery respects gitignore rules.
    ToggleGitignore,
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

/// One row in the ncdu-style directory browser.
#[derive(Debug)]
pub(super) enum BrowserEntry<'a> {
    /// Synthetic link to the parent directory.
    Parent,
    /// A child directory with cumulative subtree metrics.
    Directory(DirectoryMetrics),
    /// A file stored directly at the current level.
    File(&'a FileMetrics),
}

/// Which kinds of entries the directory browser exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectoryVisibility {
    /// Show child directories and immediate files together.
    All,
    /// Hide files while retaining directory navigation.
    DirectoriesOnly,
}

impl BrowserEntry<'_> {
    /// Full report-relative path.
    pub(super) fn path(&self) -> &str {
        match self {
            Self::Parent => "..",
            Self::Directory(directory) => &directory.path,
            Self::File(file) => &file.path,
        }
    }

    /// Whether this row can be entered as a directory.
    pub(super) const fn is_directory(&self) -> bool {
        matches!(self, Self::Parent | Self::Directory(_))
    }

    /// Number of files represented by this row.
    pub(super) const fn file_count(&self) -> usize {
        match self {
            Self::Parent => 0,
            Self::Directory(directory) => directory.files,
            Self::File(_) => 1,
        }
    }

    /// Bytes represented by this row.
    pub(super) const fn bytes(&self) -> u64 {
        match self {
            Self::Parent => 0,
            Self::Directory(directory) => directory.bytes,
            Self::File(file) => file.bytes,
        }
    }

    /// Line counts represented by this row under the current test policy.
    pub(super) fn lines(&self, dashboard: &Dashboard) -> LineCounts {
        match self {
            Self::Parent => LineCounts::default(),
            Self::Directory(directory) => directory.lines,
            Self::File(file) => dashboard.file_lines(file),
        }
    }

    /// Whether the row represents only test code.
    pub(super) const fn is_test_only(&self) -> bool {
        match self {
            Self::Parent => false,
            Self::Directory(directory) => directory.is_test_only,
            Self::File(file) => file.is_test,
        }
    }
}

/// Everything the dashboard is showing.
#[derive(Debug, Clone)]
pub struct Dashboard {
    report: Report,
    view: View,
    hide_tests: bool,
    cursors: [usize; View::ALL.len()],
    detail_scrolls: [u16; View::ALL.len()],
    sorts: [usize; View::ALL.len()],
    filters: [String; View::ALL.len()],
    filter_regexes: [Option<Regex>; View::ALL.len()],
    filter_syntaxes: [FilterSyntax; View::ALL.len()],
    editing_filter: bool,
    directory_path: String,
    directory_cursors: Vec<usize>,
    directory_visibility: DirectoryVisibility,
    removed_dependencies: BTreeSet<String>,
    feature_overrides: BTreeMap<String, BTreeSet<String>>,
    feature_cursor: usize,
    feature_target: FeatureTarget,
    ignore_policy: IgnorePolicy,
    reload_state: ReloadState,
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
            detail_scrolls: [0; View::ALL.len()],
            sorts: [0; View::ALL.len()],
            filters: std::array::from_fn(|_| String::new()),
            filter_regexes: std::array::from_fn(|_| None),
            filter_syntaxes: [FilterSyntax::Regex; View::ALL.len()],
            editing_filter: false,
            directory_path: ".".to_owned(),
            directory_cursors: Vec::new(),
            directory_visibility: DirectoryVisibility::All,
            removed_dependencies: BTreeSet::new(),
            feature_overrides: BTreeMap::new(),
            feature_cursor: 0,
            feature_target: FeatureTarget::Dependency,
            ignore_policy: IgnorePolicy::Respect,
            reload_state: ReloadState::Current,
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
        &self.filters[self.view.index()]
    }

    /// Whether the current filter is a valid regular expression.
    #[must_use]
    pub fn filter_regex_valid(&self) -> bool {
        self.filter_syntaxes[self.view.index()] == FilterSyntax::Regex
    }

    /// Label of the active view's current sort order.
    #[must_use]
    pub fn sort_label(&self) -> &'static str {
        match (self.view, self.sorts[self.view.index()]) {
            (View::Overview | View::Findings, 0) => "severity",
            (View::Overview | View::Findings, 1) => "title",
            (View::Overview | View::Findings, _) => "rule",
            (View::Files, 0) => "weight",
            (View::Files | View::Directories, 1) => "path",
            (View::Files, 2) => "lines",
            (View::Files, 3) | (View::Directories, 0) => "size",
            (View::Files, _) => "complexity",
            (View::Directories, 2) => "files",
            (View::Directories, _) => "lines",
            (View::Dependencies, 0) => "exclusive",
            (View::Dependencies | View::DeadCode, 1) => "name",
            (View::Dependencies, _) => "reachable",
            (View::DeadCode, 0) => "confidence",
            (View::DeadCode, _) => "file",
        }
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

    /// Whether filesystem discovery currently respects ignore files.
    #[must_use]
    pub fn respect_gitignore(&self) -> bool {
        self.ignore_policy == IgnorePolicy::Respect
    }

    /// Whether the report must be rebuilt for a changed ignore policy.
    #[must_use]
    pub fn reload_requested(&self) -> bool {
        self.reload_state == ReloadState::Requested
    }

    /// Consumes a pending reload request and returns the new ignore policy.
    pub fn take_reload_request(&mut self) -> Option<bool> {
        if self.reload_state != ReloadState::Requested {
            return None;
        }
        self.reload_state = ReloadState::Current;
        Some(self.respect_gitignore())
    }

    /// Replaces analysis results after the ignore policy changes.
    pub fn replace_report(&mut self, report: Report) {
        self.report = report;
        self.clamp_cursor();
    }

    /// Sets the initial ignore policy used by the interactive reload loop.
    pub fn set_respect_gitignore(&mut self, respect: bool) {
        self.ignore_policy = if respect {
            IgnorePolicy::Respect
        } else {
            IgnorePolicy::IncludeIgnored
        };
    }

    /// The cursor position in the current view.
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursors[self.view.index()]
    }

    /// Vertical offset of the current view's detail pane.
    #[must_use]
    pub fn detail_scroll(&self) -> u16 {
        self.detail_scrolls[self.view.index()]
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
        let mut files: Vec<_> = self
            .report
            .files
            .iter()
            .filter(|file| !(self.hide_tests && file.is_test))
            .filter(|file| self.matches(&file.path))
            .collect();
        match self.sorts[View::Files.index()] {
            0 => files.sort_by(|left, right| right.weight.total_cmp(&left.weight)),
            1 => files.sort_by(|left, right| left.path.cmp(&right.path)),
            2 => files.sort_by(|left, right| {
                self.file_lines(right).code.cmp(&self.file_lines(left).code)
            }),
            3 => files.sort_by_key(|file| Reverse(file.bytes)),
            _ => files.sort_by(|left, right| {
                self.file_complexity(right)
                    .cmp(&self.file_complexity(left))
                    .then_with(|| left.path.cmp(&right.path))
            }),
        }
        files
    }

    /// Line counts for a file after applying the test-code filter.
    #[must_use]
    pub fn file_lines(&self, file: &FileMetrics) -> LineCounts {
        if self.hide_tests {
            file.lines.without(file.test_lines)
        } else {
            file.lines
        }
    }

    /// Function count for a file after applying the test-code filter.
    #[must_use]
    pub fn file_function_count(&self, file: &FileMetrics) -> usize {
        file.rust.as_ref().map_or(0, |rust| {
            rust.functions
                .iter()
                .filter(|function| !(self.hide_tests && function.is_test))
                .count()
        })
    }

    /// Immediate child directories at the current browser level.
    ///
    /// Metrics include every matching file below the child, like `ncdu`, so a
    /// parent row honestly represents the whole subtree rather than only files
    /// stored directly inside it.
    #[must_use]
    pub fn directories(&self) -> Vec<DirectoryMetrics> {
        let mut children = BTreeMap::<String, DirectoryMetrics>::new();

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
                    lines: LineCounts::default(),
                    is_test_only: true,
                });
            entry.files = entry.files.saturating_add(1);
            entry.bytes = entry.bytes.saturating_add(file.bytes);
            entry.lines.add(self.file_lines(file));
            entry.is_test_only = entry.is_test_only && file.is_test;
        }

        let mut rows: Vec<_> = children.into_values().collect();
        match self.sorts[View::Directories.index()] {
            0 => rows.sort_by_key(|row| Reverse(row.bytes)),
            1 => rows.sort_by(|left, right| left.path.cmp(&right.path)),
            2 => rows.sort_by_key(|row| Reverse(row.files)),
            _ => rows.sort_by_key(|row| Reverse(row.lines.code)),
        }
        if self.directory_path != "." {
            rows.insert(
                0,
                DirectoryMetrics {
                    path: "..".to_owned(),
                    files: 0,
                    bytes: 0,
                    lines: LineCounts::default(),
                    is_test_only: false,
                },
            );
        }
        rows
    }

    /// Child directories and immediate files at the current browser level.
    #[must_use]
    pub(super) fn browser_entries(&self) -> Vec<BrowserEntry<'_>> {
        let mut entries: Vec<_> = self
            .directories()
            .into_iter()
            .filter(|directory| directory.path != "..")
            .map(BrowserEntry::Directory)
            .collect();

        if self.directory_visibility == DirectoryVisibility::All {
            entries.extend(
                self.files()
                    .into_iter()
                    .filter(|file| file.directory == self.directory_path)
                    .map(BrowserEntry::File),
            );
        }

        match self.sorts[View::Directories.index()] {
            0 => entries.sort_by_key(|entry| Reverse(entry.bytes())),
            1 => entries.sort_by(|left, right| left.path().cmp(right.path())),
            2 => entries.sort_by_key(|entry| Reverse(entry.file_count())),
            _ => entries.sort_by_key(|entry| Reverse(entry.lines(self).code)),
        }
        if self.directory_path != "." {
            entries.insert(0, BrowserEntry::Parent);
        }
        entries
    }

    /// Whether the directory browser is hiding files.
    #[must_use]
    pub const fn directories_only(&self) -> bool {
        matches!(
            self.directory_visibility,
            DirectoryVisibility::DirectoriesOnly
        )
    }

    /// Directory currently open in the level-by-level browser.
    #[must_use]
    pub fn directory_path(&self) -> &str {
        &self.directory_path
    }

    /// Direct dependencies matching the current filter, heaviest first.
    #[must_use]
    pub fn packages(&self) -> Vec<&PackageNode> {
        let mut packages: Vec<_> = self
            .report
            .dependencies
            .heaviest_direct()
            .into_iter()
            .filter(|package| self.matches(&package.name))
            .collect();
        match self.sorts[View::Dependencies.index()] {
            0 => packages.sort_by_key(|package| Reverse(package.exclusive_count)),
            1 => packages.sort_by(|left, right| left.name.cmp(&right.name)),
            _ => packages.sort_by_key(|package| Reverse(package.transitive_count)),
        }
        packages
    }

    /// Whether a direct dependency is disabled in the current simulation.
    #[must_use]
    pub fn dependency_is_removed(&self, id: &str) -> bool {
        self.removed_dependencies.contains(id)
    }

    /// Number of direct dependencies in the resolved graph.
    #[must_use]
    pub fn direct_dependency_count(&self) -> usize {
        self.report.dependencies.heaviest_direct().len()
    }

    /// Bytes occupied by every external package's checked-out source.
    #[must_use]
    pub fn dependency_source_bytes(&self) -> u64 {
        self.report
            .dependencies
            .packages
            .iter()
            .filter(|package| !package.is_workspace_member)
            .fold(0_u64, |total, package| {
                total.saturating_add(package.source_bytes)
            })
    }

    /// Number of external crates that the simulated build would compile.
    #[must_use]
    pub fn simulated_build_dependency_count(&self) -> usize {
        if self.removed_dependencies.is_empty() {
            return self.report.dependencies.external_packages;
        }
        let reachable = self.simulated_reachable();
        self.report
            .dependencies
            .packages
            .iter()
            .filter(|package| !package.is_workspace_member && reachable.contains(&package.id))
            .count()
    }

    /// Source bytes retained by the simulated external build graph.
    #[must_use]
    pub fn simulated_build_source_bytes(&self) -> u64 {
        if self.removed_dependencies.is_empty() {
            return self.dependency_source_bytes();
        }
        let reachable = self.simulated_reachable();
        self.report
            .dependencies
            .packages
            .iter()
            .filter(|package| !package.is_workspace_member && reachable.contains(&package.id))
            .fold(0_u64, |total, package| {
                total.saturating_add(package.source_bytes)
            })
    }

    /// Unreferenced items matching the current filters.
    #[must_use]
    pub fn dead_code(&self) -> Vec<&DeadCodeCandidate> {
        let mut entries: Vec<_> = self
            .report
            .dead_code
            .iter()
            .filter(|candidate| !(self.hide_tests && candidate.is_test))
            .filter(|candidate| self.matches(&candidate.name) || self.matches(&candidate.file))
            .collect();
        match self.sorts[View::DeadCode.index()] {
            0 => {}
            1 => entries.sort_by(|left, right| left.name.cmp(&right.name)),
            _ => entries.sort_by(|left, right| left.file.cmp(&right.file)),
        }
        entries
    }

    /// Findings matching the current filter.
    #[must_use]
    pub fn findings(&self) -> Vec<&Finding> {
        let mut findings: Vec<_> = self
            .report
            .findings
            .iter()
            .filter(|finding| self.matches(&finding.title) || self.matches(finding.rule.id()))
            .collect();
        match self.sorts[self.view.index()] {
            0 => {}
            1 => findings.sort_by(|left, right| left.title.cmp(&right.title)),
            _ => findings.sort_by(|left, right| left.rule.id().cmp(right.rule.id())),
        }
        findings
    }

    /// Direct dependencies currently removed by the simulator.
    #[must_use]
    pub fn removed_dependency_count(&self) -> usize {
        self.removed_dependencies.len()
    }

    /// External packages that become unreachable in the simulated graph.
    #[must_use]
    pub fn simulated_reclaimed_packages(&self) -> usize {
        self.simulated_unreachable_packages().len()
    }

    /// Direct dependency names explicitly removed from the simulation.
    #[must_use]
    pub fn removed_dependency_names(&self) -> Vec<&str> {
        self.report
            .dependencies
            .packages
            .iter()
            .filter(|package| self.removed_dependencies.contains(&package.id))
            .map(|package| package.name.as_str())
            .collect()
    }

    /// External packages no longer reachable in the simulated graph.
    #[must_use]
    pub fn simulated_unreachable_packages(&self) -> Vec<&PackageNode> {
        if self.removed_dependencies.is_empty() {
            return Vec::new();
        }
        let reachable = self.simulated_reachable();
        let mut packages: Vec<_> = self
            .report
            .dependencies
            .packages
            .iter()
            .filter(|package| !package.is_workspace_member && !reachable.contains(&package.id))
            .collect();
        packages.sort_by(|left, right| left.name.cmp(&right.name));
        packages
    }

    /// Package whose Cargo features are currently being simulated.
    #[must_use]
    pub fn feature_target_package(&self) -> Option<&PackageNode> {
        if self.feature_target == FeatureTarget::Root {
            self.report
                .dependencies
                .packages
                .iter()
                .filter(|package| package.is_root_package)
                .min_by(|left, right| left.name.cmp(&right.name))
                .or_else(|| {
                    self.report
                        .dependencies
                        .packages
                        .iter()
                        .filter(|package| package.is_workspace_member)
                        .min_by(|left, right| left.name.cmp(&right.name))
                })
        } else {
            self.selected_package()
        }
    }

    /// Available features and their simulated enabled state for the target.
    #[must_use]
    pub fn simulated_features(&self) -> Vec<(&str, bool)> {
        let Some(package) = self.feature_target_package() else {
            return Vec::new();
        };
        let override_features = self.feature_overrides.get(&package.id);
        package
            .available_features
            .iter()
            .map(|feature| {
                let enabled = override_features.map_or_else(
                    || package.features.iter().any(|entry| entry == feature),
                    |features| features.contains(feature),
                );
                (feature.as_str(), enabled)
            })
            .collect()
    }

    /// Cursor within the target package's feature list.
    #[must_use]
    pub const fn feature_cursor(&self) -> usize {
        self.feature_cursor
    }

    /// Whether feature controls currently target the workspace root package.
    #[must_use]
    pub fn feature_root_target(&self) -> bool {
        self.feature_target == FeatureTarget::Root
    }

    /// How many rows the current view has.
    #[must_use]
    pub fn row_count(&self) -> usize {
        match self.view {
            View::Overview => self.report.findings.len(),
            View::Files => self.files().len(),
            View::Directories => self.browser_entries().len(),
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
        if self.filter().is_empty() {
            return true;
        }

        self.filter_regexes[self.view.index()]
            .as_ref()
            .is_none_or(|filter| filter.is_match(text))
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
            Action::ToggleDirectoriesOnly => {
                self.directory_visibility = match self.directory_visibility {
                    DirectoryVisibility::All => DirectoryVisibility::DirectoriesOnly,
                    DirectoryVisibility::DirectoriesOnly => DirectoryVisibility::All,
                };
                self.clamp_cursor();
            }
            Action::ScrollDetailDown => {
                let scroll = &mut self.detail_scrolls[self.view.index()];
                *scroll = scroll.saturating_add(3);
            }
            Action::ScrollDetailUp => {
                let scroll = &mut self.detail_scrolls[self.view.index()];
                *scroll = scroll.saturating_sub(3);
            }
            Action::NextSort => {
                let view = self.view.index();
                self.sorts[view] = (self.sorts[view] + 1) % sort_count(self.view);
                self.detail_scrolls[view] = 0;
                self.set_cursor(0);
            }
            Action::SimulateRemoveDependency => self.simulate_remove_dependency(),
            Action::RestoreDependencies => {
                self.removed_dependencies.clear();
                self.clamp_cursor();
            }
            Action::ToggleGitignore => {
                self.ignore_policy = match self.ignore_policy {
                    IgnorePolicy::Respect => IgnorePolicy::IncludeIgnored,
                    IgnorePolicy::IncludeIgnored => IgnorePolicy::Respect,
                };
                self.reload_state = ReloadState::Requested;
            }
            Action::NextFeature => self.move_feature(1),
            Action::PreviousFeature => self.move_feature(-1),
            Action::ToggleFeature => self.toggle_feature(),
            Action::ToggleFeatureTarget => {
                self.feature_target = match self.feature_target {
                    FeatureTarget::Dependency => FeatureTarget::Root,
                    FeatureTarget::Root => FeatureTarget::Dependency,
                };
                self.feature_cursor = 0;
            }
            Action::ToggleTests => {
                self.hide_tests = !self.hide_tests;
                self.clamp_cursor();
            }
            Action::StartFilter => self.editing_filter = true,
            Action::CommitFilter => self.editing_filter = false,
            Action::CancelFilter => {
                self.editing_filter = false;
                self.filters[self.view.index()].clear();
                self.compile_filter();
                self.clamp_cursor();
            }
            Action::FilterPush(character) => {
                self.filters[self.view.index()].push(character);
                self.compile_filter();
                self.clamp_cursor();
            }
            Action::FilterPop => {
                self.filters[self.view.index()].pop();
                self.compile_filter();
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
        let view = self.view.index();
        let next = position.min(last);
        if self.cursors[view] != next {
            self.detail_scrolls[view] = 0;
        }
        self.cursors[view] = next;
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

        let rows = self.browser_entries();
        let Some(selected) = rows.get(position) else {
            return;
        };
        if matches!(selected, BrowserEntry::Parent) {
            self.leave_directory();
            return;
        }
        let BrowserEntry::Directory(directory) = selected else {
            self.set_cursor(position);
            return;
        };
        let selected_path = directory.path.clone();
        let has_children = self.files().iter().any(|file| {
            file.directory == selected_path
                || file
                    .directory
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

    /// Compiles the filter, falling back to a literal while a regex is incomplete.
    fn compile_filter(&mut self) {
        let view = self.view.index();
        if self.filters[view].is_empty() {
            self.filter_regexes[view] = None;
            self.filter_syntaxes[view] = FilterSyntax::Regex;
            return;
        }
        if let Ok(regex) = RegexBuilder::new(&self.filters[view])
            .case_insensitive(true)
            .build()
        {
            self.filter_regexes[view] = Some(regex);
            self.filter_syntaxes[view] = FilterSyntax::Regex;
        } else {
            self.filter_regexes[view] = RegexBuilder::new(&regex::escape(&self.filters[view]))
                .case_insensitive(true)
                .build()
                .ok();
            self.filter_syntaxes[view] = FilterSyntax::Literal;
        }
    }

    /// Removes the selected direct dependency from the simulated workspace edges.
    fn simulate_remove_dependency(&mut self) {
        if self.view != View::Dependencies {
            return;
        }
        let Some(id) = self.selected_package().map(|package| package.id.clone()) else {
            return;
        };
        self.removed_dependencies.insert(id);
        self.clamp_cursor();
    }

    /// IDs reachable from workspace members after simulated direct edges are removed.
    fn simulated_reachable(&self) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut queue: VecDeque<String> = self
            .report
            .dependencies
            .packages
            .iter()
            .filter(|package| package.is_workspace_member)
            .map(|package| package.id.clone())
            .collect();
        while let Some(id) = queue.pop_front() {
            for edge in self
                .report
                .dependencies
                .edges
                .iter()
                .filter(|edge| edge.from == id)
            {
                if self.removed_dependencies.contains(&edge.to)
                    && self
                        .report
                        .dependencies
                        .package(&id)
                        .is_some_and(|package| package.is_workspace_member)
                {
                    continue;
                }
                if seen.insert(edge.to.clone()) {
                    queue.push_back(edge.to.clone());
                }
            }
        }
        seen
    }

    fn move_feature(&mut self, delta: i64) {
        let count = self.simulated_features().len();
        if count == 0 {
            self.feature_cursor = 0;
            return;
        }
        let current = i64::try_from(self.feature_cursor).unwrap_or_default();
        let last = i64::try_from(count.saturating_sub(1)).unwrap_or(i64::MAX);
        self.feature_cursor =
            usize::try_from(current.saturating_add(delta).clamp(0, last)).unwrap_or_default();
    }

    fn toggle_feature(&mut self) {
        let Some(package) = self.feature_target_package() else {
            return;
        };
        let Some(feature) = package.available_features.get(self.feature_cursor) else {
            return;
        };
        let id = package.id.clone();
        let feature = feature.clone();
        let original = package.features.iter().cloned().collect();
        let enabled = self.feature_overrides.entry(id).or_insert(original);
        if !enabled.remove(&feature) {
            enabled.insert(feature);
        }
    }

    fn file_complexity(&self, file: &FileMetrics) -> u32 {
        file.rust.as_ref().map_or(0, |rust| {
            rust.functions
                .iter()
                .filter(|function| !(self.hide_tests && function.is_test))
                .map(|function| function.complexity)
                .sum()
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterSyntax {
    Regex,
    Literal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeatureTarget {
    Dependency,
    Root,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IgnorePolicy {
    Respect,
    IncludeIgnored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReloadState {
    Current,
    Requested,
}

const fn sort_count(view: View) -> usize {
    match view {
        View::Overview | View::Findings | View::Dependencies | View::DeadCode => 3,
        View::Directories => 4,
        View::Files => 5,
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
