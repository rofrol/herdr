//! Agents listed under their space (`ui.sidebar.spaces.agents`): per agent a
//! line with its state and task, then a line with its herdr-job counts when
//! it has jobs. The first step of folding the agents panel into spaces; the
//! panel stays until this works in daily use.

use std::collections::HashSet;

use ratatui::{buffer::Buffer, layout::Rect, style::Style};

use super::*;
use crate::protocol::{ClientShellSnapshot, ClientShellWorkspace};

/// The pane metadata token herdr-job reports its counts in, e.g. `⧖ 1 ✓2`.
const JOBS_TOKEN: &str = "jobs";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SpaceAgentLine {
    Agent {
        pane_id: String,
        status: crate::api::schema::AgentStatus,
        waiting: bool,
        text: String,
        focused: bool,
    },
    Jobs {
        pane_id: String,
        jobs: String,
    },
}

impl SpaceAgentLine {
    fn pane_id(&self) -> &str {
        match self {
            Self::Agent { pane_id, .. } | Self::Jobs { pane_id, .. } => pane_id,
        }
    }
}

/// The agent lines shown under `workspace`, or none when the setting is off.
/// A collapsed worktree parent stands for its whole group, so it lists the
/// group's agents.
pub(super) fn space_agent_lines(
    snapshot: &ClientShellSnapshot,
    workspace: &ClientShellWorkspace,
    collapsed_groups: &HashSet<String>,
    config: &ClientShellConfig,
) -> Vec<SpaceAgentLine> {
    if !config.spaces.agents {
        return Vec::new();
    }
    let workspace_ids = super::sidebar::displayed_workspaces(snapshot, workspace, collapsed_groups)
        .map(|workspace| workspace.workspace_id.as_str())
        .collect::<Vec<_>>();
    let mut lines = Vec::new();
    for pane_id in super::agent_sidebar::ordered_agent_pane_ids(snapshot, config.agent_panel_sort) {
        let Some(agent) = snapshot.agents.iter().find(|agent| {
            agent.pane_id == pane_id && workspace_ids.contains(&agent.workspace_id.as_str())
        }) else {
            continue;
        };
        let name = agent
            .display_agent
            .as_deref()
            .or(agent.name.as_deref())
            .or(agent.agent.as_deref())
            .unwrap_or("agent");
        // The reported title wins; otherwise the task title the agent sets as
        // its terminal title (Claude Code summarizes the task there), as on tabs.
        let text = [
            agent.title.as_deref(),
            agent.terminal_title_stripped.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|title| !title.is_empty())
        .map_or_else(|| format!("{name} · no task"), str::to_owned);
        lines.push(SpaceAgentLine::Agent {
            pane_id: agent.pane_id.clone(),
            status: agent.agent_status,
            waiting: waits_on_job(snapshot, &agent.tab_id, agent.agent_status),
            text,
            focused: agent.focused,
        });
        if let Some((_, jobs)) = agent
            .tokens
            .iter()
            .find(|(key, value)| key == JOBS_TOKEN && !value.trim().is_empty())
        {
            lines.push(SpaceAgentLine::Jobs {
                pane_id: agent.pane_id.clone(),
                jobs: jobs.trim().to_owned(),
            });
        }
    }
    lines
}

/// Draws `lines` from the top of `area`, below the space's own rows, and
/// returns each drawn line's rect with its agent's pane, for clicks.
pub(super) fn render_space_agent_lines(
    buffer: &mut Buffer,
    area: Rect,
    lines: &[SpaceAgentLine],
    config: &ClientShellConfig,
) -> Vec<(Rect, String)> {
    let palette = &config.palette;
    let mut hits = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let y = area.y.saturating_add(index as u16);
        if y >= area.bottom() {
            break;
        }
        hits.push((
            Rect::new(area.x, y, area.width, 1),
            line.pane_id().to_owned(),
        ));
        match line {
            SpaceAgentLine::Agent {
                status,
                waiting,
                text,
                focused,
                ..
            } => {
                let x = area.x.saturating_add(3);
                super::render::put_text(
                    buffer,
                    x,
                    y,
                    1,
                    agent_icon(*status, *waiting, config.status_indicators),
                    Style::default().fg(agent_color(*status, *waiting, palette)),
                );
                let text_x = x.saturating_add(2);
                let width = area.right().saturating_sub(text_x).saturating_sub(1);
                super::render::put_text(
                    buffer,
                    text_x,
                    y,
                    width,
                    &truncate(text, width as usize),
                    Style::default().fg(if *focused {
                        palette.text
                    } else {
                        palette.subtext0
                    }),
                );
            }
            SpaceAgentLine::Jobs { jobs, .. } => {
                let x = area.x.saturating_add(5);
                let width = area.right().saturating_sub(x).saturating_sub(1);
                super::render::put_text(
                    buffer,
                    x,
                    y,
                    width,
                    &truncate(jobs, width as usize),
                    Style::default().fg(palette.overlay0),
                );
            }
        }
    }
    hits
}

/// `text` cut to `width` columns, ending in `…` when cut.
fn truncate(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    if unicode_width::UnicodeWidthStr::width(text) <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for character in text.chars() {
        let char_width = character.width().unwrap_or(0);
        if used + char_width + 1 > width {
            break;
        }
        out.push(character);
        used += char_width;
    }
    if width > 0 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::AgentStatus;
    use crate::protocol::ClientShellAgent;

    fn agent(pane_id: &str, workspace_id: &str, title: Option<&str>) -> ClientShellAgent {
        ClientShellAgent {
            pane_id: pane_id.into(),
            workspace_id: workspace_id.into(),
            tab_id: "tab_1".into(),
            name: None,
            display_agent: Some("claude".into()),
            agent: Some("claude".into()),
            title: title.map(str::to_owned),
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: AgentStatus::Working,
            state_change_seq: 0,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: false,
        }
    }

    fn config(agents: bool) -> ClientShellConfig {
        let mut config = ClientShellConfig::from_config(&crate::config::Config::default());
        config.spaces.agents = agents;
        config
    }

    #[test]
    fn lists_the_spaces_agents_with_their_jobs_only_when_enabled() {
        let mut snapshot = super::super::tests::snapshot();
        let mut with_jobs = agent("pane_1", "ws_1", Some("Fix the drop marker"));
        with_jobs.tokens.push(("jobs".into(), "⧖ 1 ✓2".into()));
        snapshot.agents = vec![
            with_jobs,
            agent("pane_2", "ws_1", None),
            agent("pane_3", "elsewhere", Some("not here")),
        ];
        let workspace = snapshot.workspaces[0].clone();

        assert!(
            space_agent_lines(&snapshot, &workspace, &HashSet::new(), &config(false)).is_empty()
        );
        assert_eq!(
            space_agent_lines(&snapshot, &workspace, &HashSet::new(), &config(true)),
            vec![
                SpaceAgentLine::Agent {
                    pane_id: "pane_1".into(),
                    status: AgentStatus::Working,
                    waiting: false,
                    text: "Fix the drop marker".into(),
                    focused: false,
                },
                SpaceAgentLine::Jobs {
                    pane_id: "pane_1".into(),
                    jobs: "⧖ 1 ✓2".into(),
                },
                SpaceAgentLine::Agent {
                    pane_id: "pane_2".into(),
                    status: AgentStatus::Working,
                    waiting: false,
                    text: "claude · no task".into(),
                    focused: false,
                },
            ]
        );
    }

    #[test]
    fn falls_back_to_the_terminal_title_for_the_task() {
        let mut snapshot = super::super::tests::snapshot();
        let mut titled = agent("pane_1", "ws_1", None);
        titled.terminal_title_stripped = Some("Alternatives to herdr".into());
        let mut reported = agent("pane_2", "ws_1", Some("Reported task"));
        reported.terminal_title_stripped = Some("Terminal title".into());
        let mut blank = agent("pane_3", "ws_1", Some(" "));
        blank.terminal_title_stripped = Some("  ".into());
        snapshot.agents = vec![titled, reported, blank];
        let workspace = snapshot.workspaces[0].clone();

        let texts = space_agent_lines(&snapshot, &workspace, &HashSet::new(), &config(true))
            .into_iter()
            .filter_map(|line| match line {
                SpaceAgentLine::Agent { text, .. } => Some(text),
                SpaceAgentLine::Jobs { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            texts,
            ["Alternatives to herdr", "Reported task", "claude · no task"]
        );
    }

    #[test]
    fn listing_agents_drops_the_spaces_own_state_icon() {
        let workspace = super::super::tests::snapshot().workspaces[0].clone();
        let has_icon = |agents: bool| {
            super::super::sidebar::workspace_rows(
                &workspace,
                AgentStatus::Working,
                (0, 0),
                false,
                &config(agents).spaces,
            )
            .iter()
            .flatten()
            .any(|token| matches!(token.kind, crate::ui::ResolvedTokenKind::StateIcon))
        };

        assert!(has_icon(false));
        assert!(!has_icon(true));
    }

    #[test]
    fn truncation_keeps_the_width_and_marks_the_cut() {
        assert_eq!(truncate("Name unnamed tabs after", 10), "Name unna…");
        assert_eq!(truncate("short", 10), "short");
    }
}
