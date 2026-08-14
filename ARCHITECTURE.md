# helix — Architecture

A ground-up native rewrite of the original TypeScript controller for coding agents (Claude Code,
Codex, Cursor, Grok, Hermes, Pi) in Rust, with a gpui UI. Fresh app; no backwards compatibility
required.

**Pillars:**
- **Local only.** One process, one data directory. No accounts, no server, no network sync. The
  machine you run it on is the whole system.
- Everything device-side is Rust.
- Feature parity with the original **except token-usage display** and everything that existed to
  serve multiple devices.
- Frontend is **gpui** (pinned Zed rev). Virtualization + markdown techniques ported from
  **mugen + pretext** (`docs/research/mugen-pretext.md`).
- Smooth transitions/animations matching the original (catalog in
  `docs/research/feature-inventory.md` §1.12).

## 1. Topology

```
gpui UI ─ in-proc RPC ─ engine (sessions, terminals, repos, docs, local SQLite store)
```

- **Engine = backend**: runs agents, owns terminals, repos/worktrees, diff production, doc
  hosting, agent-CLI credential slots. Pure Rust, driven entirely through the typed RPC.
- **UI = viewport**: gpui app rendering engine state. Organized around **spaces** — (device,
  folder) pairs; with one device that reads as "one row per folder you work in". The sidebar is
  the data: an attention-sorted Sessions list, filtered by a searchable spaces dropdown ("All
  spaces" included) that also hosts space management. The horizontal tabs are a **device-local
  viewport** onto that list (`ui-settings.json openTabs`, cross-space): closing a tab is local
  only — archiving is an explicit sidebar action — and a sidebar click (re)opens a session as a
  tab. The new-session canvas carries a space picker (defaulting to the sidebar filter, else the
  last selected space).

### One process
Single binary `helix`, one mode, no subcommands: the window runs the engine **in-process** (RPC
over an in-memory duplex — same envelopes, same dispatch loop, zero serialization shortcuts, so
the boundary stays honest). Nothing is served on a port, so no second viewport, headless engine,
or daemon can attach. An exclusive lock on the data dir (`InstanceLock`) makes a second instance
fail loudly instead of racing the SQLite stores.

## 2. Data model

Two stores, both persisted through `helix-store` (SQLite at `profiles/local/docs.sqlite3`):

1. **Session doc** (per chat, Loro CRDT) — the transcript + durable command queue. `meta` map,
   `messages` list (parts as list-of-maps with **LoroText bodies** — the measured 1.03× oplog
   shape; never LWW value rewrites), `commands` list with ledger rules 1–3 (append-only entries;
   host-only outcomes; dedupe/TTL/supersede evaluation). Continuation splitting at 256KB,
   render-only tool parts (full inputs stay in the local run journal). Constants carried over
   (`STREAM_COMMIT_MS=120`, compaction at 8MB, retain 30d, tail 64). Loro stays because the
   transcript is an append-heavy structure with fine-grained text edits and a proven schema — not
   because anything replicates it.

2. **Workspace registry** (`helix-doc::registry`) — the four sidebar tables: **spaces** (id,
   deviceId, path, name?, gitDetected, checkoutId — the app's unit of organization; SpacesSync
   stamps git presence so branch pickers / the diff sidebar gate on a bool, no RPC), **chats**
   index (id, deviceId, title, archived, cwd, branch, checkoutId, spaceId, lastSeenAt,
   lastMessagePreview/At, config), **devices** (id, name, platform, lastSeenAt, version), and
   **session status** rows (Working indicator; staleness-checked client-side so a crashed run
   never shows eternal "Working"). Rows are merged by `apply_op` under an HLC clock: per-field
   clocks, tombstone deletes, `deleteSpace` cascading the space row plus every chat/session row
   in it. Local writes are stamped and applied in place — there is nothing to acknowledge them.
   `lastSeenAt` on a chat is the seen marker behind the "completed (unseen)" indicator.

   *Why one row table and not N docs:* the sidebar needs one subscription for the whole list
   (grouping, resort animations, unseen markers) and one snapshot to persist.

   The legacy Loro workspace doc (`workspace2`) is still read once, as the migration source when
   no registry snapshot exists; the snapshot stays on disk for rollback.

3. **Mirror layer** (`helix-doc`) — typed structs for the schema and **incremental** application
   of `doc.subscribe` diffs into cached state (no full re-hydration per change — the fix for the
   original's O(transcript) re-projection). The UI renders mirror state directly, with transcript
   deltas (`transcript_delta`) on the RPC stream instead of whole-Vec frames.

### Command plane
Send/steer/interrupt/respondInput = durable command entries in the session doc (`QueueCommand`),
drained by the engine that hosts the chat (mark-processed BEFORE execute, so a crash mid-run can
never double-execute; steer with no live run dispatches as the next turn). The ledger is the
reason a kill -9 mid-turn resumes cleanly instead of replaying side effects.

## 3. Cargo workspace

```
helix/
  Cargo.toml                 # workspace
  crates/
    proto/        helix-proto    # wire types: AgentEvent, ToolCall, RunRequest, Model,
                                 # entities, RPC envelopes (serde; ndjson framing);
                                 # `view` = the pure derivations the UI reads
                                 # (sort orders, staleness gating, grouping)
    doc/          helix-doc      # session-doc schema + mirror layer, parts fold,
                                 # continuations, command ledger, transcript deltas,
                                 # workspace registry (HLC row tables)
    store/        helix-store    # SQLite: doc snapshots + processed-command ledger
    harness/      helix-harness  # Harness trait + claude-code (stream-json subprocess),
                                 # codex (app-server JSON-RPC), ACP agents (Grok, Hermes,
                                 # pi), mock; steering mailbox, requestInput,
                                 # models/reasoning/options catalogs, login-shell env
    engine/       helix-engine   # sessions engine (pub/sub, run journal, recovery, stall
                                 # watchdog), doc host + command executor, repos/worktrees,
                                 # checkout-diff production, terminals (portable-pty),
                                 # uploads, agent accounts (cred swap), titles, spaces
    rpc/          helix-rpc      # typed req/resp/stream control plane over the in-memory
                                 # transport (one dispatch loop, ndjson envelopes)
    syntax/       helix-syntax   # tree-sitter highlight spans (no UI/engine deps)
    ui/           helix-ui       # gpui app: shell, sidebar, transcript, composer,
                                 # terminal view, changes pane, settings, animation kit
  apps/
    helix/                       # the binary: logging, data dir, window
  docs/                          # this file lives at the root; research reports here
```

Engine async runtime: **tokio** throughout; the UI bridges via `gpui_tokio` (`Tokio::spawn`
futures surfaced as gpui `Task`s). The engine runs on its own tokio runtime; the UI never blocks
on it.

## 4. UI plan (gpui) — parity + smoothness

Reference: `docs/research/gpui.md`, `docs/research/mugen-pretext.md`,
feature spec `docs/research/feature-inventory.md` §1.

- **Deps**: `gpui` + `gpui_platform` pinned to one Zed rev (Apache-2.0). **We do not use Zed's
  GPL crates** (`markdown`, `ui`, `theme`, `editor`) — markdown, components, and theme are ours.
- **Transcript**: gpui `list()` + `ListState::new(n, ListAlignment::Bottom, overdraw)` (sum-tree
  offsets, follow-tail). On top of it, the mugen behaviors gpui doesn't give us:
  - stick-to-bottom **spring** with feed-forward tracking of streaming growth; interrupt from
    *user input* (wheel-up / drag), re-engage within a 70px band; own-send re-engages + smooth
    scrolls;
  - **block-granularity rows** (one row = one markdown block / tool group, not one message) with
    stable ids `msgId#blockId`; live turn stays unsplit, re-splits on persist; optimistic echo
    rows share the client-minted id so persistence never flickers;
  - row height memoization keyed by (row id, content length, width) so a streamed token
    re-measures one row;
  - scroll-anchor absorption for above-viewport height changes.
- **Markdown** (`helix-ui::markdown`): `pulldown-cmark` parsing on `background_spawn` with
  coalescing (Zed's proven pattern), block-level incremental re-parse of the streaming tail
  (incremark's O(delta) idea: only re-parse from the last stable block boundary), monochrome
  theme where **numbers drive layout, colors are paint**. Code blocks: monospace, no wrap ⇒
  height = lines × line-height (layout independent of highlight); syntax highlighting via
  `helix-syntax` (tree-sitter) run time-sliced in the background, colors applied as text runs
  (paint-only). Streaming **fade-in veil** on newly appended text via `with_animation` opacity
  (paint-layer, never affects layout). `prefers-reduced-motion` honored.
- **Composer**: hand-rolled gpui text input (IME, selection, clipboard, key actions),
  compact↔expanded auto-flip by measured text width, auto-grow 76–260px, Enter/Shift+Enter,
  Send→Steer→Stop morph, drafts + attachments per chat, drag-drop/paste images, QuestionPanel
  (paged, 1-9 keys, 220ms auto-advance) replacing the composer while input is requested. Pickers
  (harness/model, traits, repo w/ folder browser, branch w/ worktree toggle) as gpui popovers
  with `menu-in` scale/fade.
- **Terminal**: `alacritty_terminal` (vte state machine, MIT/Apache) + `portable-pty` on the
  engine side; custom gpui grid element; tabs w/ drag-reorder (150ms sliding transforms), height
  drag 160px–55vh, 12ms input coalescing / 80ms resize debounce, 1MB replay, detach ≠ close.
- **Changes pane**: unified-patch parser → virtualized file/hunk/line rows, per-file collapse
  (180ms height tween), time-sliced highlight, 200ms width transition on the pane itself.
- **Animation kit** (`helix-ui::motion`): helpers over gpui `Animation` reproducing the original's
  catalog — `fade-in` (0.5s, cubic-bezier(0.16,1,0.3,1), translateY 4→0), `splash-out`,
  `helix-pulse` staggered cell wave (boot splash + loaders), `gradient-spin-pulse` matrix
  spinner (WorkingIndicator + rotating flavour word), `menu-in`/`dialog-in` scale-fades, 200ms
  ease-out width/height transitions for sidebar/panes, sidebar-resort **slide animation**
  (we own the list, so animate row positions directly — the View Transitions equivalent, 260ms
  cubic-bezier(0.22,1,0.36,1)), reduced-motion switch.
- **Theme**: always-dark monochrome, oklch-derived neutral scale precomputed to Hsla, hairline
  borders, Geist/Geist Mono bundled fonts.
- **Boot**: the shell renders from `ConnectionStatus` — splash while the engine assembles, an
  error card with Retry if assembly fails, the app when it is Ready. There is no sign-in or
  organization gate.

## 5. Engine plan

Direct ports of the original's behaviors (spec: feature-inventory §3):
- **Sessions engine**: per-session broadcast hub; on-disk run journal (resumable `seq` replay,
  crash auto-resume); persistent steerable sessions (steering mailbox at step/turn boundary; idle
  reaper; 10min stall watchdog); recovery stamps `aborted`.
- **Doc host**: per-chat handle (write user entries, stream assistant segments at 120ms commits,
  drain commands with processed-ledger idempotence, debounced snapshot saves); a warm-doc LRU
  (cap 12 + byte budget, watched/running/pending-command docs pinned) so reopening is a ~11ms
  SQLite load instead of unbounded residency.
- **Harness**: trait mirroring the original's `HarnessShape`; Claude Code via `claude` CLI
  stream-json in/out (control protocol for permissions/AskUserQuestion→requestInput, resume,
  steering); Codex via app-server JSON-RPC; Grok/Hermes/pi over ACP; model/reasoning/option
  catalogs ported from the TS `packages/harness`.
- **Repos/diffs**: `git` subprocess (matches the original, avoids libgit2 edge cases); worktrees
  under `~/.helix/worktrees`; fs watchers (`notify`) + 2min repair tick; diff capture (patch +
  numstat + untracked, 3MiB cap, sha256) published on the local `WatchCheckoutDiffs` stream and
  reconciled into the registry's `branch`/`checkoutId` fields.
- **Agent accounts**: credential-slot swap (macOS Keychain via `security-framework`, files
  elsewhere), plan labels, usage probes, paste-code/browser-poll OAuth flows — these are the
  agent CLIs' own logins, nothing to do with app accounts.
- **Identity**: no auth. A stable `device-id` file plus a `local-profile.json` uuid scope the data
  dir; device rows exist so the UI has a name to show.

## 6. Deliberate exclusions

- **Multi-device sync** — CRDT rooms, the Cloudflare Worker + Durable Objects, the device-room
  relay, `targetDeviceId` RPC forwarding, presence heartbeats, and the registry room protocol.
- **Accounts** — WorkOS sign-in, organizations, the sign-in/org gates, per-account profiles, and
  the local→synced import wizard.
- **Daemon / second viewport** — headless mode, the localhost IPC WebSocket server, and
  launchd/systemd service management. The app embeds its engine and nothing else can attach.
- **Auto-update** — the release checker, staged bundle swap, and update strip.
- **Mobile** — the iOS app.
- **Token-usage display** — profile heatmap, lifetime stats, per-message token columns.
  Rate-limit meters on agent accounts are *kept* (probed from the CLIs).
- **Tool-output sidecar** — full outputs live in the doc and the run journal; there is no blob
  store to fetch them from.

**Kept verbatim** from the original: session-doc schema shape + constants, command ledger rules,
render-parts privacy policy, UX behaviors and animation timings.

## 7. History

Status legend: ✅ shipped · 🟡 shipped with named gaps · ⚪️ built then removed.

- ✅ **M0 Scaffold** — workspace builds; `proto`/`doc` crates with ledger + parts + continuation
  unit tests; gpui hello-window runs.
- ⚪️ **M1 Doc + sync core** — Loro room client, edge convergence tests. Removed with the sync
  layer; the doc/mirror half of it is what `helix-doc` still is.
- ✅ **M2 Engine core** — Claude harness end-to-end: the embedded engine runs a turn, journal +
  doc writes, recovery test.
- ✅ **M3 UI core** — shell (sidebar/panes/header), transcript (virtualized, markdown, streaming,
  stick-to-bottom), composer (send/steer/stop, question panel).
- ⚪️ **M4 Multi-device** — device-room host/client virtual sockets, workspace doc entity sync,
  presence. Removed.
- 🟡 **M5 Full surface** — terminals, changes pane, repo/branch/folder pickers + worktrees,
  agent accounts UI, settings (devices/shortcuts/archived/appearance/notifications), Codex + ACP
  harnesses. Gaps: composer attachment UI (engine upload RPCs exist).
- 🟡 **M6 Polish** — keyboard map, clippy/fmt sweep, Linux packaging (`scripts/package-linux.sh`
  + release profile), macOS bundling (`scripts/package-macos.sh`, `dist/macos/`). Gaps:
  prefers-reduced-motion coverage, engine hardening (watchdogs beyond the current stall timer).

## 8. Open questions (tracked, non-blocking)

1. Text shaping performance for analytic row heights: gpui measures shaped text natively (Rust ⇒
   cheap), so we use gpui `list()` measurement + memoization rather than porting pretext's full
   analytic kernel; revisit only if cold-open of huge transcripts measures slow.
2. The registry's HLC merge is heavier than a single-writer store needs. It stays because rows
   carry per-field clocks that a claim-vs-create race depends on; a flat last-write-wins store
   would need that race re-solved first.
3. `materialize_tail` and the legacy `WorkspaceDoc` reader are kept for the migration path only
   and have no live caller.
