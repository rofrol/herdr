# TODO

- [ ] Do I still need the Agents panel? Maybe an Agents tab next to Spaces,
  sorted by priority by default, where a click switches to the right tab in
  Spaces.
  - Oracles (DeepSeek, GPT-6 Luna, 2026-09-26): keep Agents as an attention
    inbox in a `Spaces | Agents` sidebar tab; Spaces keeps its badges.
    Clicking an agent focuses its pane but the sidebar stays on Agents
    (otherwise every click loses the queue); "reveal in Spaces" separately.
    Priority order must not jump while the pointer is over the list.
    Attention count on the tab label.
- [ ] Claude reports that a new version is available. A herdr menu item that
  restarts Claude instances when possible? How: send the instances a message
  to restart once they finish their work? Same for pi.
  - Oracles: do not ask the agent (it costs context and cannot replace its
    own process); herdr restarts it. Version: record `claude --version` when
    the pane starts, compare with the binary on disk (mtime only as a hint).
  - Restart only when the pane is idle, not blocked, with no draft in the
    input box, no subagents (`SubagentStop` hook) and no jobs; otherwise mark
    it "restart pending". Then `/exit` and resume with the plan from
    `src/agent_resume.rs` (`claude --resume <id>`, `pi --session <path>`),
    one at a time.
  - Menu with a preview: how many idle / working / blocked, pick which.
    Launch flags (permission mode, model, env) must be recorded; resume does
    not restore them. For pi, check that `--session` restores everything.
- [x] Make the tabs consistent (two rows: main `1 ⧖ 1 | lazygit | 3 | +`,
  below it `claude:` and the job tabs).
  - Oracles: only the active main tab gets the filled blue background; the
    active entry in the second row gets an underline or bold (now there are
    two blue highlights at once). The parent entry without the colon and the
    grey block, e.g. `claude` with an icon, then a `│` separator. One rule
    for dimming. A number on every main tab or on none.
  - Done 2026-09-26 as variant D: full accent only on the entry on screen.
