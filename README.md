# spec-viewer

<p align="center">
  <a href="https://github.com/antoine-bouteiller/spec-viewer/actions/workflows/ci.yaml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/antoine-bouteiller/spec-viewer/ci.yaml?style=for-the-badge&branch=main&label=ci&logo=github"></a>
  <a href="https://codecov.io/github/antoine-bouteiller/spec-viewer"><img alt="Codecov" src="https://img.shields.io/codecov/c/github/antoine-bouteiller/spec-viewer?style=for-the-badge&logo=codecov"></a>
</p>

A keyboard-first terminal viewer for the plans and specs in a Git worktree. It works over SSH, in a
pane beside an agent, or anywhere a browser is out of reach.

## Usage

```sh
spec-viewer [ROOT] [--theme light|dark] [--config PATH]
```

- `ROOT` defaults to the current directory.
- `--theme` overrides the configured light or dark mode.
- `--config` adds a TOML config layer on top of the user and worktree configs.

The same session serves the corpus as a web app on `127.0.0.1`; the footer shows its URL, and `o`
opens the current view there. The first
port the OS assigns is stored per project in `$XDG_CONFIG_HOME/spec-viewer/ports.toml` (else
`~/.config/spec-viewer/ports.toml`) and reused by later sessions, unless another process holds it.
Mermaid diagrams load from jsDelivr; offline, their source stays visible.

To reach the web app from another machine, set `[web]` in the user config
(`$XDG_CONFIG_HOME/spec-viewer/config.toml`); worktree and `--config` files may not set it:

```toml
[web]
bind = "0.0.0.0"   # an IP, or a hostname to bind all interfaces
cert = "cert.pem"  # optional, with key: serve HTTPS; relative to the config file
key = "key.pem"
```

The server has no authentication, so only bind beyond loopback on a trusted network.

Each `[[tab]]` shows a path tree of the `.md` and `.mdx` files below its folder. A file belongs to
the tab with the deepest folder containing it. A worktree `.spec-viewer.toml` replaces the whole
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

## Development

```sh
direnv allow          # or: nix develop
cargo run -- .
cargo test
```

The behaviour is specified in [`spec/terminal-ui.spec.md`](spec/terminal-ui.spec.md) and
[`spec/web-ui.spec.md`](spec/web-ui.spec.md).
