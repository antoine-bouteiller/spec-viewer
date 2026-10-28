// Web client for spec-viewer: sidebar tree, structure rail, and the server-rendered document.
const MERMAID = 'https://cdn.jsdelivr.net/npm/mermaid@12/dist/mermaid.esm.min.mjs'
const THEME_KEY = 'spec-viewer.theme'
const UNKNOWN_TONE = 'var(--status-unknown)'
const NO_DOCS = { changed: null, project: '', tabs: [] }

const $ = (id) => document.getElementById(id)
const page = $('page')
const nav = $('nav')
const rail = $('rail')
const content = $('content')
const diffToggle = $('diff')
const searchTrigger = $('search-open')
const searchDialog = $('search')
const searchInput = $('search-input')
const searchResults = $('search-results')
const searchCount = $('search-count')
const searchFooter = searchDialog.querySelector('.search-footer')

// Keybindings: the `[keys]` config's `main` and `search_dialog` tables, from `/api/keys`.
let keys = { main: {}, search_dialog: {} }
// A single character already carries Shift, as in the terminal.
const pressed = (event, binding = []) =>
  binding.some(
    (chord) =>
      event.key === chord.key &&
      event.ctrlKey === chord.ctrl &&
      event.altKey === chord.alt &&
      event.metaKey === chord.meta &&
      ([...chord.key].length === 1 || event.shiftKey === chord.shift)
  )
const keyLabel = (binding) => binding?.[0]?.label

const ENTITIES = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }
const esc = (value) => String(value).replace(/[&<>"']/g, (character) => ENTITIES[character])

const icon = (className, paths) =>
  `<svg class="${className}" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" viewBox="0 0 24 24">${paths
    .map((d) => `<path d="${d}"/>`)
    .join('')}</svg>`
const CHEVRON = icon('chevron', ['M9.00005 6C9.00005 6 15 10.4189 15 12C15 13.5812 9 18 9 18'])
const SPINNER = icon('spinner', [
  'M12 3V6',
  'M12 18V21',
  'M21 12L18 12',
  'M6 12L3 12',
  'M18.3635 5.63672L16.2422 7.75804',
  'M7.75804 16.2422L5.63672 18.3635',
  'M18.3635 18.3635L16.2422 16.2422',
  'M7.75804 7.75804L5.63672 5.63672',
])

// URL state: wt, tab, doc, diff.

const URL_KEYS = ['wt', 'tab', 'doc', 'diff']

const readUrl = () => {
  const params = new URLSearchParams(location.search)
  const tab = params.get('tab')
  return {
    diff: params.get('diff') === 'true' ? 'true' : undefined,
    doc: params.get('doc') || undefined,
    tab: tab || undefined,
    wt: params.get('wt') || undefined,
  }
}

let url = readUrl()

const setUrl = (patch) => {
  url = { ...url, ...patch }
  const params = new URLSearchParams()
  for (const key of URL_KEYS) {
    if (url[key]) {
      params.set(key, url[key])
    }
  }
  const query = params.toString()
  history.replaceState(null, '', query ? `?${query}` : location.pathname)
}

const state = {
  doc: null,
  docError: null,
  docKey: '',
  docs: NO_DOCS,
  docsError: null,
  expanded: new Set(),
  focusedKey: null,
  pending: false,
  project: null,
  treeId: '',
}

// Tabs in config order; an unknown `tab` falls back to the first.
const tabs = () => state.docs.tabs
const tab = () => (tabs().find((item) => item.name === url.tab) ?? tabs()[0])?.name
const worktree = () => state.project?.worktrees.find((item) => item.path === url.wt)
const listed = () => tabs().find((item) => item.name === tab())?.nodes ?? []

// Tree model.

const nodeKey = (node, parentKey) => node.path ?? `${parentKey}/${node.name}`
const isFamilyRoot = (node) => Boolean(node.path) && Boolean(node.children?.length)
const childrenOf = (node) => (isFamilyRoot(node) ? [] : (node.children ?? []))
const containsPath = (node, path) => node.path === path || (node.children ?? []).some((child) => containsPath(child, path))

const findNode = (node, path) => {
  if (node.path === path) {
    return node
  }
  for (const child of node.children ?? []) {
    const found = findNode(child, path)
    if (found) {
      return found
    }
  }
  return null
}

const ancestorKeys = (nodes, active, parentKey = '') => {
  for (const node of nodes) {
    const key = nodeKey(node, parentKey)
    if (node.path === active) {
      return []
    }
    const ancestors = ancestorKeys(node.children ?? [], active, key)
    if (ancestors.length || node.children?.some((child) => child.path === active)) {
      return [key, ...ancestors]
    }
  }
  return []
}

// An umbrella's leaves, with their companions, are reached through the umbrella.
const isUmbrellaLeaf = (parent, child) => parent.kind === 'umbrella' && child.kind !== 'umbrella' && !child.companion
const listedOf = (node) => (node.children ?? []).filter((child) => !isUmbrellaLeaf(node, child))
const touches = (node, paths) => paths.has(node.path) || (node.children ?? []).some((child) => touches(child, paths))

// A companion attached to its owner is reached through the owner's row, not listed on its own.
const matchingDocuments = (nodes, paths) =>
  nodes.flatMap((node) => [
    ...(node.path &&
    (paths.has(node.path) || (node.children ?? []).some((child) => isUmbrellaLeaf(node, child) && touches(child, paths)))
      ? [node]
      : []),
    ...matchingDocuments(
      listedOf(node).filter((child) => !(node.path && child.companion)),
      paths
    ),
  ])

const documentsOf = (nodes) =>
  nodes.flatMap((node) => [...(node.path ? [node] : []), ...(isFamilyRoot(node) ? [] : documentsOf(node.children ?? []))])

const familyOf = (node, path) => {
  for (const child of node.children ?? []) {
    const nested = familyOf(child, path)
    if (nested) {
      return nested
    }
  }
  return isFamilyRoot(node) && containsPath(node, path) ? [node, ...documentsOf(node.children ?? [])] : null
}

const activeOwner = () => tabs().flatMap((item) => item.nodes).find((node) => findNode(node, url.doc ?? null)) ?? null

const changedFilter = () => (url.diff === 'true' && state.docs.changed ? new Set(state.docs.changed) : undefined)

// Shared fragments.

const statusBadge = (status, archived, chip) => {
  const shown = archived ? { name: 'archived', tone: UNKNOWN_TONE, known: false } : status
  if (!shown) {
    return ''
  }
  const classes = ['status-badge', chip && 'is-chip', !shown.known && 'is-unknown'].filter(Boolean).join(' ')
  return `<span class="${classes}" style="--tone: ${esc(shown.tone)}" aria-label="status: ${esc(shown.name)}">${esc(shown.name)}</span>`
}

const rowName = (name, current, title) =>
  `<span class="row-name${current ? ' is-current' : ''}"${title ? ` title="${esc(title)}"` : ''}>${esc(name)}</span>`

const rowButton = (current, attributes, inner, extraClass = '') =>
  `<button type="button" class="tree-row${current ? ' is-current' : ''}${extraClass}" ${attributes}${current ? ' aria-current="page"' : ''}>${inner}</button>`

// Sidebar.

const treeMarkup = (nodes, depth = 0, parentKey = '') => {
  const active = url.doc ?? null
  const items = nodes.map((node) => {
    const key = nodeKey(node, parentKey)
    const children = childrenOf(node)
    const hasChildren = children.length > 0
    const current = node.path === active || (isFamilyRoot(node) && containsPath(node, active))
    const expanded = state.expanded.has(key)
    const toggle = hasChildren
      ? `<button type="button" class="tree-toggle" tabindex="-1" aria-label="${expanded ? 'Collapse' : 'Expand'} ${esc(node.name)}" aria-expanded="${expanded}">${CHEVRON}</button>`
      : ''
    const row = `<div class="tree-row${current ? ' is-current' : ''} tree-node" role="treeitem" tabindex="-1" data-key="${esc(key)}"${
      node.path ? ` data-path="${esc(node.path)}"` : ''
    }${hasChildren ? ` aria-expanded="${expanded}"` : ''}${current ? ' aria-current="page"' : ''}>${toggle}${rowName(node.name, current, node.path)}${statusBadge(
      node.status,
      node.archived,
      true
    )}</div>`
    return `<li>${row}${hasChildren && expanded ? treeMarkup(children, depth + 1, key) : ''}</li>`
  })
  return depth ? `<ul class="tree-group" role="group">${items.join('')}</ul>` : `<ul role="tree">${items.join('')}</ul>`
}

const renderSidebar = () => {
  const project = state.project
  const selected = worktree()
  const parts = []
  if (project) {
    const options = project.worktrees
      .map(
        (item) =>
          `<option value="${esc(item.path)}"${item.path === selected?.path ? ' selected' : ''}>${esc(item.name)}${item.main ? ' (main)' : ''}</option>`
      )
      .join('')
    parts.push(
      `<div class="worktree"><select class="select" aria-label="Worktree"><option value="" disabled${selected ? '' : ' selected'}>Select a worktree…</option>${options}</select></div>`
    )
  }
  if (selected) {
    parts.push(
      `<div class="doc-tabs">${tabs()
        .map(({ name }) => rowButton(tab() === name, `data-tab="${esc(name)}"`, esc(name), ' doc-tab'))
        .join('')}</div>`
    )
    const paths = changedFilter()
    const nodes = listed()
    if (paths) {
      const matches = matchingDocuments(nodes, paths)
      parts.push(
        matches.length
          ? `<ul>${matches
              .map(
                (node) =>
                  `<li>${rowButton(
                    url.doc === node.path,
                    `data-path="${esc(node.path)}"`,
                    `<span class="match"><span class="match-name">${esc(node.name)}</span><span class="match-path">${esc(node.path)}</span></span>${statusBadge(
                      node.status,
                      node.archived,
                      true
                    )}`
                  )}</li>`
              )
              .join('')}</ul>`
          : `<p class="list-empty">no matching ${tab()}</p>`
      )
    } else {
      parts.push(nodes.length ? treeMarkup(nodes) : `<p class="list-empty">no ${tab()}</p>`)
    }
  }
  const hadFocus = nav.contains(document.activeElement)
  nav.innerHTML = parts.join('')
  syncTreeFocus(hadFocus)

  diffToggle.disabled = state.docs.changed === null
  diffToggle.setAttribute('aria-pressed', String(url.diff === 'true'))
}

const treeRows = () => [...nav.querySelectorAll('.tree-node')]

// One row is tabbable: the focused row, else the current document's, else the first.
const syncTreeFocus = (refocus) => {
  const rows = treeRows()
  const target =
    rows.find((row) => row.dataset.key === state.focusedKey) ?? rows.find((row) => row.dataset.path === url.doc) ?? rows[0]
  if (!target) {
    return
  }
  target.tabIndex = 0
  if (refocus) {
    target.focus()
  }
}

const focusRow = (row) => {
  if (row) {
    state.focusedKey = row.dataset.key
    for (const other of treeRows()) {
      other.tabIndex = other === row ? 0 : -1
    }
    row.focus()
  }
}

const toggle = (key) => {
  if (!state.expanded.delete(key)) {
    state.expanded.add(key)
  }
  state.focusedKey = key
  renderSidebar()
}

const resetTree = () => {
  const id = `${url.wt}:${tab()}`
  if (state.treeId !== id) {
    state.treeId = id
    state.expanded = new Set()
    state.focusedKey = null
  }
  for (const key of ancestorKeys(listed(), url.doc ?? null)) {
    state.expanded.add(key)
  }
}

nav.addEventListener('click', (event) => {
  const target = event.target
  const tabButton = target.closest('[data-tab]')
  if (tabButton) {
    selectTab(tabButton.dataset.tab)
    return
  }
  const toggleButton = target.closest('.tree-toggle')
  const row = target.closest('.tree-node')
  if (toggleButton && row) {
    toggle(row.dataset.key)
    return
  }
  if (row) {
    state.focusedKey = row.dataset.key
    if (row.dataset.path) {
      open(row.dataset.path)
    } else if (row.hasAttribute('aria-expanded')) {
      toggle(row.dataset.key)
    }
    return
  }
  const match = target.closest('button[data-path]')
  if (match) {
    open(match.dataset.path)
  }
})

nav.addEventListener('change', (event) => {
  if (event.target.matches('select')) {
    selectWorktree(event.target.value)
  }
})

nav.addEventListener('keydown', (event) => {
  const row = event.target.closest('.tree-node')
  if (!row) {
    return
  }
  const rows = treeRows()
  const index = rows.indexOf(row)
  const expandable = row.hasAttribute('aria-expanded')
  const expanded = row.getAttribute('aria-expanded') === 'true'
  const group = row.parentElement.querySelector(':scope > .tree-group')
  const parentRow = row.parentElement.parentElement.closest('li')?.querySelector(':scope > .tree-node')
  const main = keys.main
  if (pressed(event, main.down)) {
    event.preventDefault()
    focusRow(rows[index + 1])
  } else if (pressed(event, main.up)) {
    event.preventDefault()
    focusRow(rows[index - 1])
  } else if (pressed(event, main.right)) {
    event.preventDefault()
    if (expandable && !expanded) {
      toggle(row.dataset.key)
    } else if (group) {
      focusRow(group.querySelector('.tree-node'))
    }
  } else if (pressed(event, main.left)) {
    event.preventDefault()
    if (expandable && expanded) {
      toggle(row.dataset.key)
    } else {
      focusRow(parentRow)
    }
  } else if (pressed(event, main.open) && row.dataset.path) {
    event.preventDefault()
    open(row.dataset.path)
  }
})

// Search dialog: the server ranks matches with the terminal's search (`spec_viewer_core::search`).

const search = { hits: [], request: 0, selection: 0, total: 0 }

const highlight = (text, matches) => {
  const matched = new Set(matches)
  return Array.from(text)
    .map((character, index) => (matched.has(index) ? `<mark>${esc(character)}</mark>` : esc(character)))
    .join('')
}

const renderSearch = () => {
  searchCount.textContent = `${search.hits.length}/${search.total}`
  searchResults.innerHTML = search.hits.length
    ? search.hits
        .map((hit, index) => {
          const selected = index === search.selection
          return `<li id="search-hit-${index}" role="option" class="search-hit${selected ? ' is-selected' : ''}" aria-selected="${selected}" data-index="${index}"><span class="search-hit-name">${highlight(
            hit.name,
            hit.titleMatches
          )}</span>${statusBadge(hit.status, hit.archived, true)}<span class="search-hit-path">${highlight(hit.path, hit.pathMatches)}</span><span class="search-hit-tab">${esc(
            hit.tab
          )}</span></li>`
        })
        .join('')
    : '<li class="list-empty">no matching documents</li>'
  searchInput.setAttribute('aria-activedescendant', search.hits.length ? `search-hit-${search.selection}` : '')
  searchResults.querySelector('.is-selected')?.scrollIntoView({ block: 'nearest' })
}

const runSearch = async () => {
  const request = ++search.request
  let result = { hits: [], total: search.total }
  try {
    result = await getJson('/api/search', { q: searchInput.value, wt: url.wt })
  } catch {
    // An unreachable server shows no matches.
  }
  if (request === search.request) {
    search.hits = result.hits
    search.total = result.total
    search.selection = Math.min(search.selection, Math.max(search.hits.length - 1, 0))
    renderSearch()
  }
}

const openSearch = () => {
  if (!url.wt || searchDialog.open) {
    return
  }
  searchInput.value = ''
  search.selection = 0
  searchDialog.showModal()
  searchInput.focus()
  void runSearch()
}

const moveSearch = (delta) => {
  search.selection = Math.min(Math.max(search.selection + delta, 0), Math.max(search.hits.length - 1, 0))
  renderSearch()
}

const chooseHit = (index) => {
  const hit = search.hits[index]
  if (!hit) {
    return
  }
  searchDialog.close()
  setUrl({ doc: hit.path, tab: hit.tab })
  resetTree()
  void loadDoc()
}

searchTrigger.addEventListener('click', openSearch)

searchInput.addEventListener('input', () => {
  search.selection = 0
  void runSearch()
})

searchInput.addEventListener('keydown', (event) => {
  const bindings = keys.search_dialog
  if (pressed(event, bindings.down)) {
    moveSearch(1)
  } else if (pressed(event, bindings.up)) {
    moveSearch(-1)
  } else if (pressed(event, bindings.open)) {
    chooseHit(search.selection)
  } else if (pressed(event, bindings.clear)) {
    searchInput.value = ''
    search.selection = 0
    void runSearch()
  } else if (pressed(event, bindings.close)) {
    searchDialog.close()
  } else {
    return
  }
  event.preventDefault()
})

// Escape closes a modal dialog natively; keep it open when `close` does not bind it.
searchDialog.addEventListener('cancel', (event) => {
  if (!pressed({ key: 'Escape', ctrlKey: false, altKey: false, metaKey: false, shiftKey: false }, keys.search_dialog.close)) {
    event.preventDefault()
  }
})

searchResults.addEventListener('click', (event) => {
  const hit = event.target.closest('[data-index]')
  if (hit) {
    chooseHit(Number(hit.dataset.index))
  }
})

searchDialog.addEventListener('click', (event) => {
  if (event.target === searchDialog) {
    searchDialog.close()
  }
})

addEventListener('keydown', (event) => {
  const typing = event.target.closest?.('input, select, textarea, [contenteditable]')
  const shortcut = pressed(event, keys.main.search)
  if (shortcut && !typing && !searchDialog.open) {
    event.preventDefault()
    openSearch()
  }
})

diffToggle.addEventListener('click', () => {
  setUrl({ diff: url.diff === 'true' ? undefined : 'true' })
  renderSidebar()
})

// Rail.

let sections = []

const sectionsMarkup = (toc, nested) =>
  toc.length
    ? `<ol class="toc${nested ? ' toc-nested' : ''}">${toc
        .map(
          (entry) =>
            `<li><a href="#${esc(entry.id)}" data-section="${esc(entry.id)}" class="toc-link${entry.depth === 3 ? ' toc-sub' : ''}">${esc(entry.text)}</a></li>`
        )
        .join('')}</ol>`
    : ''

const renderRail = () => {
  const active = url.doc ?? null
  const owner = activeOwner()
  const toc = state.pending ? [] : (state.doc?.toc ?? [])
  sections = toc
  if (!owner) {
    rail.innerHTML = '<p class="rail-empty">No document selected.</p>'
    return
  }
  const files = familyOf(owner, active)
  if (!files) {
    rail.innerHTML = `<nav aria-label="Document structure"><h2 class="rail-heading">Sections</h2>${sectionsMarkup(toc)}</nav>`
  } else {
    const items = files
      .map((file, index) => {
        const current = active === file.path
        const badges = `${index === 0 ? '<span class="index-badge">index</span>' : ''}${
          file.kind === 'examples' ? '<span class="index-badge">examples</span>' : ''
        }`
        return `<li>${rowButton(current, `data-path="${esc(file.path)}" title="${esc(file.name)}"`, `${rowName(file.name, current)}${badges}`)}${
          current ? sectionsMarkup(toc, true) : ''
        }</li>`
      })
      .join('')
    const outside = findNode(owner, active) && !files.some((file) => file.path === active)
    rail.innerHTML = `<nav aria-label="Document structure"><h2 class="rail-heading">Files</h2><ul>${items}</ul>${
      outside ? `<h2 class="rail-heading rail-heading-spaced">Sections</h2>${sectionsMarkup(toc)}` : ''
    }</nav>`
  }
  syncSection()
}

const syncSection = () => {
  const headings = sections.map((entry) => document.getElementById(entry.id)).filter(Boolean)
  if (!headings.length) {
    return
  }
  const top = page.getBoundingClientRect().top + 24
  let current = headings[0].id
  for (const heading of headings) {
    if (heading.getBoundingClientRect().top <= top) {
      current = heading.id
    }
  }
  for (const link of rail.querySelectorAll('.toc-link')) {
    link.classList.toggle('is-current', link.dataset.section === current)
  }
}

page.addEventListener('scroll', syncSection, { passive: true })

rail.addEventListener('click', (event) => {
  const file = event.target.closest('button[data-path]')
  if (file) {
    open(file.dataset.path)
  }
})

// Document.

const metric = (chip) => {
  if (chip.kind !== 'checklist' || !chip.total) {
    return ''
  }
  const tone = chip.done === chip.total ? 'var(--status-done)' : 'var(--status-in-progress)'
  return `<span class="metric"><span class="metric-count">${esc(chip.label)} ${chip.done}/${chip.total}</span><span class="meter"><span class="meter-fill" style="background: ${tone}; width: ${
    (chip.done / chip.total) * 100
  }%"></span></span></span>`
}

const docMarkup = (doc) => {
  const { meta } = doc
  const owner = activeOwner()
  const kind = (owner && findNode(owner, url.doc ?? null)?.kind) ?? meta.kind ?? 'document'
  const family = owner && familyOf(owner, url.doc ?? null)
  const details = [
    kind,
    family && family[0].path !== doc.rootPath ? family[0].name : '',
    ...meta.chips.filter((chip) => chip.kind === 'count' && chip.count).map((chip) => `${chip.count} ${chip.label}`),
    meta.legacy ? 'legacy format' : '',
  ].filter(Boolean)
  const name = doc.path.split('/').pop() ?? ''
  return `<div class="doc"><header class="doc-header">
    <div class="doc-title-row"><h1 class="doc-title">${esc(meta.title)}</h1>${statusBadge(meta.status, false, false)}<span class="doc-stats">${meta.chips
      .map(metric)
      .join('')}<span class="doc-lines">${meta.lines} lines</span></span></div>
    <div class="doc-meta"><div class="doc-kind">${esc(details.join(' · '))}</div>${meta.author ? `<span>${esc(meta.author)}</span>` : ''}${
      meta.date ? `<span class="doc-date">${esc(meta.date)}</span>` : ''
    }${meta.related.length ? `<span class="doc-related">related: ${esc(meta.related.join(', '))}</span>` : ''}<span class="doc-path"><span class="doc-path-text">${esc(
      doc.rootPath
    )}</span><button type="button" class="copy-chip" data-copy="${esc(doc.rootPath)}" title="${esc(doc.rootPath)}">path</button><button type="button" class="copy-chip" data-copy="${esc(
      name
    )}">name</button></span></div>
  </header><article class="prose">${doc.html}</article></div>`
}

const renderPage = () => {
  const tone = state.pending || !state.doc?.meta.status?.known ? 'var(--border)' : state.doc.meta.status.tone
  $('status-bar').style.background = tone
  const error = state.docError ?? state.docsError
  if (state.pending) {
    content.innerHTML = `<div class="loading">${SPINNER}<span class="loading-text">Loading…</span></div>`
  } else if (error) {
    content.innerHTML = `<p class="pane-message pane-error">Could not load this document: ${esc(error)}</p>`
  } else if (!state.doc) {
    content.innerHTML = '<p class="pane-message">Select a document.</p>'
  } else {
    content.innerHTML = docMarkup(state.doc)
    if (state.doc.hasMermaid) {
      void renderDiagrams()
    }
  }
}

const copy = (value, done) =>
  navigator.clipboard.writeText(value).then(() => {
    done(true)
    setTimeout(() => done(false), 1200)
  })

content.addEventListener('click', (event) => {
  const target = event.target
  const link = target.closest('a[data-doc]')
  if (link) {
    event.preventDefault()
    open(link.dataset.doc)
    return
  }
  const chip = target.closest('.xref-chip')
  if (chip?.dataset.ref) {
    void copy(chip.dataset.ref, (copied) => (copied ? (chip.dataset.copied = 'true') : delete chip.dataset.copied))
    return
  }
  const copyChip = target.closest('.copy-chip')
  if (copyChip) {
    const label = copyChip.textContent
    void copy(copyChip.dataset.copy, (copied) => (copyChip.textContent = copied ? 'copied' : label))
  }
})

const isDark = () => {
  const mode = document.documentElement.dataset.theme
  return mode ? mode === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches
}

let mermaidModule = null

// Diagrams render one at a time: a concurrent run lets a failing diagram's error output land in another's container.
const renderDiagrams = async () => {
  const diagrams = [...content.querySelectorAll('pre.mermaid')]
  if (!diagrams.length) {
    return
  }
  for (const diagram of diagrams) {
    diagram.dataset.source ??= diagram.textContent ?? ''
    diagram.textContent = diagram.dataset.source
    diagram.removeAttribute('data-processed')
  }
  try {
    mermaidModule ??= import(MERMAID)
    const { default: mermaid } = await mermaidModule
    const styles = getComputedStyle(document.documentElement)
    const color = (name) => styles.getPropertyValue(name).trim()
    mermaid.initialize({
      startOnLoad: false,
      theme: 'base',
      themeVariables: {
        background: color('--bg'),
        darkMode: isDark(),
        lineColor: color('--muted'),
        primaryBorderColor: color('--accent'),
        primaryColor: color('--bg-elevated'),
        primaryTextColor: color('--fg'),
        secondaryColor: color('--bg-hover'),
        secondaryTextColor: color('--fg'),
        tertiaryColor: color('--code-bg'),
        tertiaryTextColor: color('--fg'),
        textColor: color('--fg'),
      },
    })
    for (const diagram of diagrams) {
      if (!diagram.isConnected) {
        return
      }
      await mermaid.run({ nodes: [diagram] }).catch(() => {
        diagram.textContent = diagram.dataset.source ?? ''
        diagram.removeAttribute('data-processed')
      })
    }
  } catch {
    // Offline or blocked CDN: the Mermaid source stays visible.
    mermaidModule = null
  }
}

matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => void renderDiagrams())
new MutationObserver(() => void renderDiagrams()).observe(document.documentElement, { attributeFilter: ['data-theme'] })

// Theme selector.

const applyTheme = (mode) => {
  const root = document.documentElement
  if (mode === 'system') {
    root.removeAttribute('data-theme')
    localStorage.removeItem(THEME_KEY)
  } else {
    root.setAttribute('data-theme', mode)
    localStorage.setItem(THEME_KEY, mode)
  }
  for (const option of document.querySelectorAll('.theme-option')) {
    option.setAttribute('aria-checked', String(option.dataset.mode === mode))
  }
}

for (const option of document.querySelectorAll('.theme-option')) {
  option.addEventListener('click', () => applyTheme(option.dataset.mode))
}
applyTheme(localStorage.getItem(THEME_KEY) ?? 'system')

// Data.

const getJson = async (path, params) => {
  const response = await fetch(`${path}?${new URLSearchParams(params)}`)
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText}`)
  }
  return response.json()
}

const loadDocs = async () => {
  const wt = url.wt
  try {
    const docs = await getJson('/api/docs', { wt })
    if (url.wt === wt) {
      state.docs = docs
      state.docsError = null
    }
  } catch (error) {
    if (url.wt === wt) {
      state.docs = NO_DOCS
      state.docsError = error.message
    }
  }
  if (url.wt === wt) {
    document.title = state.docs.project ? `${state.docs.project} · Specs` : 'Specs'
    resetTree()
    renderAll()
  }
}

// A reload of the open document keeps its scroll position; opening another scrolls to the top.
const loadDoc = async () => {
  const { wt, doc } = url
  const key = wt && doc ? `${wt}|${doc}` : ''
  const silent = key === state.docKey && !state.pending
  state.docKey = key
  if (!key) {
    state.doc = null
    state.docError = null
    state.pending = false
    renderAll()
    return
  }
  if (!silent) {
    state.pending = true
    renderAll()
  }
  let loaded = null
  let error = null
  try {
    loaded = await getJson('/api/doc', { path: doc, wt })
  } catch (failure) {
    error = failure.message
  }
  if (state.docKey !== key) {
    return
  }
  state.doc = loaded
  state.docError = error
  state.pending = false
  renderAll()
  if (!silent) {
    page.scrollTo({ top: 0 })
  }
}

let events = null

const connectEvents = () => {
  events?.close()
  events = url.wt ? new EventSource(`/api/events?${new URLSearchParams({ wt: url.wt })}`) : null
  events?.addEventListener('message', () => {
    void loadDocs()
    void loadDoc()
    if (searchDialog.open) {
      void runSearch()
    }
  })
}

const loadProjects = async () => {
  try {
    const { projects } = await getJson('/api/projects', {})
    state.project = projects[0] ?? null
  } catch {
    state.project = null
  }
  if (state.project && !worktree()) {
    const fallback = state.project.worktrees.find((item) => item.main) ?? state.project.worktrees[0]
    if (fallback) {
      selectWorktree(fallback.path, !url.wt)
      return
    }
  }
  renderSidebar()
}

// Actions.

const renderAll = () => {
  renderSidebar()
  renderRail()
  renderPage()
}

const open = (path) => {
  setUrl({ doc: path })
  resetTree()
  void loadDoc()
}

const selectTab = (next) => {
  setUrl({ doc: undefined, tab: next })
  resetTree()
  void loadDoc()
}

// A link without `wt` keeps its `doc`, opened in the default worktree.
const selectWorktree = (path, keepDoc = false) => {
  setUrl({ doc: keepDoc ? url.doc : undefined, wt: path })
  state.docs = NO_DOCS
  resetTree()
  connectEvents()
  void loadDocs()
  void loadDoc()
}

addEventListener('focus', () => void loadProjects())

const loadKeys = async () => {
  try {
    keys = await getJson('/api/keys', {})
  } catch {
    return
  }
  const searchKey = searchTrigger.querySelector('.search-key')
  searchKey.textContent = keyLabel(keys.main.search) ?? ''
  searchKey.hidden = !searchKey.textContent
  const dialog = keys.search_dialog
  const [up, down] = [keyLabel(dialog.up), keyLabel(dialog.down)]
  const move = up && down ? ([...up].length === 1 && [...down].length === 1 ? up + down : `${up}/${down}`) : undefined
  searchFooter.textContent = [
    move && `${move} move`,
    ...[
      [dialog.open, 'open'],
      [dialog.clear, 'clear'],
      [dialog.close, 'close'],
    ].map(([binding, action]) => keyLabel(binding) && `${keyLabel(binding)} ${action}`),
  ]
    .filter(Boolean)
    .join(' · ')
}

const start = async () => {
  void loadKeys()
  await loadProjects()
  if (worktree() && !events) {
    connectEvents()
    void loadDocs()
    void loadDoc()
  }
}

void start()
