# spec-viewer

<p align="center">
  <a href="https://github.com/antoine-bouteiller/spec-viewer/actions/workflows/ci.yaml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/antoine-bouteiller/spec-viewer/ci.yaml?style=for-the-badge&branch=main&label=ci&logo=github"></a>
  <a href="https://codecov.io/github/antoine-bouteiller/spec-viewer"><img alt="Codecov" src="https://img.shields.io/codecov/c/github/antoine-bouteiller/spec-viewer?style=for-the-badge&logo=codecov"></a>
</p>

A keyboard-first terminal viewer for the plans and specs in a Git worktree. It works over SSH, in a
pane beside an agent, or anywhere a browser is out of reach.

## Usage

```sh
spec-viewer [ROOT] [--host NAME] [--theme light|dark]
spec-viewer daemon [--stdio | --idle-exit SECONDS]
```

- `ROOT` defaults to the current directory.
- `--host` opens `ROOT`, an absolute path, on a configured remote host.
- `--theme` overrides the configured light or dark mode.

Drag over the document to copy its text through OSC 52, without code-block borders; a double click
copies a word.

### Daemon

One daemon per machine reads the corpus for every terminal and browser. `spec-viewer` starts it
when none runs, as `spec-viewer daemon --idle-exit 300`, and it exits five minutes after its last
client. Run `spec-viewer daemon` under launchd or systemd to keep one up. Its socket, lock, and log
live in `$XDG_RUNTIME_DIR/spec-viewer/` (else `$TMPDIR/spec-viewer-<uid>/`), and the projects it
has opened in `$XDG_STATE_HOME/spec-viewer/projects.toml` (else `~/.local/state/spec-viewer/`).

The daemon serves the web app at `http://localhost:4747`, for every project and host; the terminal
footer shows the URL, linked with OSC 8, and `o` opens the current view there. A port held by
another process leaves the daemon without a web app. Mermaid diagrams load from jsDelivr; offline,
their source stays visible.

### Remote hosts

The daemon reaches other machines over SSH and nothing listens there. Each host needs
`spec-viewer` on its `PATH`. Declare hosts in the user config
(`$XDG_CONFIG_HOME/spec-viewer/config.toml`):

```toml
[web]
port = 4747

[[host]]
name = "devbox"
ssh = "devbox"      # runs: ssh -T devbox spec-viewer daemon --stdio
# command = ["kubectl", "exec", "-i", "pod", "--", "spec-viewer", "daemon", "--stdio"]  # instead of ssh
```

A remote clone of a local project, matched by `remote.origin.url`, shows as one more worktree of
it, in the terminal picker and the web dropdown, and follows the local `.spec-viewer.toml`. Remote
documents take the statuses, tabs, and chips of your user config.

### Config

The user config holds every section. A worktree `.spec-viewer.toml` may set only `[[tab]]`,
`[[status]]`, and `[[chip]]`, which replace the user's lists, and edits to either apply without a
restart. `crates/config/default.toml` lists every key.

Each `[[tab]]` shows a path tree of the `.md` and `.mdx` files below its folder. A file belongs to
the tab with the deepest folder containing it. A later layer's list replaces the whole
list; the default is:

```toml
[[tab]]
name = "plans"
folder = ".plan"
companions = ["examples"]   # x.examples.md nests under x.md

[[tab]]
name = "specs"              # no folder: the whole worktree, through Git (ignored files excluded)
kinds = ["spec"]            # list only *.spec.md[x]; omit to list every file
companions = ["examples", "discovery"]
```

Every key is configurable in `[keys]`, in herdr's format, for both the terminal and the web app.
A layer rebinding one action keeps the other defaults; `crates/config/default.toml` lists every action:

```toml
[keys]
quit = ["q", "ctrl+q"]
switch_tab = ["1..9", "alt+1..9"]
hints = []                  # unbind

[keys.search_dialog]
close = ["esc", "ctrl+g"]
```

## Development

```sh
direnv allow          # or: nix develop
cargo run -- .
cargo test
```

The behaviour is specified in [`spec/terminal-ui.spec.md`](spec/terminal-ui.spec.md),
[`spec/web-ui.spec.md`](spec/web-ui.spec.md), and [`spec/daemon.spec.md`](spec/daemon.spec.md).
