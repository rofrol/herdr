# local.job

Long work started by an agent (a build in a VM, an export, a test suite) is
invisible when the agent only launches it: the agent shows `idle` and you have
to ask how it is going. `herdr-job` runs the command in its own unfocused tab,
so the output is live one click away, and puts its state in the sidebar of the
pane that started it. The state lives in files, not in the agent, so it works
the same for Claude Code, pi, or any other agent, and for you.

```sh
id=$(herdr-job run --name "Build b17" --why "test the new app set" -- make image)
herdr-job wait "$id"      # follows the log, exits with the command's exit code
herdr-job list            # all jobs: running / ok / failed (code) / lost
herdr-job log "$id"       # log path
herdr-job clean           # close this pane's finished job tabs (--all: everyone's)
```

- The job tab is labelled `⏳ Build b17`, then `✓ Build b17` or `✗ Build b17`.
  After success it closes itself 10 s later (`--keep` leaves it open); after a
  failure it stays open with the output.
- The tab's last row is a pinned footer: state, name, `--why`, which agent and
  workspace started it, and the job id. Output scrolls above it.
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

## Design and limits

- State lives in files, not in the agent, so Claude, pi and a person share it,
  and it survives the agent's context. A rule in the agent's instructions
  alone was not enough: agents forget it, and a launcher that returns at once
  still looks finished.
- Notifications: a job started by hand notifies when it ends; a job started
  by an agent does not, because the agent reports it when `herdr-job wait`
  wakes it, and two notifications for one event are noise. `--notify`
  overrides both.
- Clicking the notification focuses the job tab, or its workspace once the
  tab is closed (herdr's `notification.show_for_pane`; older herdr builds get
  a notification without a click target). A click only moves focus; it never
  runs a command, so a remote server cannot choose what your machine runs.
  Click actions work on macOS with `terminal-notifier`; Linux (D-Bus) is not
  done yet.
- A job is alive while its executor holds an `flock`, not while its PID
  exists: after a crash or reboot the PID can belong to another process and
  `wait` would hang forever.
- macOS and Linux only (`flock`, `/bin/sh`, POSIX signals). State files are
  private (0600): commands and logs may contain secrets.

## TODO: clickable notifications on Linux

On Linux the completion notification is shown (`notify-send`), but clicking
it does nothing: `show_desktop_notification_with_details` in
`src/platform/linux.rs` drops the click action.

Implementation, in the fork's Rust code:

- Send click-capable notifications over D-Bus
  (`org.freedesktop.Notifications.Notify` with a `default` action) through the
  existing `zbus` dependency; `notify-send` stays the fallback.
- Check `GetCapabilities` for `actions` first; some daemons ignore actions.
- One listener per client for `ActionInvoked` and `NotificationClosed`,
  matched by the id `Notify` returns, bounded, and reset when the daemon
  restarts (ids are daemon-local and get reused).
- On `ActionInvoked`, run the same focus commands macOS runs (agent → tab →
  workspace); never a command chosen by the server.
- Raise the terminal window too. Focusing a herdr tab does not bring the
  terminal forward on Wayland, which blocks focus stealing; macOS gets this
  from `terminal-notifier -activate`. Candidates: the `ActivationToken`
  signal (xdg-activation), compositor commands such as
  `hyprctl dispatch focuswindow`.

Testing, so any maintainer can repeat it (no personal VMs):

1. Feasibility first: on a Wayland desktop, send one notification with a
   `default` action (`gdbus call --session --dest org.freedesktop.Notifications
   ... Notify`), watch `gdbus monitor`, click it: does the daemon send
   `ActionInvoked` and `ActivationToken`? This decides the window-raising
   approach.
2. Rust tests on a private session bus (`just test`, Linux CI): a fake
   notification server via `zbus`; command order, no-actions fallback,
   unknown or repeated ids, bounds, daemon restart.
3. A scripted real-daemon test in a container (Docker/OrbStack/CI): headless
   sway + mako, herdr server and client, `makoctl invoke` as the click,
   assert the focused tab. Does not cover window raising.
4. Manual checklist on public images: Fedora Workstation (GNOME), Fedora KDE,
   any Hyprland distribution. Build the fork, run
   `herdr-job run --notify always --name T -- 'sleep 20; false'`, switch to
   another desktop workspace, click; then with `true` (tab closed → herdr
   workspace). Record daemon, version, `actions` capability, whether herdr
   focused the tab and whether the window came forward.

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
long test suites) with
`herdr-job run --name "<short description>" --why "<what it is for>" -- <command>`,
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
