# local.job

Long work started by an agent (a build in a VM, an export, a test suite) is
invisible when the agent only launches it: the agent shows `idle` and you have
to ask how it is going. `herdr-job` runs the command in its own unfocused tab,
so the output is live one click away, and puts its state in the sidebar of the
pane that started it. The state lives in files, not in the agent, so it works
the same for Claude Code, pi, or any other agent, and for you.

```sh
id=$(herdr-job run --name "Build b17" -- make image)   # returns at once
herdr-job wait "$id"      # follows the log, exits with the command's exit code
herdr-job list            # all jobs: running / ok / failed (code) / lost
herdr-job log "$id"       # log path
herdr-job clean           # close tabs of finished jobs and forget them
```

- The job tab is labelled `⏳ Build b17`, then `✓ Build b17` or `✗ Build b17`.
- The starting pane gets a `$jobs` token: `⏳ Build b17` (or `⏳ 2 jobs`) while
  running, then `✓ …` or `✗ <code> …` for 30 minutes.
- An agent runs `herdr-job wait <id>` as its background task, so it wakes up
  with the real result instead of "started".
- Closing a job tab stops the job; `wait` then reports `lost` (exit 125) or
  the hangup's exit code (129). A job never runs twice, even if relaunch
  replays its tab after a server restart.
- The command must represent the work: if it only starts something elsewhere
  (a VM, `nohup`, a remote host) and returns, wait for that work inside the
  command (e.g. poll its status file).

State: `~/.local/state/herdr-job/<id>/` (`meta.json`, `log`, `exit`, `lock`).
macOS and Linux only. The state directory must be on a local filesystem:
a job's liveness is an `flock` held by its executor, which network
filesystems may not honour.

## Claude background tasks: `herdr-bg-badge`

A Claude Code `Stop`/`SubagentStop` hook that shows Claude's own background
tasks (from the hook payload's `background_tasks`) as a `$bg` token:
`⏳ <description>` for one task, `⏳ N bg` for several. Payloads are logged to
`~/.local/state/herdr-bg/payloads.jsonl`.

```json
"Stop": [{"hooks": [{"type": "command", "command": "~/.local/bin/herdr-bg-badge", "timeout": 10}]}],
"SubagentStop": [{"hooks": [{"type": "command", "command": "~/.local/bin/herdr-bg-badge", "timeout": 10}]}]
```

## Setup

```sh
herdr plugin link ~/personal_projects/herdr/plugins/job   # or: herdr plugin install rofrol/herdr/plugins/job
ln -s ~/personal_projects/herdr/plugins/job/herdr-job ~/.local/bin/
ln -s ~/personal_projects/herdr/plugins/job/herdr-bg-badge ~/.local/bin/
```

Show the tokens in the sidebar (`~/.config/herdr/config.toml`):

```toml
[ui.sidebar.agents]
rows = [
  ["state_icon", "machine", "workspace", "tab"],
  ["agent"],
  ["$jobs", "$bg"],
]
```

then apply it to the running server with `herdr server reload-config`.

## Tell your agents to use it

Agents only use `herdr-job` if their instructions say so. Add this to the
global instruction file of each agent, so it applies in every project:

| Agent | Global instructions |
|---|---|
| Claude Code | `~/.claude/CLAUDE.md` |
| pi | `~/.pi/agent/AGENTS.md` (or `CLAUDE.md` there) |
| Codex | `~/.codex/AGENTS.md` |

```markdown
# Long-running work

Run work that takes more than a minute (builds, exports, VM or remote jobs,
long test suites) with `herdr-job run --name "<short description>" -- <command>`,
then wait for it in the background with `herdr-job wait <id>`. The command must
block until the work is really done: if it only starts work elsewhere (a VM,
a remote host, a detached process), make it wait for that work, e.g. by polling
its status file. Do not detach it with `nohup` or `&`.
```

Running sessions read the file at start, so restart them or send them the
text. A project whose long work runs somewhere special (a VM console, a
remote builder) should keep a script that runs a command there and blocks
until it finishes, so agents can wrap it: `herdr-job run -- ./vm-run make`.

The `Jobs` popup lists all jobs: open it from the plugin pane list, or bind a
key to `herdr-job list` with `[[keys.command]] type = "popup"`.
