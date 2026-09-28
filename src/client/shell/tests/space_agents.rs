use super::*;

fn state_with_agent(agents: bool) -> ClientShellState {
    let mut config = ClientShellConfig::from_config(&Config::default());
    config.spaces.agents = agents;
    let mut state = ClientShellState::new(config);
    let mut projected = snapshot();
    let tokens = vec![("jobs".to_owned(), "1⧖ 2✓".to_owned())];
    projected.agents.push(ClientShellAgent {
        pane_id: "pane_1".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: None,
        display_agent: Some("claude".into()),
        agent: Some("claude".into()),
        title: Some("Fold agents into spaces".into()),
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: AgentStatus::Working,
        state_change_seq: 0,
        state_labels: Vec::new(),
        tokens,
        focused: true,
    });
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state
}

#[test]
fn spaces_list_their_agents_and_jobs_under_them_when_enabled() {
    let mut state = state_with_agent(true);
    let frame = state.compose(106, 30).unwrap();
    let rows = frame_rows(&frame);
    let task = rows
        .iter()
        .position(|row| row.contains("Fold agents into"))
        .expect("agent line under the space");
    assert!(rows[task + 1].contains("1⧖ 2✓"), "{}", rows[task + 1]);
    let space = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.workspace_id == "ws_1")
        .expect("space hit");
    assert!(
        (space.rect.y..space.rect.bottom()).contains(&(task as u16 + 1)),
        "the space's block covers its agent and job lines"
    );
}

#[test]
fn spaces_keep_their_rows_when_disabled() {
    let mut state = state_with_agent(false);
    let frame = state.compose(106, 30).unwrap();
    let rows = frame_rows(&frame);
    let space = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.workspace_id == "ws_1")
        .expect("space hit");
    let space_rows = &rows[space.rect.y as usize..space.rect.bottom() as usize];
    assert!(
        space_rows.iter().all(|row| !row.contains("1⧖ 2✓")),
        "{space_rows:?}"
    );
}

#[test]
fn clicking_an_agent_line_focuses_its_pane() {
    let mut state = state_with_agent(true);
    state.compose(106, 30).unwrap();
    let (rect, pane_id) = state.hits.space_agents[0].clone();
    assert_eq!(pane_id, "pane_1");
    let outcome =
        state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: rect.x + 4,
            row: rect.y,
            modifiers: KeyModifiers::empty(),
        })]);
    assert!(outcome.actions.iter().any(|action| matches!(action,
        ClientShellAction::Endpoint { request, .. }
            if matches!(&request.method, crate::api::schema::Method::PaneFocus(target)
                if target.pane_id == "pane_1"))));
}

fn with_job(
    state: &mut ClientShellState,
    status: AgentStatus,
    job: Option<crate::api::schema::TabStatus>,
) {
    let mut projected = state.snapshot.as_deref().expect("snapshot").clone();
    projected.agents[0].agent_status = status;
    if let Some(job) = job {
        let mut tab = projected.tabs[0].clone();
        tab.tab_id = "tab_job".into();
        tab.label = "build".into();
        tab.focused = false;
        tab.parent_tab_id = Some("tab_1".into());
        tab.status = Some(job);
        projected.tabs.push(tab);
    }
    state.set_snapshot(Box::new(projected));
}

fn agent_icon_color(state: &mut ClientShellState) -> (String, ratatui::style::Color) {
    let frame = state.compose(106, 30).unwrap();
    let buffer = frame.to_ratatui_buffer().expect("buffer");
    let (rect, _) = state.hits.space_agents[0];
    let cell = &buffer[(rect.x + 3, rect.y)];
    (cell.symbol().to_owned(), cell.fg)
}

#[test]
fn an_idle_agent_with_a_running_job_shows_the_waiting_mark() {
    use crate::api::schema::TabStatus;
    let mut state = state_with_agent(true);
    with_job(&mut state, AgentStatus::Idle, Some(TabStatus::Running));
    let mauve = state.config.palette.mauve;
    assert_eq!(agent_icon_color(&mut state), ("●".to_owned(), mauve));

    // A finished job, or a working agent, keeps the usual status.
    let mut state = state_with_agent(true);
    with_job(&mut state, AgentStatus::Idle, Some(TabStatus::Succeeded));
    assert_ne!(agent_icon_color(&mut state).1, mauve);
    let mut state = state_with_agent(true);
    with_job(&mut state, AgentStatus::Working, Some(TabStatus::Running));
    assert_eq!(agent_icon_color(&mut state).1, state.config.palette.yellow);
}

#[test]
fn the_symbols_style_uses_a_clock_for_waiting() {
    use crate::api::schema::TabStatus;
    let mut state = state_with_agent(true);
    state.config.status_indicators = crate::config::StatusIndicatorStyle::Symbols;
    with_job(&mut state, AgentStatus::Done, Some(TabStatus::Running));
    assert_eq!(agent_icon_color(&mut state).0, "◷");
}

#[test]
fn hiding_the_agents_panel_gives_its_height_to_the_spaces() {
    let mut shown = state_with_agent(true);
    shown.compose(106, 30).unwrap();
    let shown_body = shown.hits.workspace_body;
    assert!(!shown.hits.agents.is_empty());

    let mut hidden = state_with_agent(true);
    hidden.config.show_agents_panel = false;
    let frame = hidden.compose(106, 30).unwrap();
    let rows = frame_rows(&frame);
    assert!(hidden.hits.agents.is_empty());
    assert_eq!(hidden.hits.agent_sort_toggle, Rect::default());
    assert_eq!(hidden.hits.sidebar_section_divider, Rect::default());
    assert!(hidden.hits.workspace_body.height > shown_body.height);
    assert!(
        rows.iter().all(|row| !row.starts_with(" agents")),
        "{rows:?}"
    );
}
