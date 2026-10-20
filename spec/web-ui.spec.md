---
title: Web UI
status: review
author: Antoine Bouteiller
date: 2026-10-19
module: [crates/render-web, crates/config, crates/spec-viewer]
related: [spec/terminal-ui.spec.md]
---

## Why

A terminal session shows one document at a time in monospace. Diagrams, wide tables, and long
specs read better in a browser, and a browser tab can stay open beside an editor. Each
`spec-viewer` session therefore also serves its project's corpus as a web app, on loopback unless
the user config binds elsewhere, with the same corpus, tree, metadata, and live reload as the
terminal. The layout and styling follow the Bun
plan-viewer that preceded this repository. This spec owns the HTML rendering, the HTTP surface, the
browser client, and the per-project port. Discovery, metadata, the tree, and confinement live in the
`source`, `parser`, and `core` crates.

- `[G-1]` A browser view of the same corpus as the terminal, reachable from the running session.
- `[G-2]` A stable URL per project across sessions, so a bookmarked or open tab keeps working.
- `[NG-1]` No editing: the server never writes to the worktree.
- `[NG-2]` No authentication: a non-loopback bind is opt-in from the user config only.
- `[NG-3]` No headless server mode: the web app lives as long as a terminal session.
- `[NG-4]` No full-text search: navigation is the tree plus a fuzzy title and path search.
- `[NG-5]` No offline diagrams: Mermaid loads from a CDN, and offline shows source.

## Design

- `[PI-1]` One core: the server reads `spec_viewer_core::Library` and never re-derives corpus,
  metadata, tree, or confinement logic. It renders the same semantic `Document` as the terminal.
- `[PI-2]` Never blank: a readable document always renders, with degradation shown in place.
- `[PI-3]` Parity by structure: element structure and class names match the Bun viewer, so its
  stylesheet applies unchanged.

| Decision                | Choice                                                                                                                       | Rationale                                                                          |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `[KD-1]` Server         | `tiny_http` on `web.bind` (default `127.0.0.1`), one thread per request; `rustls` (ring) when `web.cert` and `web.key` are set | Synchronous like the rest of the workspace; a thread per SSE stream needs no async |
| `[KD-2]` Client         | One hand-written ES module, `index.html`, and CSS, embedded with `include_str!`                                              | The build stays Cargo-only; no Node or Bun in the flake or CI                      |
| `[KD-3]` Render site    | The server renders HTML; the client injects it                                                                               | Highlighting and the parser already run in Rust                                    |
| `[KD-4]` Highlighting   | `syntect` class spans (`hl-` prefix); each token takes its ANSI slot's palette colour, with rules scoped to the colour mode  | Both modes ship in one response; scoping keeps their specificities from competing  |
| `[KD-5]` Mermaid        | `mermaid@12` ESM imported from jsDelivr on demand, run per diagram, re-run on theme change                                   | Keeps 3 MB of JavaScript out of the binary                                         |
| `[KD-6]` Lifetime       | `spec-viewer [ROOT]` starts the server before the terminal UI; it stops when the session exits                               | One command; the footer shows the URL                                              |
| `[KD-7]` Port           | Stored per project key in the user config dir; first session stores the OS-assigned port; a taken port falls back unrecorded | A stable URL without a registry or coordination between sessions                   |
| `[KD-8]` Status colours | Configured `[[status]]` colours become `--tone-N`; an ANSI slot maps to a design token, a hex colour is used as is           | Statuses stay configurable (`spec/terminal-ui.spec.md` `[CT-4]`) in both surfaces  |
| `[KD-9]` Colour mode    | Light, dark, or system, chosen in the page and kept in `localStorage`                                                        | The browser, not the terminal background, governs the page                         |

## Outcome

- `[SO-1]` Every terminal session serves its project's corpus at a URL, loopback by default, shown
  in the footer; `o` opens the current worktree, tab, and document there. — demonstrated by
  `[VC-5]`, `[VC-6]`, `[VC-8]`
- `[SO-2]` A project keeps the same port across sessions unless another process holds it.
  — demonstrated by `[VC-5]`, `[VC-7]`
- `[SO-3]` Every parser construct, including the MDX dialect components, renders as HTML with the
  design's class names, highlighted code, and Mermaid diagrams. — demonstrated by `[VC-1]`, `[VC-2]`
- `[SO-4]` The API serves only the project's listed worktrees and confined documents, and a corpus
  change reloads the open page. — demonstrated by `[VC-3]`, `[VC-4]`

## Contracts

### [CT-1] HTML rendering

Kind: api · Anchor: `crates/render-web/src/html.rs#to_html`

```rust
pub fn to_html(document: &spec_viewer_core::Document) -> Html;
pub struct Html { pub html: String, pub has_mermaid: bool }
```

| Construct                         | HTML                                                                                                    |
| --------------------------------- | ------------------------------------------------------------------------------------------------------- |
| Headings                          | `<hN id="<parser id>">`                                                                                 |
| Lists, task lists                 | Tight items without `<p>`; `<li class="task-list-item"><input type="checkbox" disabled [checked]>`      |
| GFM tables                        | `<table>` with `text-align` per column                                                                  |
| Code fences                       | `div.md-code[data-lang]` > `<pre class="hl">` with `hl-` spans; lang on the border; else escaped        |
| Mermaid fences                    | `<pre class="mermaid">` holding escaped source; sets `has_mermaid`                                      |
| Xref IDs                          | `<button class="xref-chip" data-kind="<family>" data-ref="<ID>">`                                       |
| Doc links                         | `<a href="…" data-doc="<worktree-relative path>">`                                                      |
| `CodeTabs`                        | `.md-codetabs-wrapper` > `section.md-codetab-panel[data-label]` with an `<h3>` label                    |
| `Comparison` / `Option`           | `section.md-comparison` > `.md-comparison-grid` > `article.md-comparison-option[data-name]`             |
| `DecisionMatrix`                  | `table.md-matrix` with one `td[data-score]`                                                             |
| `AnnotatedDiff` / `MarginNote`    | `figure.md-annotated-diff` with `ol.md-margin-notes` > `li.md-margin-note[data-line]`                   |
| `Flowchart` / `NodeDetail`        | `.md-flowchart` with a Mermaid `<pre>`, then `section.md-node-detail[data-name]`                        |
| `StepRail` / `Step`               | `ol.md-steprail` > `li.md-step[data-title]`, unnumbered, a dot on the rail per step                     |
| Unknown component, `{expression}` | `div.md-unknown[data-component]` (`span` inline), children rendered                                     |
| Lowercase JSX element             | Children rendered without a wrapper                                                                     |
| Raw HTML                          | Passed through unescaped                                                                                |
| Degraded document                 | `div.md-degraded` banner above the plain-markdown fallback                                              |

Text and attribute values are HTML-escaped.

### [CT-2] HTTP surface

Kind: api · Anchor: `crates/render-web/src/server.rs#start`

```rust
/// Binds `<web.bind>:<preferred>`, else any free port, and serves on background threads until exit.
/// A `web.bind` that is not an IP binds all interfaces; `web.cert` and `web.key` enable TLS.
pub fn start(library: Library, config: &Config, preferred: Option<u16>) -> io::Result<SocketAddr>;
```

| Route                          | Response                                                                            |
| ------------------------------ | ----------------------------------------------------------------------------------- |
| `GET /`, `/app.js`, `/app.css` | Embedded client assets                                                              |
| `GET /theme.css`               | Per-mode `--tone-N` status colours and highlighting rules                           |
| `GET /api/projects`            | `{ projects: [{ name, path, worktrees: [{ name, main, path }] }] }`                 |
| `GET /api/keys`                | `{ main, search_dialog }`: action → `[{ key, ctrl, alt, shift, meta, label }]`, `key` as `KeyboardEvent.key`; ranges omitted |
| `GET /api/docs?wt=`            | `{ changed, project, tabs: { name, nodes: Node[] }[] }`; `Node.companion` marks companions |
| `GET /api/search?wt=&q=`       | `{ total, hits: [{ tab, name, path, status?, archived, titleMatches, pathMatches }] }` per `spec/terminal-ui.spec.md` `[CT-6]` |
| `GET /api/doc?wt=&path=`       | `{ path, rootPath, meta, html, toc: [{ id, text, depth }], hasMermaid, degraded }`  |
| `GET /api/events?wt=`          | `text/event-stream`: `data: reload` per debounced change burst, `: ping` every 15 s |

```text
Node   = { name, path?, kind, status?: Status, archived, children?: Node[] }
Status = { name, known, tone }          // tone: "var(--tone-N, …)" or "var(--status-unknown)"
meta   = { title, status?, author?, date?, related, chips: Chip[], lines, legacy, kind? }
Chip   = { kind: "checklist", label, done, total } | { kind: "count", label, count }
```

`wt` must canonicalize to a worktree the project currently lists, else `403`. A document read is
`403` outside the worktree and `404` when missing, per `spec_viewer_core::Library::open`. Other paths
are `404`. Each events stream owns its own watch and drops it when a write fails.

### [CT-3] Client

Kind: type · Anchor: `crates/render-web/assets/app.js`

```text
┌ sidebar ───────────────┬ rail ──────────┬ page ─────────────────────────────────┐
│ Specs.  light dark sys │ Files          │ ▔▔▔▔▔ status-coloured bar ▔▔▔▔▔▔▔▔▔▔▔▔ │
│ search all tabs [Diff] │  index         │ Title  [status]   AC 3/5 ▮▮▯  N lines │
│ worktree ▾ (main)      │  phase-1       │ kind · owner · N phases · author · date│
│ [plans] specs          │   Why          │ related: …   path [path] [name]        │
│ ▸ doc/architecture     │   Design       │ rendered body                          │
└────────────────────────┴────────────────┴───────────────────────────────────────┘
```

```text
URL state: ?wt=<worktree path>&tab=<tab name>&doc=<worktree-relative path>&diff=true
```

The tree, Diff, family rail, and header follow `spec/terminal-ui.spec.md` `[CT-2]`. Keys come
from `/api/keys`, the terminal's `[keys]` config (`spec/terminal-ui.spec.md` `[CT-4]`); a single
character matches whatever Shift state produced it. `keys.search` (default `/`, Ctrl-K, Cmd-K)
outside a text field, or a click on the search line, opens a modal dialog that queries
`/api/search` on each input, so its ranking is the terminal's `[CT-6]`. In the dialog
`keys.search_dialog` applies: `up` and `down` move, `open` or a click opens the hit in its tab,
`clear` clears, and `close` or a click outside closes; Esc closes only while `close` binds it.
The search line's key and the dialog footer show the first key of each action. A reload event
reruns an open search. A missing or unlisted `wt` selects the main worktree; a missing `wt` keeps
`doc`. A focused tree row moves with `keys.up` and `keys.down`, expands and collapses with
`keys.right` and `keys.left`, and opens with `keys.open`; the defaults add `j`, `k`, `l`, and `h` to the arrows. The rail highlights the
section under the top of the page. A click on an xref chip copies its ID; a click on a doc link opens
its document. A reload event refetches the corpus and the open document, keeping the scroll
position. A diagram that fails to render, or a CDN that fails to load, leaves its source visible.

### [CT-4] Port file

Kind: config · Anchor: `crates/config/src/ports.rs#Ports`

```toml
# $XDG_CONFIG_HOME/spec-viewer/ports.toml (else ~/.config/spec-viewer/ports.toml)
"/abs/path/to/project/.git" = 41873   # key: spec_viewer_core::Project::key
```

```rust
impl Ports {
    pub fn user() -> Option<Self>;
    pub fn at(file: impl Into<PathBuf>) -> Self;
    pub fn get(&self, project: &Path) -> Option<u16>;   // missing or malformed file, or 0 / out of range: None
    pub fn set(&self, project: &Path, port: u16) -> io::Result<()>; // keeps other entries; refuses a malformed file
}
```

### [CT-5] Session lifecycle

Kind: invariant · Anchor: `crates/spec-viewer/src/main.rs#serve`

After the config loads, the session builds a second `Library` for the server and starts it on the
stored port. A missing stored port records the OS-assigned one; a taken stored port serves elsewhere
and leaves the file unchanged; a write failure is ignored. A server that cannot bind at all exits 1
with `spec-viewer: cannot start web server: <error>`. The URL is `https` with a certificate, else
`http`; its host is the configured hostname, `localhost` for an all-interfaces IP, else the bound
address. The terminal footer appends `o <url>`, and `o` opens the current view per
`spec/terminal-ui.spec.md` `[CT-2]`.

### [CT-6] Web config

Kind: config · Anchor: `crates/config/default.toml`

```toml
[web]                 # only in $XDG_CONFIG_HOME/spec-viewer/config.toml
bind = "127.0.0.1"    # an IP, or a hostname: binds 0.0.0.0, since a hostname often resolves to loopback
cert = "cert.pem"     # optional PEM chain; requires key; relative to the user config file
key = "key.pem"       # optional PEM private key; requires cert
```

`[web]` in a worktree `.spec-viewer.toml` or a `--config` file, an empty `bind`, or `cert` without
`key` (or the reverse) is a config error naming the file and key. An unreadable or invalid PEM file
fails the server start per `[CT-5]`.

## Acceptance

- `[VC-1]` Given markdown with headings, a doc link, bare and bracketed xrefs, a task list, and an
  aligned table, then the HTML carries heading ids, `data-doc`, xref chips with their family, checkbox
  items, and per-column alignment. — demonstrates `[SO-3]`
- `[VC-2]` Given `fixtures/dialect.spec.mdx`, then every `[CT-1]` component renders with its class
  and data attribute, both Mermaid fences are `<pre class="mermaid">` with `has_mermaid`, bash is
  highlighted, and the unknown lexer stays escaped. A document that fails MDX parsing starts with the
  degraded banner. — demonstrates `[SO-3]`
- `[VC-3]` Given a served Git worktree, then `/`, `/theme.css`, `/api/projects`, `/api/keys`, `/api/docs`,
  `/api/search`, and `/api/doc` return the `[CT-2]` shapes, a path escaping the worktree is `403`, a missing document is
  `404`, and an unlisted `wt` is `403`. — demonstrates `[SO-4]`
- `[VC-4]` Given an open events stream, when a `.plan/*.md` file is written, then the stream sends
  `data: reload` within 5 s. — demonstrates `[SO-4]`
- `[VC-5]` Given a free preferred port, then the server binds it; given a taken one, it binds another.
  Given an empty port file, the first session stores its port; a second session while the first runs
  serves elsewhere and leaves the stored port unchanged. — demonstrates `[SO-1]`, `[SO-2]`
- `[VC-6]` Given a session with a web URL and an open plan, then the link `o` opens carries `wt`,
  `tab`, and `doc`; without a web URL, `o` reports that the web app is not running.
  — demonstrates `[SO-1]`
- `[VC-8]` Given a user config with `bind = "0.0.0.0"` and a certificate, then the server listens on
  all interfaces and answers HTTPS; `[web]` in a worktree or `--config` file, or a lone `cert`, is a
  config error; relative PEM paths resolve against the user config file. — demonstrates `[SO-1]`
- `[VC-7]` Given a port file, then `set` keeps other projects' entries, a malformed file yields no
  port and is not overwritten, and an out-of-range or non-integer value yields no port.
  — demonstrates `[SO-2]`

The client's layout, keyboard navigation, theme switching, and Mermaid rendering are qualified
manually in a browser.

## Caveats

- `[C-1]` Any local process or page can call the API: the server has no authentication, and raw
  HTML in a document runs in the page. Documents come from the user's own worktrees.
- `[C-2]` The client needs a browser with CSS nesting and ES modules.
- `[C-3]` `o` runs `open` on macOS and `xdg-open` elsewhere; over SSH there is usually no opener, and
  the footer message shows the link instead.
- `[C-4]` Two concurrent sessions of one project serve on different ports; only the first holds the
  stored one.
