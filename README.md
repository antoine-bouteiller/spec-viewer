# spec-viewer

A keyboard-first terminal viewer for the plans and specs in a Git worktree. It works over SSH, in a
pane beside an agent, or anywhere a browser is out of reach.

## Usage

```sh
spec-viewer [ROOT] [--theme light|dark] [--config PATH]
```

- `ROOT` defaults to the current directory.
- `--theme` overrides the configured light or dark mode.
- `--config` adds a TOML config layer on top of the user and worktree configs.

## Development

```sh
direnv allow          # or: nix develop
cargo run -- .
cargo test
```

The behaviour is specified in [`spec/terminal-ui.spec.md`](spec/terminal-ui.spec.md).
