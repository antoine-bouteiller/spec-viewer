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

## Development

```sh
direnv allow          # or: nix develop
cargo run -- .
cargo test
```

The behaviour is specified in [`spec/terminal-ui.spec.md`](spec/terminal-ui.spec.md).
