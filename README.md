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

`scripts/dev.sh` uses [cargo-watch](https://github.com/watchexec/cargo-watch) to rebuild, rebundle and relaunch on every change under `src/` or `assets/`:

```sh
./scripts/dev.sh /path/to/project    # defaults to the repo itself
```

It runs the `.app`, not the bare binary, so the icon and the name are the same in development as in a release. Bundling a debug build costs about half a second on top of the Rust rebuild — the `.icns` is only regenerated when the artwork changes, and signing is skipped.

### macOS app bundle

A Dock and Finder icon needs a real `.app`. Put a square 1024×1024 PNG at `assets/icon.png`, then:

```sh
./scripts/bundle-mac.sh                  # writes target/Helix.app
open target/Helix.app --args /path/to/project
```

Pre-rendered sizes in `assets/icon.iconset/` are used as-is; without that directory the script downscales `icon.png` with `sips`, which is visibly softer at 16px. `ICON`, `ICONSET`, `BUNDLE_ID`, `PROFILE` and `SIGN` override the defaults.

The icon and the name both come from `Info.plist` (`CFBundleIconFile`, `CFBundleName`) — there is no runtime code for either, which is why the dev loop runs the bundle too.

The bundle is ad-hoc signed, not notarized: fine on the machine that built it, blocked by Gatekeeper anywhere else.

`cargo run` still works but produces a bare binary, so macOS falls back to the executable name and a generic icon.

### Keybindings

| Key | Action |
| --- | --- |
| ⌘T | New terminal tab |
| ⌘⇧T | New Claude Code session |
| ⌘W | Close active tab |
| ⌘S | Save file (editor tabs) |
| ⌘1…9 | Activate tab by position |
| ⌃1…9 | Switch to project by position |
| ⌃Tab / ⌘⇧] | Next tab |
| ⌃⇧Tab / ⌘⇧[ | Previous tab |
| ⌘B | Toggle left sidebar |
| ⌘L | Toggle right sidebar |
| ⌘K / ⌘P | Search |
| ⌘C / ⌘V | Copy / paste in terminal |

## Architecture

Cargo workspace, one crate per domain; UI crates depend on domain crates, never the reverse. Modules communicate through events (tokio channels) and GPUI entities.

```
helix/
  assets/      app icon: svg source, 1024 png, prerendered iconset
  scripts/     dev loop, macOS bundler
  src/
    app/         binary: window bootstrap, macOS blur and icon integration
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
