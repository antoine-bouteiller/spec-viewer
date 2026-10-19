---
title: Terminal UI
status: amended
author: Antoine Bouteiller
date: 2026-10-16
module: [crates/render-tui, crates/config, crates/spec-viewer]
related: [spec/web-ui.spec.md]
---

## Why

`spec-viewer [ROOT]` shows a worktree's plans and specs in a terminal: from an SSH session, in a
pane beside an agent, or anywhere a browser is out of reach. This spec owns that surface: the
screen, the keys, the terminal rendering of the semantic document, the theme, and the configured
statuses and header chips. Discovery, metadata, the tree, and confinement live in the `source`,
`parser`, and `core` crates; this spec states the behaviour the screen relies on.

- `[NG-1]` No image diagrams: Mermaid renders as box-drawing text, or as source when it fails.
- `[NG-2]` No mouse interaction beyond wheel scrolling.
- `[NG-3]` No editing: the viewer never writes to the worktree.
- `[NG-4]` No config hot reload: the config is read once per session.

## Design

- `[PI-1]` Keyboard first: every action has a key, and the screen shows a worktree's whole corpus.
- `[PI-2]` One core: the terminal reads `spec_viewer_core::Library` and never re-derives corpus,
  metadata, tree, or confinement logic.
- `[PI-3]` Colours name meanings, not values: every coloured element takes a role, a status, or an
  xref family, and each of those names a palette colour.

| Decision                  | Choice                                                                                                                                                                                                    | Rationale                                                                                                 |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| `[KD-1]` Stack            | `ratatui` on `crossterm`                                                                                                                                                                                  | `TestBackend` makes every frame snapshot-testable                                                         |
| `[KD-2]` Default worktree | The worktree containing `ROOT`, by longest path-prefix match                                                                                                                                              | A terminal user starts where they are working                                                             |
| `[KD-3]` Clipboard        | OSC 52 (`ESC ]52;c;<base64> BEL`)                                                                                                                                                                         | Works over SSH and inside tmux                                                                            |
| `[KD-4]` In-body actions  | Hint mode labels every visible doc link and xref; a label opens the target document or copies the ID                                                                                                      | Reaches in-body targets without a pointer                                                                 |
| `[KD-5]` Theme model      | A theme is a palette of named colours. The built-in `terminal` defines the 16 ANSI slot names, each the terminal's own colour. Roles name palette colours; code tokens take the colour of their ANSI slot | Changing one palette colour recolours every role naming it; the default emits ANSI only, except `surface` |
| `[KD-6]` Mode             | `--theme`, else `theme.mode`; `auto` uses the OSC 11 background reply (120 ms), else dark. The mode picks `light_name` or `dark_name`                                                                     | The terminal background governs light and dark                                                            |
| `[KD-7]` Outline depth    | The rail outline lists the document `toc` (h1–h3); deeper headings are styled but not rail entries                                                                                                        | Keeps the rail short                                                                                      |
| `[KD-8]` Config layers    | TOML, `deny_unknown_fields`: embedded default < user < worktree < `--config`. Tables merge key by key; arrays (`status`, `chip`, `theme.roles.headings`) replace the lower layer wholesale                | Element-wise array merging would make removing a default status or chip impossible                        |
| `[KD-9]` Worktree config  | `.spec-viewer.toml` is read once at start-up from the worktree containing `ROOT`; switching worktree does not reload it                                                                                   | One config per session (`[NG-4]`)                                                                         |

## Outcome

- `[SO-1]` `spec-viewer` in any worktree shows, navigable by keyboard, the worktree picker, tabs,
  corpus tree with status chips, filter, `Diff`, structure rail, document header, and rendered
  body. — demonstrated by `[VC-1]`, `[VC-2]`
- `[SO-2]` A corpus change refreshes the tree and the open document, keeping its scroll position.
  — demonstrated by `[VC-3]`
- `[SO-3]` A user or project configures statuses, header chips, and the theme through layered
  TOML. — demonstrated by `[VC-4]`, `[VC-5]`, `[VC-6]`, `[VC-7]`

## Contracts

### [CT-1] Terminal rendering

Kind: api · Anchor: `crates/render-tui/src/terminal.rs#to_terminal`

```rust
pub fn to_terminal(document: &spec_viewer_core::Document, width: u16, theme: &Theme) -> TermDoc;
pub struct TermDoc { lines: Vec<Line<'static>>, toc: Vec<(TocEntry, usize)>, targets: Vec<TargetSpan> }
pub struct TargetSpan { target: Target, line: usize, column: usize, length: usize } // terminal columns
pub enum Target { Doc(String), Xref(String) }
```

| Construct                      | Terminal form                                                     |
| ------------------------------ | ----------------------------------------------------------------- |
| Headings                       | Bold, coloured by `roles.headings[depth - 1]` (last entry reused) |
| Paragraphs, lists, task lists  | Wrapped to width; `[x]`/`[ ]` markers                             |
| GFM tables                     | Box-drawn in `muted`, cells wrapped; stacked when too narrow      |
| Code fences                    | Highlighted by ANSI slot, boxed on `code_bg`, border in `muted`   |
| Mermaid fences                 | Box-drawn diagram labelled `mermaid`; source on error or overflow |
| Inline code                    | `inline_code`                                                     |
| Xref IDs                       | `roles.xref.<family>`, family `g ng pi kd so ct vc c oq ex other` |
| Doc links                      | Underlined hint targets                                           |
| `CodeTabs`                     | Each tab as a labelled code block                                 |
| `Comparison` / `Option`        | Titled cards side by side; bordered blocks when too narrow        |
| `DecisionMatrix`               | Table carrying the score                                          |
| `AnnotatedDiff` / `MarginNote` | Diff block followed by numbered notes, line labels `annotation`   |
| `Flowchart` / `NodeDetail`     | Mermaid diagram, then the node details as cards                   |
| `StepRail` / `Step`            | `○` title per step on a `│` rail                                  |
| Unknown component              | `warning` box `Unsupported <Name>`; inline `{expr}`, `<Name>…`    |
| Raw HTML                       | Dimmed source text                                                |
| Degraded document              | `warning` banner line above the plain-markdown fallback           |

### [CT-2] Screen and keys

Kind: type · Anchor: `crates/render-tui/src/tui/app.rs#App`

```text
┌ sidebar or rail ───────┬ document ──────────────────────────────┐
│ worktree ▾ (main)      │ Title  [status]                         │
│ [plans] specs   Diff   │ AC 3/5 ▮▮▯ T 2/9 ▮▯▯  N lines           │
│ / filter               │ kind · owner · N phases · author · date │
│ ▸ doc/architecture     │ related: …   path  y path · Y name      │
│   spec title  [chip]   │ rendered body                           │
└────────────────────────┴─────────────────────────────────────────┘
```

```text
state: worktree, tab, doc, filter, diff, expanded, navigator, focus (sidebar|rail|document), scroll
Tab / S-Tab   toggle focus           1 / 2    plans / specs (clears doc)
w             worktree picker (clears doc)     /        filter; Esc clears
d             Diff toggle; specs tab only, disabled when changed specs are unknown
↑↓ jk         move   → ← lh  expand / collapse   Enter  open
PgUp PgDn g G scroll document        f        hint mode (open link / copy xref)
y / Y         copy absolute path / file name   e        sidebar / rail (open document only)
o             open the current view in the web app (spec/web-ui.spec.md)
q / C-c       quit
```

One pane left of the document shows either the sidebar (the corpus tree) or the rail, never both.
Opening a document shows the rail; `e` swaps them, and `/` or clearing the document shows the
sidebar. A live reload keeps the shown pane and focus.

Tree semantics: the specs tab nests by path, compacts single-child directory chains, nests a spec
under the document its `parent-spec` resolves to, and lets an umbrella own a directory it fully
explains. Companions (`<stem>.discovery.*`, `<stem>.examples.*`) sit under their spec or plan. The
plans tab is flat, with a plan index owning its phase files. A non-empty filter or `Diff` shows a
flat list with paths and omits attached examples. A document row never expands. A family member
keeps its owner row current, and ancestors of the open document expand. Filter and `Diff` persist
across tab and worktree switches; a switch clears the document and expansion. An archived document
shows an `archived` chip in `status_unknown`. The header kind is the document's tree kind (`plan`,
`phase`, `umbrella`, `leaf`, `discovery`, `examples`). The rail lists the nearest family's files
when the document belongs to one, then the document's outline.

Header chips: each `[[chip]]` is matched against the document's `##` sections, the first listed
section present winning. A `checklist` chip renders as `<label> done/total` with a meter on the
metrics line, before `N lines`, in `meter_progress` or, when complete, `meter_complete`; it is
omitted when its total is zero. A `headings` chip counts the section's `###` headings and renders
as `N <label>` in the details line; it is omitted when zero.

Statuses: the status head (trimmed, lower-cased, cut at `;`, `,`, `—`, or `-`) matches a
`[[status]]` name or alias case-insensitively and shows as `[<name>]` in that entry's colour. An
unmatched head shows as-is in `status_unknown`. Focus borders, the current row, and the active
rail entry use `accent`. Unfocused borders, the filter, paths, header details, and the footer use
`muted`. Hint labels use `hint_fg` on `hint_bg`.

### [CT-3] Live reload

Kind: invariant · Anchor: `crates/render-tui/src/tui/mod.rs#start`

Each debounced burst of `spec_viewer_core::Library::watch` refreshes the corpus and reopens the
current document at its scroll position. The watch follows the selected worktree. Raw mode, mouse
capture, and the alternate screen are restored on exit and on panic.

### [CT-4] Config

Kind: config · Anchor: `crates/config/default.toml`

`crates/config/default.toml` is the default layer and the reference for every key. Lower layers
first: default < `$XDG_CONFIG_HOME/spec-viewer/config.toml` (else `~/.config/spec-viewer/config.toml`)
< `<worktree containing ROOT>/.spec-viewer.toml` < `--config PATH`.

| Key                                     | Meaning                                                                                                                                                                                              |
| --------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `theme.mode`                            | `auto`, `light`, or `dark`                                                                                                                                                                           |
| `theme.name`, `light_name`, `dark_name` | Built-in theme, per mode when set: `terminal`                                                                                                                                                        |
| `theme.palette`, `theme.palette.<mode>` | Name → colour: `#rrggbb`, `rgb(r,g,b)`, an ANSI slot name, or `reset`. New names allowed. Applied over the built-in, mode table last                                                                 |
| `theme.roles.*`, `theme.roles.xref.*`   | Palette names for `accent`, `muted`, `inline_code`, `code_bg`, `annotation`, `warning`, `headings`, `meter_progress`, `meter_complete`, `hint_fg`, `hint_bg`, `status_unknown`, and each xref family |
| `[[status]]`                            | `name`, optional `aliases`, `color` (palette name); names and aliases unique across entries                                                                                                          |
| `[[chip]]`                              | `label`, `kind` (`checklist` or `headings`), `sections` (`##` names, any case)                                                                                                                       |
| `[web]`                                 | User config only: `bind`, `cert`, `key` per `spec/web-ui.spec.md` `[CT-6]`                                                                                                                          |

A palette value that is an ANSI slot name means the terminal's own colour for that slot, not
another palette entry: palette values do not chain. Under `mode = "auto"`, every role and status
must resolve in both the light and the dark palette, because the mode is known only after start-up.

A missing user or worktree file is skipped. A missing `--config` file, invalid TOML, an unknown
key, an unknown theme, a malformed colour, a role or status naming no palette colour, an unknown
`kind`, or a duplicate status name or alias exits 1 with one stderr line naming the file and the
key, or `line N` for a TOML syntax error.

### [CT-5] CLI

Kind: api · Anchor: `crates/spec-viewer/src/cli.rs#parse`

```text
spec-viewer [ROOT] [--theme light|dark] [--config PATH]
exit 1: unresolvable or non-directory ROOT, invalid --theme, config error per [CT-4],
        web server bind failure; one stderr line
also serves the web app for the session (spec/web-ui.spec.md [CT-5])
```

## Acceptance

- `[VC-1]` Given the shared fixture rendered by `TestBackend` at 160×48 under the default config,
  then snapshots show the sidebar tree with status chips after `e`, the header fields, the rail as a
  family list for a plan phase and as a section outline for a standalone spec, and every `[CT-1]`
  construct of `fixtures/dialect.spec.mdx`. — demonstrates `[SO-1]`
- `[VC-2]` Given key sequences on the fixture, then: a filter shows a flat list with paths and
  without attached examples; `d` restricts specs to the changed set; `w` switches worktree and
  clears the document; opening a phase keeps its plan row current; hint mode on an xref writes an
  OSC 52 sequence carrying the ID; hint mode on a doc link opens its target; opening a document
  replaces the sidebar with the rail and `e` swaps them back. — demonstrates `[SO-1]`
- `[VC-3]` Given an open document, when a new `.plan/*.md` file is written and the open file is
  edited, then a reload arrives, the tree lists the new plan, the body shows the edit, and the
  scroll position is kept. — demonstrates `[SO-2]`
- `[VC-4]` Given a worktree config whose `[[status]]` list holds only
  `{ name = "shipped", aliases = ["released"], color = "magenta" }`, then `status: released` shows
  `[shipped]` in magenta in the tree and the header, and `status: done` shows `[done]` in
  `status_unknown`. — demonstrates `[SO-3]`
- `[VC-5]` Given the single chip `{ label = "R", kind = "checklist", sections = ["risks"] }` and a
  `## Risks` checklist with one of two items checked, then the header shows `R 1/2` and no `AC`,
  `T`, or `phases`. — demonstrates `[SO-3]`
- `[VC-6]` Given a palette setting every ANSI slot and `surface` to hex colours, then the dialect
  document, code blocks included, uses only those colours. Given `[theme.palette] blue = "#123456"`
  on `terminal`, then the focused border, `kd` xrefs, and `[accepted]` render `#123456`. A
  `[theme.palette.light]` colour applies under light mode only, and `--theme light` beats
  `mode = "dark"`. A new palette name used by a role applies. A role naming no palette colour exits
  1. — demonstrates `[SO-3]`
- `[VC-7]` Given user, worktree, and `--config` layers each setting `accent`, then `--config` wins.
  A worktree `[[status]]` list fully replaces the user's. Invalid TOML, an unknown key, or a missing
  `--config` path exits 1 with one stderr line naming the file. — demonstrates `[SO-3]`

## Caveats

- `[C-1]` OSC 52 depends on the terminal; tmux needs `set-clipboard on`.
- `[C-2]` The OSC 11 background query runs on Unix only; elsewhere `auto` resolves to dark.
- `[C-3]` Hex palette colours, including the default `surface`, need a truecolor terminal;
  otherwise the terminal approximates them.

## Changelog

| Date       | Amendment                                                                                                                        | Sections affected                    | Reason                                                           |
| ---------- | -------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------ | ---------------------------------------------------------------- |
| 2026-10-18 | Moved out of spec-kit; dropped `parent-spec`; tree semantics stated in `[CT-2]`                                                  | Why, Design, CT-2                    | `spec-viewer` became its own repository and owns this behaviour  |
| 2026-10-18 | `to_terminal` takes the semantic `Document`; module paths follow the workspace crates                                            | CT-1, CT-3                           | The HTML-shaped tree existed only for parity with the Bun viewer |
| 2026-10-18 | Layered config for theme, statuses, and header chips (`[KD-5]`, `[KD-8]`, `[KD-9]`, `[CT-4]`)                                    | Design, CT-2, CT-4, CT-5, Acceptance | Projects use other statuses, header sections, and colours        |
| 2026-10-19 | `o` opens the web app; the session also serves it                                                                                | CT-2, CT-5                           | The browser view moved into `spec-viewer` (spec/web-ui.spec.md)  |
| 2026-10-19 | Code blocks boxed on `code_bg` and highlighted by ANSI slot; Mermaid drawn as text; Catppuccin built-ins and code themes removed | CT-1, CT-4, Design, VC-6, C-3        | Colours follow the configured palette                            |
| 2026-10-19 | The sidebar and the rail share one pane; opening a document shows the rail, `e` swaps them; `surface` defaults to hex            | Design, CT-2, VC-1, VC-2, C-3        | Wider document; code blocks stand out on any terminal background |
| 2026-10-19 | Inline expressions stay in the paragraph; comparison and node-detail cards side by side; step rail; `muted` table borders        | CT-1                                 | Closer to the web rendering                                      |
