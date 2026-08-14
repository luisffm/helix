# Helix

A native desktop app for driving coding agents (Claude Code, Codex, Cursor, Grok, Hermes, Pi)
on your own machine. Rust end to end, [gpui](https://www.gpui.rs) for the UI.

Everything is local: one process, one data directory, no accounts, no daemon, no network
sync. Sessions, transcripts, worktrees, and agent logins live on this device only.

## Build and run

Needs a Rust toolchain (see `rust-toolchain.toml`; edition 2024) and, on Linux, the usual
gpui build dependencies.

```bash
cargo run -p helix          # debug build, ~/.helix as the data dir
cargo build -p helix --release
```

The first build takes a while — gpui and tree-sitter dominate it.

## Development loop

```bash
scripts/dev.sh              # watch + rebuild + relaunch, mock harness, /tmp/helix-dev-data
scripts/dev.sh --claude     # drive the real claude-code CLI instead of the mock
scripts/dev.sh --slow       # pace mock streams so streaming is watchable
scripts/dev.sh --data DIR   # use another data dir
```

There is no hot reload: gpui is a native app, so every change rebuilds and restarts the
process. The data dir survives; window state does not. `scripts/dev.sh` needs
`cargo install cargo-watch`.

Tests:

```bash
cargo test --workspace
```

## Packaging

```bash
scripts/package-macos.sh    # dist/Helix.app (icon from dist/helix.png, ad-hoc signed)
scripts/package-linux.sh    # tarball with the binary, .desktop entry, and icon
```

## Data directory

`~/.helix` by default, `$HELIX_DATA_DIR` to override. Inside it:

| path                       | what                                                      |
| -------------------------- | --------------------------------------------------------- |
| `profiles/local/`          | doc snapshots + the processed-command ledger (SQLite)      |
| `profiles/local/journals/` | per-chat run journals (crash recovery)                     |
| `profiles/local/uploads/`  | attachments committed for agent runs                       |
| `logs/`                    | one log file per launch, previous launch kept as `.old`    |
| `device-id`                | stable per-installation id                                 |
| `worktrees/`               | worktrees Helix creates for sessions                       |

An existing `~/.helix-native` directory is adopted on first launch if `~/.helix` is absent.

## Useful environment variables

| variable                    | effect                                                        |
| --------------------------- | ------------------------------------------------------------- |
| `HELIX_DATA_DIR`            | data directory override                                       |
| `HELIX_HARNESS`             | default harness for new chats (`mock`, `codex`, `cursor`, …)   |
| `HELIX_WORKTREES_DIR`       | where session worktrees are created                           |
| `HELIX_DEVICE_NAME`         | name shown for this device                                    |
| `HELIX_DISABLE_SOUND`       | mute the completion chime                                     |
| `HELIX_DISABLE_NOTIFICATIONS` | no OS notifications                                         |
| `RUST_LOG`                  | log filter (default `info`, loro quieted to `warn`)            |

Mock-harness knobs (`HELIX_MOCK_*`) and capture knobs (`HELIX_OPEN_ROUTE`,
`HELIX_FORCE_*`, `HELIX_MOTION_SCALE`) are documented where they are read.

## Layout

```
apps/helix        the binary: logging, data dir, window
crates/ui         gpui shell — sidebar, transcript, composer, terminal, diffs, settings
crates/engine     sessions, doc host + command executor, repos/worktrees, terminals, uploads
crates/harness    agent CLIs (Claude Code, Codex, Cursor, Grok, Hermes, Pi, mock)
crates/doc        Loro session docs + the workspace registry
crates/store      SQLite snapshots and the processed-command ledger
crates/rpc        typed control plane over the in-process transport
crates/proto      wire types shared by engine, UI, and RPC
crates/syntax     tree-sitter highlighting
```

`ARCHITECTURE.md` covers the engine/UI split in depth.

## License

MIT — see `LICENSE`.
