# Helix

A native Agent Development Environment (ADE) built in Rust with [GPUI](https://www.gpui.rs).

Helix is a **worktree-first** cockpit for AI-assisted development: each project is bound to a Git worktree, and everything — terminals, Claude Code sessions, git state, file changes — revolves around it in a single window. No Electron, no webviews.

## Status

Milestone 1 (foundation) — functional:

- Native GPUI window with translucent blurred background (macOS)
- Three-region layout: collapsible left sidebar, tabbed workspace, collapsible right sidebar
- Full terminal emulation per tab (`alacritty_terminal` + native PTY): ANSI colors, scrollback, selection, copy/paste, resize
- Claude Code sessions: each `✦ Claude` tab spawns `claude` in its own PTY, tracked as an agent
- Live agent tree with status (Running / Idle / Error / Finished) derived from PTY activity
- Real-time git panel via libgit2: branch, staged/unstaged/untracked, conflicts, commits, stash, ahead/behind
- Filesystem watcher (notify) driving live refresh of git state and the file tree
- Sidebar tree: project → worktrees (primary + linked, with branch) → agents per worktree
- Uses the user's login shell (fish, zsh, bash, …) detected via `getpwuid`, with `$SHELL` fallback

Milestone 2 (editor, diff, source control) — functional:

- Tabs hold terminals, editors or diffs (`TabContent`), with VS Code preview semantics: single click on a file opens a replaceable preview tab, double click pins it
- Code editor per file: tree-sitter syntax highlighting, line numbers, auto-indent, indent guides, in-buffer search (via `gpui-component`'s code editor); ⌘S saves
- Read-side gating: files over 50 MB, binaries (NUL probe over the first 8 KB) and images get dedicated viewers instead of the editor
- Agent-safe buffers: a clean editor reloads when a file changes on disk; a dirty one never does — it shows a banner with Reload from disk / Keep my edits, resolved by comparing FNV-1a content signatures rather than a write-echo timer
- Diff view against four bases (working tree, index, HEAD, merge-base) computed from git blob pairs with `similar`, rendered per line with real syntax highlighting on both sides, old/new gutters and a 120k-line / 6M-char render cap
- File tree with per-file git status (colored name + status letter), status propagated to ancestor directories by dominance, folder/chevron split, dotfile toggle, collapse-all and refresh
- Source control panel: per-file stage/unstage, stage all, commit with libgit2
- Pull requests through the `gh` CLI: branch lookup, check rollup, review decision, conflict state, and a blocked-reason/next-action state machine that turns one button into install gh → authenticate → commit → publish → push → sync → create PR. A lookup that fails is `Unavailable`, never "no PR", so it can never create a duplicate

Planned next:

- M3: AI-generated commit messages and PR bodies (water-fill diff truncation is already in `helix-git`), attention-tiered PR polling, checks detail panel
- M4: agent telemetry (tokens, tool calls) via Claude Code hooks, tab drag & drop
- M5: Linux and Windows support

## Running

```sh
cargo run -- /path/to/project    # defaults to the current directory
```

### Development loop

`scripts/dev.sh` uses [cargo-watch](https://github.com/watchexec/cargo-watch) to rebuild and relaunch the app whenever any crate changes:

```sh
./scripts/dev.sh /path/to/project    # defaults to the repo itself
```

### Keybindings

| Key | Action |
| --- | --- |
| ⌘T | New terminal tab |
| ⌘⇧T | New Claude Code session |
| ⌘W | Close active tab |
| ⌘S | Save file (editor tabs) |
| ⌃Tab / ⌘⇧] | Next tab |
| ⌃⇧Tab / ⌘⇧[ | Previous tab |
| ⌘B | Toggle left sidebar |
| ⌘R | Toggle right sidebar |
| ⌘C / ⌘V | Copy / paste in terminal |

## Architecture

Cargo workspace, one crate per domain; UI crates depend on domain crates, never the reverse. Modules communicate through events (tokio channels) and GPUI entities.

```
helix/
  app/         binary: window bootstrap, macOS blur integration
  ui/          GPUI views: root layout, sidebars, workspace, terminal/editor/diff views, theme
  terminal/    PTY + alacritty_terminal backend (no UI dependency)
  agents/      session launch specs (shell, claude) and agent metadata
  buffer/      file reads with size/binary gating, language detection, content signatures
  git/         libgit2 snapshots, index ops (stage/commit), blob-pair diffs, remote ops
  github/      gh CLI transport, hosted-review model, PR eligibility state machine
  worktree/    project/worktree discovery and metadata
  filesystem/  debounced recursive fs watcher
  events/      shared event types and channels
  models/      pure domain types shared by all crates
  state/       session activity status, history log
  commands/    gpui actions and default keybindings
```

Requires Rust 1.85+ and macOS (for now). Uses gpui's `runtime_shaders` feature, so full Xcode is not required — Command Line Tools are enough.
