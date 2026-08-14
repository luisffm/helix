# Backlog

Notes parked for later, newest first.

## Terminals into the worktree tab strip

The strip (`crates/ui/src/shell/tabs.rs`) already models a `WorktreeTab::Terminal`
tab and closes it; nothing creates one yet, because the terminal still owns its
own bar in a panel below the transcript. What is left:

- `terminal/panel.rs`: key its tab map by the strip's scope (the worktree path)
  instead of the chat id, take the scope from the shell rather than reading
  `selected_chat`, and render the body only — the strip owns the tabs.
- `OpenTerminal` RPC: take a `cwd` (the worktree) beside `chatId`, which today
  is the only way the engine resolves the folder (`engine/src/rpc.rs:1020`).
- Shell: `+ → New terminal` in the strip's plus menu, a terminal tab renders
  full-pane (no composer, no changes pane), ⌘J creates/focuses one, and the
  bottom-panel plumbing goes (height tween, drag anchor, `SessionPanels::
  terminal_open`).
- Tab drag-reorder: `panel.rs` still carries the tested pure helpers
  (`reorder_tabs`, `drop_index`, `slide_offset`, `active_after_reorder`) the
  strip can reuse.

## Worktree card title: branch or commit subject?

The cards landed titled by their branch. ORCA titles a non-primary card
`refactor(mcp): tool fact…`, which cannot be a ref (spaces, a colon) — that
reads as the worktree HEAD's commit subject, shown because a minted
`helix/<name>` branch says nothing. Needs a new RPC (or a field on `RepoRef`)
carrying the subject per worktree. OPEN until asked.

## DONE (kept for the shape it describes) — sidebar worktree cards

Every session running in a worktree should be listed under that worktree, so
one glance says what is running where. ORCA's shape: the project is a plain
header, and each worktree is a rounded CARD holding its own agents.

```
 ▣ helix                                       ← project: glyph tile + name
╭───────────────────────────────────────────╮  ← one card per worktree
│  ☾ main  (primary)                        │  ← status glyph + branch + pill
│     2 agents                          ⌄   │  ← count + its own collapse
│   ☾ ✻ ◐ Adicionar context men…      now   │  ← running: wash, bright title
│   ⊙ ✻   Mostrar diff acima do n…    11m   │  ← done: green check, dimmed
╰───────────────────────────────────────────╯
```

- Project header: the glyph in a bordered tile, name at full strength. No
  session count on it — the cards carry that.
- Worktree row: a status glyph (spinning while any agent in it runs), the
  branch, and a `primary` pill on the project root. A second ORCA card titles
  a non-primary worktree `refactor(mcp): tool fact…`, which cannot be a ref
  (spaces, a colon) — that reads as the worktree HEAD's commit subject, shown
  because a minted `helix/<name>` branch says nothing. Needs a new RPC (or a
  field on `RepoRef`) to carry the subject per worktree — OPEN: branch or
  subject?
- `N agents` line with its own collapse chevron, separate from the worktree's.
- Session rows: status glyph (spinner running / green check done), harness
  mark, a progress glyph while working, title, and elapsed time right-aligned
  (`now`, `11m`). Running rows keep a selected-style wash and a bright title;
  finished ones dim to muted.

Today `sidebar_rows` (`crates/proto/src/view.rs:630`) already nests sessions
under worktree rows; what is missing is the card frame, the count line with its
collapse, the `primary` pill, and the per-row status glyph column.

Today `sidebar_rows` (`crates/proto/src/view.rs:630`) already nests sessions
under worktree rows; what is missing is the count header, the per-group
collapse, and the elapsed-time/status column.
