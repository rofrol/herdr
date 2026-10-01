//! A filter bar above the spaces list, like fzf: type to narrow the list to
//! the spaces and tabs that match. Client-only presentation state: nothing is
//! sent to the server, nothing is saved, and every client has its own.
//!
//! A query matches as a smart-case subsequence (lowercase ignores case, an
//! uppercase letter makes it exact) against a space's name and branch, the
//! names of its agents, a tab's label, the agents in the tab and the labels
//! of the tabs nested under it (so `gem` finds the tab running an `ask gemini`
//! job). The list keeps its order. A space that matches shows all its tabs; a
//! space shown only for a tab shows only the matching tabs; a worktree
//! parent stays as the context of a matching child.

use std::collections::HashSet;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

use super::render::put_text;
use super::state::WorkspaceEntry;
use super::*;
use crate::app::state::Palette;
use crate::protocol::{ClientShellSnapshot, ClientShellWorkspace};

/// The bar's state. `open` shows the bar, `focused` sends typed keys to it
/// instead of the pane; clicking a pane blurs it and leaves the filter on.
#[derive(Debug, Default)]
pub(super) struct SpaceFilter {
    pub(super) open: bool,
    pub(super) focused: bool,
    pub(super) query: String,
}

impl SpaceFilter {
    /// Whether the list is narrowed.
    pub(super) fn active(&self) -> bool {
        self.open && !self.query.trim().is_empty()
    }

    pub(super) fn close(&mut self) {
        *self = Self::default();
    }
}

/// What the sidebar draws for the bar: the text, whether it takes keys, and
/// what the query shows (none while the text is empty).
pub(super) struct FilterRender<'a> {
    pub(super) query: &'a str,
    pub(super) focused: bool,
    pub(super) view: Option<FilterView>,
}

/// Whether `query` is a subsequence of `text`: case-insensitive unless the
/// query has an uppercase letter. An empty query matches everything.
pub(super) fn matches(query: &str, text: &str) -> bool {
    let query = query.trim();
    let exact = query.chars().any(char::is_uppercase);
    let mut wanted = query.chars().filter(|c| !c.is_whitespace());
    let mut next = wanted.next();
    for c in text.chars() {
        let Some(want) = next else {
            return true;
        };
        let same = if exact {
            c == want
        } else {
            c.to_lowercase().eq(want.to_lowercase())
        };
        if same {
            next = wanted.next();
        }
    }
    next.is_none()
}

/// What a query shows: the spaces and tabs that match, and every space shown.
#[derive(Debug, Default)]
pub(super) struct FilterView {
    /// Spaces that match by their own name, branch or agents.
    spaces: HashSet<String>,
    /// Top-level tabs that match.
    tabs: HashSet<String>,
    /// Spaces shown: matching ones, ones with a matching tab, and parents of
    /// shown worktrees.
    visible: HashSet<String>,
}

impl FilterView {
    pub(super) fn new(snapshot: &ClientShellSnapshot, query: &str) -> Self {
        let mut view = Self::default();
        for workspace in &snapshot.workspaces {
            let id = &workspace.workspace_id;
            let space_texts = std::iter::once(workspace.label.as_str())
                .chain(workspace.branch.as_deref())
                .chain(
                    snapshot
                        .agents
                        .iter()
                        .filter(|agent| agent.workspace_id == *id)
                        .flat_map(|agent| {
                            [&agent.name, &agent.display_agent, &agent.agent]
                                .into_iter()
                                .flatten()
                                .map(String::as_str)
                        }),
                );
            if space_texts.into_iter().any(|text| matches(query, text)) {
                view.spaces.insert(id.clone());
                view.visible.insert(id.clone());
            }
            for tab in snapshot
                .tabs
                .iter()
                .filter(|tab| tab.workspace_id == *id && tab.parent_tab_id.is_none())
            {
                let nested = snapshot
                    .tabs
                    .iter()
                    .filter(|child| child.parent_tab_id.as_deref() == Some(tab.tab_id.as_str()))
                    .map(|child| child.label.as_str());
                let agents = snapshot
                    .agents
                    .iter()
                    .filter(|agent| agent.tab_id == tab.tab_id)
                    .flat_map(|agent| {
                        [&agent.name, &agent.display_agent, &agent.agent]
                            .into_iter()
                            .flatten()
                            .map(String::as_str)
                    });
                if std::iter::once(tab.label.as_str())
                    .chain(nested)
                    .chain(agents)
                    .any(|text| matches(query, text))
                {
                    view.tabs.insert(tab.tab_id.clone());
                    view.visible.insert(id.clone());
                }
            }
        }
        view
    }

    /// Whether a space's tab line is shown: all of them for a matching space,
    /// else the matching ones.
    pub(super) fn shows_tab(&self, workspace: &ClientShellWorkspace, tab_id: &str) -> bool {
        self.spaces.contains(&workspace.workspace_id) || self.tabs.contains(tab_id)
    }

    /// `entries` without the spaces the query hides; the tree prefix of the
    /// last shown worktree of a group is recomputed.
    pub(super) fn filter_entries(
        &self,
        snapshot: &ClientShellSnapshot,
        entries: Vec<WorkspaceEntry>,
    ) -> Vec<WorkspaceEntry> {
        let shown = |entry: &WorkspaceEntry| {
            snapshot
                .workspaces
                .get(entry.index)
                .is_some_and(|workspace| self.visible.contains(&workspace.workspace_id))
        };
        // A worktree parent that does not match stays when a child is shown.
        let mut keep = vec![false; entries.len()];
        for (at, entry) in entries.iter().enumerate() {
            if shown(entry) {
                keep[at] = true;
                if entry.indented {
                    if let Some(parent) = entries[..at].iter().rposition(|e| !e.indented) {
                        keep[parent] = true;
                    }
                }
            }
        }
        let mut kept = entries
            .into_iter()
            .zip(keep)
            .filter_map(|(entry, keep)| keep.then_some(entry))
            .collect::<Vec<_>>();
        for at in 0..kept.len() {
            let last = !kept.get(at + 1).is_some_and(|next| next.indented);
            kept[at].last_child = kept[at].indented && last;
        }
        kept
    }

    /// The space and tab Enter opens: the first shown space, or its first
    /// matching tab when the space itself does not match.
    pub(super) fn first_target(
        &self,
        snapshot: &ClientShellSnapshot,
        entries: &[WorkspaceEntry],
    ) -> Option<(String, Option<String>)> {
        entries.iter().find_map(|entry| {
            let workspace = snapshot.workspaces.get(entry.index)?;
            if !self.visible.contains(&workspace.workspace_id) {
                return None;
            }
            let tab = (!self.spaces.contains(&workspace.workspace_id))
                .then(|| {
                    snapshot
                        .tabs
                        .iter()
                        .find(|tab| {
                            tab.workspace_id == workspace.workspace_id
                                && tab.parent_tab_id.is_none()
                                && self.tabs.contains(&tab.tab_id)
                        })
                        .map(|tab| tab.tab_id.clone())
                })
                .flatten();
            Some((workspace.workspace_id.clone(), tab))
        })
    }
}

impl ClientShellState {
    /// Takes a key press for the filter bar while it has the keys. Returns
    /// whether the key was used; other keys (shortcuts with Ctrl or Alt,
    /// function keys) still reach the normal routing.
    pub(super) fn handle_space_filter_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        if !self.space_filter.open || !self.space_filter.focused || self.overlay.is_some() {
            return false;
        }
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
        match key.code {
            KeyCode::Esc => {
                // The first Esc clears the text, the next one closes the bar.
                if self.space_filter.query.is_empty() {
                    self.space_filter.close();
                } else {
                    self.space_filter.query.clear();
                }
            }
            KeyCode::Enter => self.open_first_filter_match(outcome),
            KeyCode::Backspace => {
                self.space_filter.query.pop();
            }
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                self.space_filter.query.clear();
            }
            KeyCode::Char(c) if plain && !c.is_control() => self.space_filter.query.push(c),
            _ => return false,
        }
        outcome.repaint = true;
        true
    }

    /// Opens the first space the query shows (or its first matching tab) and
    /// closes the bar.
    fn open_first_filter_match(&mut self, outcome: &mut ClientShellInput) {
        let target = self.snapshot.as_deref().and_then(|snapshot| {
            let view = FilterView::new(snapshot, &self.space_filter.query);
            let none = HashSet::new();
            let entries = self.sorted_for_sidebar(
                snapshot,
                super::render::workspace_entries(snapshot, &none),
                &none,
            );
            let entries = view.filter_entries(snapshot, entries);
            view.first_target(snapshot, &entries)
        });
        let Some((workspace_id, tab_id)) = target else {
            return;
        };
        use crate::api::schema::{Method, TabTarget, WorkspaceTarget};
        let method = match tab_id {
            Some(tab_id) => Method::TabFocus(TabTarget { tab_id }),
            None => Method::WorkspaceFocus(WorkspaceTarget { workspace_id }),
        };
        self.space_filter.close();
        self.push_endpoint_method(method, outcome);
    }
}

/// Draws the `/ filter` button that opens the bar, centred in `room` (the
/// footer row between `new` and `menu`). Returns its rect, empty when it does
/// not fit.
pub(super) fn render_filter_button(
    buffer: &mut Buffer,
    room: Rect,
    open: bool,
    palette: &Palette,
) -> Rect {
    const LABEL: &str = "/ filter";
    let width = LABEL.chars().count() as u16;
    if room.width < width + 2 || room.height == 0 {
        return Rect::default();
    }
    let x = room.x + (room.width - width) / 2;
    let style = if open {
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.overlay0)
    };
    put_text(buffer, x, room.y, width, LABEL, style);
    Rect::new(x.saturating_sub(1), room.y, width + 2, 1)
}

/// Draws the bar: `/ query▏` and a `×` at the right. Returns the bar's rect
/// and the close button's.
pub(super) fn render_filter_bar(
    buffer: &mut Buffer,
    area: Rect,
    query: &str,
    focused: bool,
    palette: &Palette,
) -> (Rect, Rect) {
    if area.width < 6 || area.height == 0 {
        return (Rect::default(), Rect::default());
    }
    let accent = Style::default().fg(palette.accent);
    put_text(buffer, area.x.saturating_add(1), area.y, 1, "/", accent);
    let close = Rect::new(area.right().saturating_sub(3), area.y, 3, 1);
    put_text(
        buffer,
        close.x.saturating_add(1),
        area.y,
        1,
        "×",
        Style::default().fg(palette.overlay1),
    );
    let room = usize::from(close.x.saturating_sub(area.x + 3));
    let shown = if query.chars().count() + 1 > room {
        // The end of the text stays visible, where the cursor is.
        let skip = query.chars().count() + 1 - room;
        query.chars().skip(skip).collect::<String>()
    } else {
        query.to_owned()
    };
    let (text, style) = if query.is_empty() && !focused {
        ("filter".to_owned(), Style::default().fg(palette.overlay0))
    } else {
        (
            format!("{shown}{}", if focused { "▏" } else { "" }),
            Style::default().fg(palette.text),
        )
    };
    put_text(
        buffer,
        area.x.saturating_add(3),
        area.y,
        close.x.saturating_sub(area.x + 3),
        &text,
        style,
    );
    (area, close)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_matches_as_a_smart_case_subsequence() {
        assert!(matches("", "anything"));
        assert!(matches("hrd", "herdr"));
        assert!(matches("HRD", "HeRDr"));
        assert!(matches("herdr", "Herdr"), "lowercase ignores case");
        assert!(!matches("Herdr", "herdr"), "an uppercase letter is exact");
        assert!(!matches("rdh", "herdr"), "order matters");
        assert!(matches("a b", "ab"), "spaces in the query are ignored");
        assert!(!matches("x", "herdr"));
    }

    /// A repo space with two worktrees, `feature` and `fix`, plus a lone space.
    fn worktree_snapshot() -> ClientShellSnapshot {
        use crate::protocol::ClientShellWorktree;
        let mut snapshot = super::super::tests::snapshot();
        let template = snapshot.workspaces[0].clone();
        let add = |id: &str, label: &str, branch: &str, linked: Option<bool>| {
            let mut workspace = template.clone();
            workspace.workspace_id = id.into();
            workspace.label = label.into();
            workspace.branch = Some(branch.into());
            workspace.worktree = linked.map(|is_linked_worktree| ClientShellWorktree {
                key: "repo".into(),
                label: "repo".into(),
                is_linked_worktree,
            });
            workspace
        };
        snapshot.workspaces = vec![
            add("ws_main", "repo", "main", Some(false)),
            add("ws_feature", "repo-feature", "feature", Some(true)),
            add("ws_fix", "repo-fix", "fix", Some(true)),
            add("ws_other", "notes", "main", None),
        ];
        snapshot.tabs.clear();
        snapshot.agents.clear();
        snapshot
    }

    fn labels(snapshot: &ClientShellSnapshot, entries: &[WorkspaceEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| {
                format!(
                    "{}{}{}",
                    if entry.indented { "  " } else { "" },
                    snapshot.workspaces[entry.index].label,
                    if entry.last_child { " (last)" } else { "" }
                )
            })
            .collect()
    }

    #[test]
    fn a_shown_worktree_keeps_its_parent_and_the_last_child_is_marked() {
        let snapshot = worktree_snapshot();
        let entries = super::super::render::workspace_entries(&snapshot, &HashSet::new());
        let shown = |query: &str| {
            let view = FilterView::new(&snapshot, query);
            labels(&snapshot, &view.filter_entries(&snapshot, entries.clone()))
        };
        // A child matched by its name: its parent stays as context.
        assert_eq!(shown("feature"), ["repo", "  repo-feature (last)"]);
        // The parent alone does not pull its children in.
        assert_eq!(
            shown("repo"),
            ["repo", "  repo-feature", "  repo-fix (last)"]
        );
        assert_eq!(shown("notes"), ["notes"]);
        // Both children, and the lone space whose branch is `main`.
        assert_eq!(
            shown("fix"),
            ["repo", "  repo-fix (last)"],
            "the other child does not match"
        );
        assert_eq!(shown("zzz"), Vec::<String>::new());
    }
}
