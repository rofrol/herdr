use super::*;
use crate::api::schema::TabStatus;

fn state_with_tabs(tabs: bool) -> ClientShellState {
    let mut config = ClientShellConfig::from_config(&Config::default());
    config.spaces.tabs = tabs;
    let mut state = ClientShellState::new(config);
    let mut projected = snapshot();
    projected.tabs[0].label = "agent tab".into();
    projected.tabs[0].agent_status = AgentStatus::Working;
    projected.agents.push(ClientShellAgent {
        pane_id: "pane_1".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: None,
        display_agent: Some("claude".into()),
        agent: Some("claude".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: AgentStatus::Working,
        state_change_seq: 0,
        awaiting_reply: false,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: true,
    });
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state
}

/// Adds a tab nested under `tab_1`, as herdr-job opens its jobs.
fn with_job(state: &mut ClientShellState, tab_id: &str, status: TabStatus) {
    let mut projected = state.snapshot.as_deref().expect("snapshot").clone();
    let mut job = projected.tabs[0].clone();
    job.tab_id = tab_id.into();
    job.label = format!("job {tab_id}");
    job.focused = false;
    job.agent_status = AgentStatus::Unknown;
    job.parent_tab_id = Some("tab_1".into());
    job.status = Some(status);
    projected.tabs.push(job);
    state.set_snapshot(Box::new(projected));
}

fn set_agent_status(state: &mut ClientShellState, status: AgentStatus, awaiting_reply: bool) {
    let mut projected = state.snapshot.as_deref().expect("snapshot").clone();
    projected.tabs[0].agent_status = status;
    projected.agents[0].agent_status = status;
    projected.agents[0].awaiting_reply = awaiting_reply;
    state.set_snapshot(Box::new(projected));
}

#[test]
fn spaces_list_their_tabs_without_nested_jobs_when_enabled() {
    let mut state = state_with_tabs(true);
    with_job(&mut state, "job_1", TabStatus::Running);
    with_job(&mut state, "job_2", TabStatus::Failed);
    with_job(&mut state, "job_3", TabStatus::Succeeded);
    let frame = state.compose(106, 30).unwrap();
    let rows = frame_rows(&frame);
    let sidebar = |row: &String| row.chars().take(28).collect::<String>();
    let line = rows
        .iter()
        .position(|row| sidebar(row).contains("agent tab"))
        .expect("tab line under the space");
    assert!(sidebar(&rows[line]).contains("⧖ 1 !1"), "{}", rows[line]);
    assert!(
        rows.iter().all(|row| !sidebar(row).contains("job job_")),
        "nested job tabs get no line: {rows:?}"
    );
    let space = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.workspace_id == "ws_1")
        .expect("space hit");
    assert!(
        (space.rect.y..space.rect.bottom()).contains(&(line as u16)),
        "the space's block covers its tab lines"
    );
    assert!(
        rows[space.rect.y as usize..line]
            .iter()
            .all(|row| !sidebar(row).contains("!1")),
        "the counts are not repeated on the space row"
    );
}

#[test]
fn spaces_keep_their_rows_when_disabled() {
    let mut state = state_with_tabs(false);
    state.compose(106, 30).unwrap();
    assert!(state.hits.space_tabs.is_empty());
}

#[test]
fn clicking_a_tab_line_enters_its_groups_last_focused_tab() {
    let mut state = state_with_tabs(true);
    with_job(&mut state, "job_1", TabStatus::Running);
    let click = |state: &mut ClientShellState| {
        state.compose(106, 30).unwrap();
        let (rect, _) = state.hits.space_tabs[0].clone();
        state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: rect.x + 4,
            row: rect.y,
            modifiers: KeyModifiers::empty(),
        })])
    };
    let focuses = |outcome: &ClientShellInput, tab: &str| {
        outcome.actions.iter().any(|action| {
            matches!(action,
            ClientShellAction::Endpoint { request, .. }
                if matches!(&request.method, crate::api::schema::Method::TabFocus(target)
                    if target.tab_id == tab))
        })
    };

    assert!(focuses(&click(&mut state), "tab_1"));

    // After the job tab had focus, the line returns to it.
    let mut state = state_with_tabs(true);
    with_job(&mut state, "job_1", TabStatus::Running);
    let mut projected = state.snapshot.as_deref().expect("snapshot").clone();
    for tab in &mut projected.tabs {
        tab.focused = tab.tab_id == "job_1";
    }
    projected.focused_tab_id = Some("job_1".into());
    projected.workspaces[0].active_tab_id = "job_1".into();
    state.set_snapshot(Box::new(projected));
    assert!(focuses(&click(&mut state), "job_1"));
}

/// The state icon of the first tab line and its colour.
fn tab_icon_color(state: &mut ClientShellState) -> (String, ratatui::style::Color) {
    let (symbol, fg, _) = tab_cell(state, 3);
    (symbol, fg)
}

/// The cell `column` columns into the first tab line: its symbol and colours.
fn tab_cell(
    state: &mut ClientShellState,
    column: u16,
) -> (String, ratatui::style::Color, ratatui::style::Color) {
    let frame = state.compose(106, 30).unwrap();
    let buffer = frame.to_ratatui_buffer().expect("buffer");
    let (rect, _) = state.hits.space_tabs[0];
    let cell = &buffer[(rect.x + column, rect.y)];
    (cell.symbol().to_owned(), cell.fg, cell.bg)
}

#[test]
fn only_the_focused_spaces_active_tab_line_is_blue() {
    let mut state = state_with_tabs(true);
    with_job(&mut state, "job_1", TabStatus::Running);
    let mut projected = state.snapshot.as_deref().expect("snapshot").clone();
    let mut other = projected.tabs[0].clone();
    other.tab_id = "tab_2".into();
    other.focused = false;
    other.agent_status = AgentStatus::Unknown;
    projected.tabs.push(other);
    state.set_snapshot(Box::new(projected));
    let palette = state.config.palette.clone();
    let frame = state.compose(106, 30).unwrap();
    let buffer = frame.to_ratatui_buffer().expect("buffer");
    let cell = |line: usize, column: u16| {
        let (rect, _) = state.hits.space_tabs[line];
        buffer[(rect.x + column, rect.y)].clone()
    };
    let (rect, _) = state.hits.space_tabs[0];
    let last = rect.width - 3;

    // The focused space's active tab is accent-tinted, the other tab a
    // lighter grey, and neither fill reaches the state icon, which keeps
    // its colour.
    let tint = cell(0, 5).bg;
    let inactive = cell(1, 5).bg;
    assert_ne!(tint, palette.accent);
    assert_ne!(tint, inactive);
    assert_ne!(cell(0, 4).bg, tint);
    assert_ne!(cell(1, 4).bg, inactive);
    assert_eq!(cell(0, 3).fg, palette.yellow);
    // On the tint the job count keeps its colour.
    assert_eq!(cell(0, last).fg, palette.yellow);
    assert_eq!(cell(0, last).bg, tint);

    // A tab without an agent gets the program mark.
    assert_eq!(cell(1, 3).symbol(), "❏");

    // In a space that is not focused the active tab is grey, not blue.
    let mut projected = state.snapshot.as_deref().expect("snapshot").clone();
    projected.workspaces[0].focused = false;
    state.set_snapshot(Box::new(projected));
    let (_, _, active_elsewhere) = tab_cell(&mut state, 5);
    assert_ne!(active_elsewhere, tint);
    assert_ne!(active_elsewhere, inactive);
}

#[test]
fn an_idle_tab_with_a_running_job_shows_the_waiting_mark() {
    let mut state = state_with_tabs(true);
    set_agent_status(&mut state, AgentStatus::Idle, false);
    with_job(&mut state, "job_1", TabStatus::Running);
    let mauve = state.config.palette.mauve;
    assert_eq!(tab_icon_color(&mut state), ("●".to_owned(), mauve));

    // A finished job, or a working agent, keeps the usual status.
    let mut state = state_with_tabs(true);
    with_job(&mut state, "job_1", TabStatus::Succeeded);
    assert_ne!(tab_icon_color(&mut state).1, mauve);
    let mut state = state_with_tabs(true);
    with_job(&mut state, "job_1", TabStatus::Running);
    assert_eq!(tab_icon_color(&mut state).1, state.config.palette.yellow);
}

#[test]
fn a_tab_awaiting_a_reply_shows_a_question_mark_over_a_running_job() {
    let mut state = state_with_tabs(true);
    set_agent_status(&mut state, AgentStatus::Done, true);
    with_job(&mut state, "job_1", TabStatus::Running);
    let done = super::super::status_color(AgentStatus::Done, &state.config.palette);
    assert_eq!(tab_icon_color(&mut state), ("?".to_owned(), done));
}

#[test]
fn a_tab_with_an_agent_awaiting_a_reply_shows_a_question_mark_in_the_tab_bar() {
    let mut state = state_with_tabs(true);
    set_agent_status(&mut state, AgentStatus::Done, true);
    let frame = state.compose(106, 30).unwrap();
    let buffer = frame.to_ratatui_buffer().expect("buffer");
    let bar = state.hits.tab_bar;
    let row: String = (bar.x..bar.right())
        .map(|x| buffer[(x, bar.y)].symbol().to_owned())
        .collect();
    assert!(row.contains("? "), "{row:?}");
}

#[test]
fn the_symbols_style_uses_a_clock_for_waiting() {
    let mut state = state_with_tabs(true);
    state.config.status_indicators = crate::config::StatusIndicatorStyle::Symbols;
    set_agent_status(&mut state, AgentStatus::Done, false);
    with_job(&mut state, "job_1", TabStatus::Running);
    assert_eq!(tab_icon_color(&mut state).0, "◷");
}

#[test]
fn the_spaces_chevron_hides_and_shows_its_tab_lines() {
    let mut state = state_with_tabs(true);
    state.compose(106, 30).unwrap();
    assert_eq!(state.hits.space_tabs.len(), 1);
    let click_chevron = |state: &mut ClientShellState| {
        let (rect, _) = state
            .hits
            .workspaces
            .iter()
            .find(|hit| hit.workspace_id == "ws_1")
            .and_then(|hit| hit.group_toggle.clone())
            .expect("chevron");
        let events = vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::empty(),
        })];
        state.handle_raw_events(events);
        state.compose(106, 30).unwrap();
    };

    // In front of the name, like tree-style tab lists.
    let frame = state.compose(106, 30).unwrap();
    let rows = frame_rows(&frame);
    let space = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.workspace_id == "ws_1")
        .expect("space hit");
    let (toggle, _) = space.group_toggle.clone().expect("chevron");
    let name_line = rows[toggle.y as usize].chars().collect::<Vec<_>>();
    assert_eq!(name_line[toggle.x as usize], '▼', "{name_line:?}");
    assert_eq!(toggle.x, space.rect.x + 1);

    click_chevron(&mut state);
    assert!(state.hits.space_tabs.is_empty());
    let frame = state.compose(106, 30).unwrap();
    assert_eq!(
        frame_rows(&frame)[toggle.y as usize]
            .chars()
            .nth(toggle.x as usize),
        Some('►')
    );
    click_chevron(&mut state);
    assert_eq!(state.hits.space_tabs.len(), 1);
}

#[test]
fn middle_and_right_click_on_a_tab_line_target_the_tab_not_its_space() {
    let mut state = state_with_tabs(true);
    state.config.confirm_close = false;
    with_job(&mut state, "job_1", TabStatus::Succeeded);
    let mut projected = state.snapshot.as_deref().expect("snapshot").clone();
    let mut other = projected.tabs[0].clone();
    other.tab_id = "tab_2".into();
    other.focused = false;
    projected.tabs.push(other);
    // The job tab had focus last; the line still stands for its group.
    for tab in &mut projected.tabs {
        tab.focused = tab.tab_id == "job_1";
    }
    projected.focused_tab_id = Some("job_1".into());
    projected.workspaces[0].active_tab_id = "job_1".into();
    state.set_snapshot(Box::new(projected));
    state.compose(106, 30).unwrap();
    let (line, _) = state.hits.space_tabs[0];
    let space = state.hits.workspaces[0].rect;
    assert!(
        space.y < line.y,
        "the space's name line sits above its tabs"
    );
    let press = |state: &mut ClientShellState, button, (column, row): (u16, u16)| {
        state.overlay = None;
        state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(button),
            column,
            row,
            modifiers: KeyModifiers::empty(),
        })])
    };
    let closes = |outcome: &ClientShellInput| {
        outcome
            .actions
            .iter()
            .filter_map(|action| match action {
                ClientShellAction::Endpoint { request, .. } => match &request.method {
                    crate::api::schema::Method::TabClose(target) => {
                        Some(format!("tab {}", target.tab_id))
                    }
                    crate::api::schema::Method::WorkspaceClose(params) => {
                        Some(format!("space {}", params.workspace_id))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let middle = crossterm::event::MouseButton::Middle;
    let right = crossterm::event::MouseButton::Right;

    let outcome = press(&mut state, middle, (line.x + 4, line.y));
    // The agent makes both closes ask first; the tab line asks for its whole
    // group, the name line for the space.
    assert!(closes(&outcome).is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ConfirmClose(ref confirm))
            if confirm.tab_target.as_ref().is_some_and(|target| {
                target.tab_id == "tab_1" && target.children == ["job_1"]
            })
    ));
    let outcome = press(&mut state, middle, (space.x + 2, space.y));
    assert!(closes(&outcome).is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ConfirmClose(ref confirm))
            if confirm.workspace_id == "ws_1" && confirm.tab_target.is_none()
    ));

    press(&mut state, right, (line.x + 4, line.y));
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
            target: ClientContextMenuTarget::Tab { ref tab_id, .. },
            ..
        })) if tab_id == "tab_1"
    ));
    press(&mut state, right, (space.x + 2, space.y));
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
            target: ClientContextMenuTarget::Workspace { ref workspace_id, .. },
            ..
        })) if workspace_id == "ws_1"
    ));
}
