//! App state: the engine connection, entity lists, and the selected chat's
//! transcript — one gpui [`Entity`] the whole shell renders from.
//!
//! ## EngineHandle
//! The engine is embedded in this process (ARCHITECTURE §1):
//! [`EngineHandle::bootstrap`] assembles it and talks the typed RPC over the
//! in-memory transport ([`InProcessEngine`]) — same envelopes, same dispatch.
//!
//! ## Async bridging
//! `bootstrap` runs on tokio via `gpui_tokio::Tokio::spawn`. Once an [`RpcClient`]
//! exists, its `call`/`subscribe` futures are runtime-agnostic (tokio channels),
//! so subscription pumps run on gpui's own executor via `cx.spawn` and fold each
//! frame into the entity with `this.update(...)` + `cx.notify()`.
//!
//! Pure logic (sort order, staleness) lives in free functions with unit tests;
//! rendering reads them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use gpui::{App, Context, Entity, Task};
use gpui_tokio::Tokio;
use serde::de::DeserializeOwned;

use helix_doc::{SessionMessageEntry, TranscriptDesync, TranscriptFrame};
use helix_engine::{Engine, EngineConfig, EngineRuntime, InstanceLock};
use helix_proto::{Chat, ChatIndicator, EngineInfo, HarnessId, Session, Space};
use helix_rpc::{RpcClient, RpcService, memory_client, methods};

// ---------------------------------------------------------------------------
// Engine handle
// ---------------------------------------------------------------------------

/// Everything needed to start the embedded engine.
#[derive(Debug, Clone)]
pub struct EngineBootConfig {
  /// Data directory for the embedded engine (`~/.helix`).
  pub data_dir: PathBuf,
  /// Harness for doc-command runs until per-chat config lands (M4).
  pub default_harness: HarnessId,
}

/// The embedded engine: owns the [`EngineRuntime`] and its in-memory RPC loop.
struct InProcessEngine {
  runtime: tokio::sync::Mutex<Option<EngineRuntime>>,
  client: RpcClient,
}

/// Cheaply clonable handle to this process's engine.
#[derive(Clone)]
pub struct EngineHandle {
  inner: Arc<InProcessEngine>,
  engine_info: EngineInfo,
}

impl EngineHandle {
  /// Assemble the engine in this process and connect to it over the in-memory
  /// transport. Must run on the tokio runtime (`Tokio::spawn`): assembly spawns
  /// tokio tasks.
  pub async fn bootstrap(config: EngineBootConfig) -> anyhow::Result<EngineHandle> {
    let engine_config = EngineConfig {
      data_dir: config.data_dir,
      default_harness: config.default_harness,
    };
    // Own the data dir before opening anything under it: two engines on one
    // data dir would race the SQLite snapshots and journals.
    std::fs::create_dir_all(&engine_config.data_dir)?;
    let lock = InstanceLock::acquire(&engine_config.data_dir)?;
    let engine_info = Engine::engine_info(&engine_config)?;
    let runtime = Engine::assemble_runtime_with_lock(&engine_config, lock).await?;
    let service: Arc<dyn RpcService> = runtime.core().rpc_service();
    let client = memory_client(service);
    Ok(EngineHandle {
      inner: Arc::new(InProcessEngine {
        runtime: tokio::sync::Mutex::new(Some(runtime)),
        client,
      }),
      engine_info,
    })
  }

  pub fn client(&self) -> &RpcClient {
    &self.inner.client
  }

  pub fn engine_info(&self) -> &EngineInfo {
    &self.engine_info
  }

  /// Graceful teardown: settles live runs and flushes every open doc.
  pub async fn shutdown(&self) {
    if let Some(runtime) = self.inner.runtime.lock().await.take() {
      runtime.shutdown().await;
    }
  }
}

// ---------------------------------------------------------------------------
// Pure state + reducers
// ---------------------------------------------------------------------------

// The frontend-agnostic derivations (sort orders, staleness gating, sidebar
// grouping, the boot gate, relative times) live in `helix_proto::view`, pure
// and with their own test suite. Re-exported here because every call site in
// this crate reads them as `state::…`.
pub use helix_proto::view::{
  ChatGroup, ConnectionStatus, Indicator, SESSION_STALE_MS, attention_rank, chat_location,
  display_status, effective_indicator, format_time_ago, group_chats, project_label, sort_active,
  sort_chats, sort_spaces, sort_tabs,
};

// ---------------------------------------------------------------------------
// AppState entity
// ---------------------------------------------------------------------------

/// A composer send whose doc command is queued but not yet executed by the
/// chat's host device — cleared when the host writes the user message back
/// into the transcript (same client-minted id as the [`AppState::echoes`]
/// dedup), or after [`PENDING_SEND_TTL_MS`].
#[derive(Debug, Clone)]
struct PendingSend {
  message_id: String,
  started: DateTime<Utc>,
}

/// How long the send-in-flight overlay may hold before the synced status
/// shows through again. Covers the queue → nudge → drain → sync round-trip
/// to a remote host; when the host is offline the dot falls back to the
/// truth after this.
pub const PENDING_SEND_TTL_MS: i64 = 30_000;

/// Root application state. Reducer methods (`apply_*`, [`Self::session_for`], …)
/// are plain `&mut self` functions so tests construct the struct directly; gpui
/// glue ([`Self::bootstrap`], [`Self::select_chat`]) layers subscriptions on top.
pub struct AppState {
  pub connection: ConnectionStatus,
  /// Sorted (see [`sort_spaces`]).
  pub spaces: Vec<Space>,
  /// Sorted (see [`sort_chats`]); includes archived rows — views filter.
  pub chats: Vec<Chat>,
  pub sessions: Vec<Session>,
  /// The project the new-session canvas mints into. Healed by
  /// [`Self::apply_spaces`] when the row vanishes; selecting a chat implies
  /// its project.
  pub selected_space: Option<String>,
  /// Deliberate "Don't work in a project" pick: while set, the canvas mints
  /// project-less sessions (cwd `~` on the picked device) and
  /// [`Self::selected_space_row`] reads as `None` — healing must NOT
  /// re-select a project underneath it.
  pub no_project: bool,
  pub selected_chat: Option<String>,
  /// Boot auto-select happened (or a manual selection superseded it).
  pub auto_selected: bool,
  /// First chats / spaces watch frame has landed — device-local state that
  /// prunes against the doc (open tabs, the sidebar space filter) must not
  /// judge by the empty pre-sync lists.
  pub chats_synced: bool,
  pub spaces_synced: bool,
  /// Joined transcript of the selected chat (continuations folded engine-side).
  pub transcript: Vec<SessionMessageEntry>,
  /// Optimistic user echoes per chat id, shown until the doc frame carrying
  /// the same message id arrives (client-minted ids make dedup exact).
  echoes: HashMap<String, Vec<SessionMessageEntry>>,
  /// Send-in-flight overlay per chat id: a queued doc command the host
  /// hasn't executed yet (see [`Self::begin_pending_send`]).
  pending_sends: HashMap<String, PendingSend>,
  /// This engine's device id (best-effort `LocalDevice` probe; `None` until
  /// the engine serves it — views degrade gracefully).
  pub local_device_id: Option<String>,
  /// Data directory (`ui-settings.json`, `composer-defaults.json`); set at
  /// bootstrap so child views can persist small preference files.
  pub data_dir: Option<PathBuf>,
  engine: Option<EngineHandle>,
  watch_tasks: Vec<Task<()>>,
  transcript_task: Option<Task<()>>,
}

impl Default for AppState {
  fn default() -> Self {
    Self::new()
  }
}

impl AppState {
  pub fn new() -> Self {
    Self {
      connection: ConnectionStatus::Connecting,
      spaces: Vec::new(),
      chats: Vec::new(),
      sessions: Vec::new(),
      selected_space: None,
      no_project: false,
      selected_chat: None,
      transcript: Vec::new(),
      echoes: HashMap::new(),
      pending_sends: HashMap::new(),
      local_device_id: None,
      data_dir: None,
      engine: None,
      watch_tasks: Vec::new(),
      transcript_task: None,
      auto_selected: false,
      chats_synced: false,
      spaces_synced: false,
    }
  }

  // ---- reducers (pure) ----

  pub fn apply_chats(&mut self, mut chats: Vec<Chat>) {
    sort_chats(&mut chats);
    self.chats = chats;
    self.chats_synced = true;
    if let Some(selected) = &self.selected_chat
      && !self.chats.iter().any(|c| &c.id == selected)
    {
      // Selected chat vanished (deleted elsewhere): drop selection + transcript.
      self.selected_chat = None;
      self.transcript.clear();
      self.transcript_task = None;
    }
  }

  pub fn apply_sessions(&mut self, sessions: Vec<Session>) {
    self.sessions = sessions;
  }

  pub fn apply_spaces(&mut self, mut spaces: Vec<Space>) {
    sort_spaces(&mut spaces);
    self.spaces = spaces;
    self.spaces_synced = true;
    // Heal a vanished selection (project deleted): fall back to the first
    // project; its chats died with it, so a matching chat selection is
    // healed by the accompanying chats frame (`apply_chats`).
    let first_space = self.spaces_sorted().first().map(|s| s.id.clone());
    if let Some(selected) = &self.selected_space
      && !self.spaces.iter().any(|s| &s.id == selected)
    {
      self.selected_space = first_space.clone();
    }
    // First frame with no selection yet: pick the first project so the
    // canvas never boots project-less by accident — unless the user
    // deliberately opted out.
    if self.selected_space.is_none() && !self.no_project {
      self.selected_space = first_space;
    }
  }

  /// Optimistic local echo of a `setChatConfig` mutate: stamp the row now so
  /// the chips update on click; the next chats watch frame carries the same
  /// value once the engine applies the LWW write.
  pub fn apply_chat_config(&mut self, chat_id: &str, config: helix_proto::ChatConfig) {
    if let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) {
      chat.config = Some(config);
    }
  }

  pub fn apply_transcript(&mut self, entries: Vec<SessionMessageEntry>) {
    // Doc frames supersede optimistic echoes carrying the same id.
    if let Some(chat_id) = self.selected_chat.as_deref()
      && let Some(echoes) = self.echoes.get_mut(chat_id)
    {
      echoes.retain(|echo| !entries.iter().any(|e| e.id == echo.id));
    }
    self.transcript = entries;
    self.ack_pending_send_from_transcript();
  }

  /// Apply a `WatchDocMessages` delta frame in place. `Err` = this copy has
  /// diverged; the watch task resubscribes for a fresh reset.
  pub fn apply_transcript_frame(&mut self, frame: TranscriptFrame) -> Result<(), TranscriptDesync> {
    helix_doc::apply_transcript_frame(&mut self.transcript, frame)?;
    if let Some(chat_id) = self.selected_chat.as_deref()
      && let Some(echoes) = self.echoes.get_mut(chat_id)
    {
      let transcript = &self.transcript;
      echoes.retain(|echo| !transcript.iter().any(|e| e.id == echo.id));
    }
    self.ack_pending_send_from_transcript();
    Ok(())
  }

  /// Add an optimistic user echo (composer send path).
  pub fn push_echo(&mut self, chat_id: &str, entry: SessionMessageEntry) {
    let echoes = self.echoes.entry(chat_id.to_string()).or_default();
    if !echoes.iter().any(|e| e.id == entry.id) {
      echoes.push(entry);
    }
  }

  /// Drop an echo (send failed — the prompt returns to the draft).
  pub fn remove_echo(&mut self, chat_id: &str, message_id: &str) {
    if let Some(echoes) = self.echoes.get_mut(chat_id) {
      echoes.retain(|e| e.id != message_id);
    }
  }

  /// Composer send fired: overlay the chat as Working until the host writes
  /// the user message back into the transcript (or the TTL lapses). A remote
  /// send has no live session row until the host drains the queued command —
  /// that gap read as "no live run" and flashed the Completed dot, and any
  /// phantom Working→Idle edge in it rang the done-chime on send (user
  /// report 2026-08-05).
  pub fn begin_pending_send(&mut self, chat_id: &str, message_id: &str, now: DateTime<Utc>) {
    self.pending_sends.insert(
      chat_id.to_string(),
      PendingSend {
        message_id: message_id.to_string(),
        started: now,
      },
    );
  }

  /// Send failed — drop the overlay so the dot tells the truth again. Only
  /// removes the overlay this message started: a quick resend must not lose
  /// its own overlay to the first send's failure cleanup.
  pub fn end_pending_send(&mut self, chat_id: &str, message_id: &str) {
    if self
      .pending_sends
      .get(chat_id)
      .is_some_and(|p| p.message_id == message_id)
    {
      self.pending_sends.remove(chat_id);
    }
  }

  /// Is a send still in flight for this chat (unacked, inside the TTL)?
  pub fn send_pending(&self, chat_id: &str, now: DateTime<Utc>) -> bool {
    self.pending_sends.get(chat_id).is_some_and(|p| {
      now.signed_duration_since(p.started).num_milliseconds() <= PENDING_SEND_TTL_MS
    })
  }

  /// When the in-flight send (if any, inside the TTL) was fired — the
  /// elapsed-timer base while the overlay reads as Working. The session
  /// row's `started_at` still belongs to the PREVIOUS turn during this
  /// window, and showing it made a fresh send open at the old turn's
  /// half-hour mark.
  pub fn pending_send_started(&self, chat_id: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    self
      .pending_sends
      .get(chat_id)
      .filter(|p| now.signed_duration_since(p.started).num_milliseconds() <= PENDING_SEND_TTL_MS)
      .map(|p| p.started)
  }

  /// The host executed the queued command iff the sent message's id showed
  /// up in the transcript (it writes the message before — causally with —
  /// the Working status; sessions.rs dispatch paths).
  fn ack_pending_send_from_transcript(&mut self) {
    if let Some(chat_id) = self.selected_chat.as_deref()
      && let Some(pending) = self.pending_sends.get(chat_id)
      && self.transcript.iter().any(|e| e.id == pending.message_id)
    {
      self.pending_sends.remove(chat_id);
    }
  }

  /// Unconfirmed echoes for the selected chat, in send order.
  pub fn pending_echoes(&self) -> &[SessionMessageEntry] {
    self
      .selected_chat
      .as_deref()
      .and_then(|id| self.echoes.get(id))
      .map(|v| v.as_slice())
      .unwrap_or(&[])
  }

  // ---- queries ----

  /// Non-archived chats in sidebar order.
  pub fn visible_chats(&self) -> impl Iterator<Item = &Chat> {
    self.chats.iter().filter(|c| !c.archived)
  }

  pub fn selected_space_row(&self) -> Option<&Space> {
    if self.no_project {
      return None;
    }
    let id = self.selected_space.as_deref()?;
    self.spaces.iter().find(|s| s.id == id)
  }

  pub fn space_row(&self, space_id: &str) -> Option<&Space> {
    self.spaces.iter().find(|s| s.id == space_id)
  }

  /// Spaces in display order — case-insensitive alphabetical, the order
  /// both space selectors (sidebar filter, composer picker) list rows in.
  /// Ties break on id so the order is stable across renders.
  pub fn spaces_sorted(&self) -> Vec<&Space> {
    let mut spaces: Vec<&Space> = self.spaces.iter().collect();
    spaces.sort_by_key(|s| (s.display_name().to_lowercase(), s.id.clone()));
    spaces
  }

  pub fn space_for_chat(&self, chat: &Chat) -> Option<&Space> {
    self.space_row(chat.space_id.as_deref()?)
  }

  /// Non-archived chats of a space in tab (creation) order. Chats with a
  /// dangling/missing `space_id` are invisible by construction.
  pub fn chats_in_space(&self, space_id: &str) -> Vec<&Chat> {
    let mut chats: Vec<&Chat> = self
      .visible_chats()
      .filter(|c| c.space_id.as_deref() == Some(space_id))
      .collect();
    sort_tabs(&mut chats);
    chats
  }

  /// Does the selected space's folder have git? Drives the branch picker and
  /// the diff sidebar (owner-stamped, synced — no RPC).
  pub fn selected_space_git(&self) -> bool {
    self.selected_space_row().is_some_and(|s| s.git_detected)
  }

  /// Full display status for a chat (tab dots, Active list). A send in
  /// flight ([`Self::begin_pending_send`]) reads as Working — the queued
  /// command is as good as running.
  pub fn display_status_for(&self, chat: &Chat, now: DateTime<Utc>) -> ChatIndicator {
    if self.send_pending(&chat.id, now) {
      return ChatIndicator::Working;
    }
    display_status(chat, self.session_for(&chat.id), now)
  }

  /// The sidebar's Sessions list: every non-archived chat of a LIVE space,
  /// on any device — idle included — in pure recency order (status drives
  /// the dot, never the position; see [`sort_active`]).
  pub fn overview_chats(&self, now: DateTime<Utc>) -> Vec<(ChatIndicator, &Chat)> {
    let mut rows: Vec<(ChatIndicator, &Chat)> = self
      .visible_chats()
      .filter(|c| match c.space_id.as_deref() {
        // Project-less sessions are first-class rows.
        None => true,
        Some(id) => self.space_row(id).is_some(),
      })
      .map(|c| (self.display_status_for(c, now), c))
      .collect();
    sort_active(&mut rows);
    rows
  }

  pub fn session_for(&self, chat_id: &str) -> Option<&Session> {
    self.sessions.iter().find(|s| s.chat_id == chat_id)
  }

  /// Staleness-checked status dot for a chat row. A send in flight reads as
  /// Working (see [`Self::display_status_for`]).
  pub fn indicator_for(&self, chat_id: &str, now: DateTime<Utc>) -> Indicator {
    if self.send_pending(chat_id, now) {
      return Indicator::Working;
    }
    effective_indicator(self.session_for(chat_id), now)
  }

  pub fn selected_chat_row(&self) -> Option<&Chat> {
    let id = self.selected_chat.as_deref()?;
    self.chats.iter().find(|c| c.id == id)
  }

  pub fn engine(&self) -> Option<&EngineHandle> {
    self.engine.as_ref()
  }

  /// Drop every account-scoped view and subscription after its runtime has
  /// stopped. The next bootstrap must never render rows from the previous
  /// account while the local profile is opening.
  pub fn prepare_runtime_replacement(&mut self, cx: &mut Context<Self>) {
    self.engine = None;
    self.watch_tasks.clear();
    self.transcript_task = None;
    self.connection = ConnectionStatus::Connecting;
    self.spaces.clear();
    self.chats.clear();
    self.sessions.clear();
    self.selected_space = None;
    self.no_project = false;
    self.selected_chat = None;
    self.auto_selected = false;
    self.chats_synced = false;
    self.spaces_synced = false;
    self.transcript.clear();
    self.echoes.clear();
    self.pending_sends.clear();
    self.local_device_id = None;
    cx.notify();
  }

  // ---- gpui glue ----

  /// Kick off (or retry) the engine bootstrap: assemble on tokio, then attach
  /// subscriptions. Safe to call again after `Failed`.
  pub fn bootstrap(state: Entity<AppState>, config: EngineBootConfig, cx: &mut App) {
    let data_dir = config.data_dir.clone();
    state.update(cx, |s, cx| {
      s.connection = ConnectionStatus::Connecting;
      s.data_dir = Some(data_dir);
      cx.notify();
    });
    let boot = Tokio::spawn(cx, EngineHandle::bootstrap(config));
    cx.spawn(async move |cx| {
      let outcome = match boot.await {
        Ok(Ok(handle)) => Ok(handle),
        Ok(Err(err)) => Err(format!("{err:#}")),
        Err(join_err) => Err(join_err.to_string()),
      };
      // NB: at the pinned rev `Entity::update(&mut AsyncApp)` returns the
      // closure's value directly (no Result) — AsyncApp implements
      // AppContext like App does.
      state.update(cx, |s, cx| match outcome {
        Ok(handle) => s.attach_engine(handle, cx),
        Err(message) => {
          tracing::error!(%message, "engine bootstrap failed");
          s.connection = ConnectionStatus::Failed(message);
          cx.notify();
        }
      });
    })
    .detach();
  }

  /// Wire the connected engine: mark Ready and start the standing watches.
  fn attach_engine(&mut self, handle: EngineHandle, cx: &mut Context<Self>) {
    let engine_info = handle.engine_info();
    self.local_device_id = Some(engine_info.device_id.clone());
    self.engine = Some(handle.clone());
    let mut watch_tasks = Vec::with_capacity(6);
    watch_tasks.extend([
      spawn_watch(
        cx,
        handle.clone(),
        methods::WATCH_SESSIONS,
        AppState::apply_sessions,
      ),
      spawn_chats_watch(cx, handle.clone()),
      spawn_watch(
        cx,
        handle.clone(),
        methods::WATCH_SPACES,
        AppState::apply_spaces,
      ),
      spawn_local_device_probe(cx, handle.clone()),
    ]);
    self.watch_tasks = watch_tasks;
    self.connection = ConnectionStatus::Ready;
    // Re-subscribe the transcript if a chat was already selected (reconnect path).
    if let Some(chat_id) = self.selected_chat.clone() {
      self.transcript_task = Some(spawn_transcript_watch(cx, handle, chat_id));
    }
    cx.notify();
  }

  /// Select a chat (or clear). Swaps the per-chat doc-transcript subscription:
  /// dropping the old task drops its stream receiver, which cancels the doc
  /// watch server-side. Selecting a chat also lands in its space and marks it
  /// seen (a global-list click must switch the tab strip too).
  pub fn select_chat(&mut self, chat_id: Option<String>, cx: &mut Context<Self>) {
    if self.selected_chat == chat_id {
      // Re-selecting still clears a fresh "completed" badge.
      if let Some(id) = chat_id {
        self.mark_chat_seen(&id, cx);
      }
      return;
    }
    self.selected_chat = chat_id.clone();
    self.auto_selected = true;
    self.transcript.clear();
    self.transcript_task = None;
    if let Some(id) = chat_id.as_deref() {
      // A chat implies its project (or the lack of one); `select_chat(None)`
      // (the new-session canvas) keeps the current project pick.
      if let Some(chat) = self.chats.iter().find(|c| c.id == id) {
        match chat.space_id.clone() {
          Some(space_id) => {
            self.selected_space = Some(space_id);
            self.no_project = false;
          }
          None => self.no_project = true,
        }
      }
      self.mark_chat_seen(id, cx);
    }
    if let (Some(chat_id), Some(handle)) = (chat_id, self.engine.clone()) {
      self.transcript_task = Some(spawn_transcript_watch(cx, handle, chat_id));
    }
    cx.notify();
  }

  /// Select a project; the caller (shell) decides which chat to land on.
  /// `Some` clears a "Don't work in a project" opt-out; `None` IS that opt-out.
  pub fn select_space(&mut self, space_id: Option<String>, cx: &mut Context<Self>) {
    match &space_id {
      Some(_) => self.no_project = false,
      None => self.no_project = true,
    }
    if self.selected_space == space_id && space_id.is_some() {
      cx.notify();
      return;
    }
    if space_id.is_some() {
      self.selected_space = space_id;
    }
    cx.notify();
  }

  /// Synced seen marker: only fires when the chat is currently unseen
  /// (idempotence — no mutate spam), stamps the local row optimistically so
  /// the LWW round-trip is invisible, and fire-and-forgets the mutate.
  pub fn mark_chat_seen(&mut self, chat_id: &str, cx: &mut Context<Self>) {
    let Some(chat) = self.chats.iter_mut().find(|c| c.id == chat_id) else {
      return;
    };
    if !chat.unseen() {
      return;
    }
    chat.last_seen_at = Some(Utc::now());
    cx.notify();
    let Some(handle) = self.engine.clone() else {
      return;
    };
    let chat_id = chat_id.to_string();
    cx.spawn(async move |_, _| {
      let params = serde_json::json!({ "op": "markChatSeen", "chatId": chat_id });
      if let Err(err) = handle.client().call(methods::MUTATE, params).await {
        tracing::warn!(chat = %chat_id, error = %err, "markChatSeen failed");
      }
    })
    .detach();
  }
}

/// Chats watch. Boot selection is the shell's job (it lands on the first
/// restored open tab, device-local state this entity can't see); this task
/// only pumps frames.
fn spawn_chats_watch(cx: &mut Context<AppState>, handle: EngineHandle) -> Task<()> {
  cx.spawn(async move |this, cx| {
    // Resubscribe loop (same contract as the transcript watch): a daemon
    // restart or RPC drop ends the stream, and a bare return here froze
    // the sidebar until app restart — new chats, renames and archives
    // from every device silently stopped arriving.
    const RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
    loop {
      let mut rx = match handle
        .client()
        .subscribe(methods::WATCH_CHATS, serde_json::json!({}))
        .await
      {
        Ok(rx) => rx,
        Err(err) => {
          tracing::debug!(error = %err, "chats watch unavailable; retrying");
          if this.update(cx, |_, _| {}).is_err() {
            return;
          }
          cx.background_executor().timer(RETRY_DELAY).await;
          continue;
        }
      };
      while let Some(value) = rx.recv().await {
        let parsed: Vec<Chat> = match serde_json::from_value(value) {
          Ok(parsed) => parsed,
          Err(err) => {
            tracing::warn!(error = %err, "dropping malformed chats frame");
            continue;
          }
        };
        let alive = this.update(cx, |state, cx| {
          state.apply_chats(parsed);
          cx.notify();
        });
        if alive.is_err() {
          return;
        }
      }
      tracing::debug!("chats stream ended; resubscribing");
      if this.update(cx, |_, _| {}).is_err() {
        return;
      }
      cx.background_executor().timer(RETRY_DELAY).await;
    }
  })
}

fn spawn_watch<T: DeserializeOwned + 'static>(
  cx: &mut Context<AppState>,
  handle: EngineHandle,
  method: &'static str,
  apply: fn(&mut AppState, T),
) -> Task<()> {
  cx.spawn(async move |this, cx| {
    // Resubscribe loop: these are the standing Sessions/Devices/Spaces
    // watches — a daemon restart ended the stream and a bare return froze
    // them for the rest of the app's life (remote Working dots staled out
    // to nothing after 45s, and Idle/Completed transitions from other
    // devices never arrived again — "the session never completes").
    const RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
    loop {
      let mut rx = match handle
        .client()
        .subscribe(method, serde_json::json!({}))
        .await
      {
        Ok(rx) => rx,
        Err(err) => {
          tracing::debug!(method, error = %err, "watch unavailable; retrying");
          if this.update(cx, |_, _| {}).is_err() {
            return;
          }
          cx.background_executor().timer(RETRY_DELAY).await;
          continue;
        }
      };
      while let Some(value) = rx.recv().await {
        let parsed: T = match serde_json::from_value(value) {
          Ok(parsed) => parsed,
          Err(err) => {
            tracing::warn!(method, error = %err, "dropping malformed watch frame");
            continue;
          }
        };
        let alive = this.update(cx, |state, cx| {
          apply(state, parsed);
          cx.notify();
        });
        if alive.is_err() {
          return;
        }
      }
      tracing::debug!(method, "watch stream ended; resubscribing");
      if this.update(cx, |_, _| {}).is_err() {
        return;
      }
      cx.background_executor().timer(RETRY_DELAY).await;
    }
  })
}

/// Best-effort `LocalDevice` probe: fills `local_device_id` for the "This
/// device" badge. Engines that don't serve the method leave it `None`.
fn spawn_local_device_probe(cx: &mut Context<AppState>, handle: EngineHandle) -> Task<()> {
  cx.spawn(async move |this, cx| {
    let Ok(value) = handle
      .client()
      .call("LocalDevice", serde_json::json!({}))
      .await
    else {
      tracing::debug!("LocalDevice unavailable; skipping this-device badge");
      return;
    };
    let id = value
      .get("id")
      .or_else(|| value.get("deviceId"))
      .and_then(|v| v.as_str())
      .map(str::to_string);
    if let Some(id) = id {
      this
        .update(cx, |state, cx| {
          state.local_device_id = Some(id);
          cx.notify();
        })
        .ok();
    }
  })
}

fn spawn_transcript_watch(
  cx: &mut Context<AppState>,
  handle: EngineHandle,
  chat_id: String,
) -> Task<()> {
  cx.spawn(async move |this, cx| {
    // Outer loop: a delta desync (missed frame) resubscribes immediately
    // and the fresh stream's opening reset heals the copy; a subscribe
    // failure, malformed frame, or stream end retries on a delay. Every
    // path re-enters the loop — a return here freezes the transcript
    // with no banner and no heal short of an app restart (this watch and
    // its engine-side room are the ONLY transcript delivery path). The
    // task itself is dropped by select_chat/apply_chats when the chat is
    // deselected or deleted, so retrying can't outlive relevance.
    const RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
    'resubscribe: loop {
      let params = serde_json::json!({ "chatId": chat_id });
      let mut rx = match handle
        .client()
        .subscribe(methods::WATCH_DOC_MESSAGES, params)
        .await
      {
        Ok(rx) => rx,
        Err(err) => {
          tracing::warn!(%chat_id, error = %err, "transcript watch failed; retrying");
          if this.update(cx, |_, _| {}).is_err() {
            return;
          }
          cx.background_executor().timer(RETRY_DELAY).await;
          continue 'resubscribe;
        }
      };
      while let Some(value) = rx.recv().await {
        let frame: TranscriptFrame = match serde_json::from_value(value) {
          Ok(frame) => frame,
          Err(err) => {
            // Schema skew (a newer peer's entry shape arriving
            // through sync): a skipped frame is a silently stale
            // copy, so resubscribe for a fresh reset — delayed,
            // in case the reset itself is what can't parse.
            tracing::warn!(error = %err, "malformed transcript frame; resubscribing");
            cx.background_executor().timer(RETRY_DELAY).await;
            continue 'resubscribe;
          }
        };
        let mut desync = false;
        let alive = this.update(cx, |state, cx| {
          // Guard against a stale pump racing a newer selection.
          if state.selected_chat.as_deref() == Some(chat_id.as_str()) {
            if let Err(err) = state.apply_transcript_frame(frame) {
              tracing::warn!(%chat_id, error = %err, "resubscribing transcript");
              desync = true;
            }
            cx.notify();
          }
        });
        if alive.is_err() {
          return;
        }
        if desync {
          continue 'resubscribe;
        }
      }
      // Stream ended: engine restart, RPC drop, or chat purge. Retry;
      // the purge case is cleaned up by apply_chats dropping this task.
      tracing::debug!(%chat_id, "transcript stream ended; resubscribing");
      if this.update(cx, |_, _| {}).is_err() {
        return;
      }
      cx.background_executor().timer(RETRY_DELAY).await;
    }
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::TimeDelta;
  // `SessionStatus` is only needed to build the fixtures below — the module
  // itself derives everything through `helix_proto::view`.
  use helix_proto::SessionStatus;

  #[tokio::test]
  async fn bootstrap_embeds_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let handle = EngineHandle::bootstrap(EngineBootConfig {
      data_dir: dir.path().to_path_buf(),
      default_harness: HarnessId::Mock,
    })
    .await
    .unwrap();
    // Same protocol over the in-memory transport: a real engine answers.
    let harnesses = handle
      .client()
      .call(methods::LIST_HARNESSES, serde_json::json!({}))
      .await
      .unwrap();
    assert!(harnesses.as_array().is_some_and(|h| !h.is_empty()));
    handle.shutdown().await;
  }

  #[tokio::test]
  async fn bootstrap_reports_local_assembly_failure_before_returning_a_handle() {
    let dir = tempfile::tempdir().unwrap();
    helix_engine::EngineProfile::local(dir.path()).unwrap();
    std::fs::create_dir(dir.path().join("profiles")).unwrap();
    std::fs::write(dir.path().join("profiles/local"), b"not a directory").unwrap();

    let error = match EngineHandle::bootstrap(EngineBootConfig {
      data_dir: dir.path().to_path_buf(),
      default_harness: HarnessId::Mock,
    })
    .await
    {
      Ok(handle) => {
        handle.shutdown().await;
        panic!("a corrupt local store must fail bootstrap")
      }
      Err(error) => error,
    };

    assert!(!format!("{error:#}").is_empty());
  }

  fn chat(id: &str, created_min: i64, last_msg_min: Option<i64>) -> Chat {
    let base = DateTime::parse_from_rfc3339("2026-07-19T12:00:00Z")
      .unwrap()
      .to_utc();
    Chat {
      id: id.into(),
      device_id: "dev".into(),
      title: None,
      archived: false,
      cwd: None,
      branch: None,
      checkout_id: None,
      config: None,
      last_message_preview: None,
      last_message_at: last_msg_min.map(|m| base + TimeDelta::minutes(m)),
      created_at: base + TimeDelta::minutes(created_min),
      harness_session_id: None,
      harness_session_cwd: None,
      space_id: None,
      last_seen_at: None,
    }
  }

  fn space(id: &str, device_id: &str, path: &str, created_min: i64) -> Space {
    let base = DateTime::parse_from_rfc3339("2026-07-19T12:00:00Z")
      .unwrap()
      .to_utc();
    Space {
      id: id.into(),
      device_id: device_id.into(),
      path: path.into(),
      name: None,
      git_detected: false,
      git_checked_at: None,
      checkout_id: None,
      created_at: base + TimeDelta::minutes(created_min),
    }
  }

  fn session(
    chat_id: &str,
    status: SessionStatus,
    updated_secs_ago: i64,
    now: DateTime<Utc>,
  ) -> Session {
    Session {
      chat_id: chat_id.into(),
      device_id: "dev".into(),
      status,
      started_at: None,
      updated_at: now - TimeDelta::seconds(updated_secs_ago),
    }
  }

  fn user_entry(id: &str) -> SessionMessageEntry {
    SessionMessageEntry {
      id: id.into(),
      role: helix_doc::MessageRole::User,
      parts: Vec::new(),
      created_at: 0,
      device_id: "dev".into(),
      status: None,
      continuation_of: None,
    }
  }

  #[test]
  fn send_pending_overlays_working_until_ttl() {
    let now = Utc::now();
    let s_chat = chat("c", 0, Some(10)); // unseen, no session row
    let mut s = AppState::new();
    assert_eq!(s.display_status_for(&s_chat, now), ChatIndicator::Completed);
    assert_eq!(s.indicator_for("c", now), Indicator::None);
    s.begin_pending_send("c", "m1", now);
    assert_eq!(s.display_status_for(&s_chat, now), ChatIndicator::Working);
    assert_eq!(s.indicator_for("c", now), Indicator::Working);
    // Time-bounded: an offline host must not leave an eternal spinner.
    let later = now + TimeDelta::milliseconds(PENDING_SEND_TTL_MS + 1);
    assert_eq!(
      s.display_status_for(&s_chat, later),
      ChatIndicator::Completed
    );
    assert_eq!(s.indicator_for("c", later), Indicator::None);
  }

  #[test]
  fn send_pending_acked_when_the_host_writes_the_message_back() {
    let now = Utc::now();
    let mut s = AppState::new();
    s.selected_chat = Some("c".into());
    s.begin_pending_send("c", "m1", now);
    // A frame without the message keeps the overlay.
    s.apply_transcript(vec![user_entry("other")]);
    assert!(s.send_pending("c", now));
    // The host executed the command: our id comes back in the doc.
    s.apply_transcript(vec![user_entry("other"), user_entry("m1")]);
    assert!(!s.send_pending("c", now));
  }

  #[test]
  fn send_failure_cleanup_only_ends_its_own_overlay() {
    let now = Utc::now();
    let mut s = AppState::new();
    s.begin_pending_send("c", "m1", now);
    s.begin_pending_send("c", "m2", now); // quick resend superseded m1
    s.end_pending_send("c", "m1"); // m1's failure cleanup arrives late
    assert!(s.send_pending("c", now), "m2's overlay must survive");
    s.end_pending_send("c", "m2");
    assert!(!s.send_pending("c", now));
  }

  #[test]
  fn chats_sort_by_last_message_desc_with_created_fallback() {
    let mut chats = vec![
      chat("a", 0, Some(10)),
      chat("b", 5, None), // no messages → keys on created_at (+5min)
      chat("c", 1, Some(30)),
      chat("d", 40, None), // created after every message
    ];
    sort_chats(&mut chats);
    let order: Vec<&str> = chats.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(order, ["d", "c", "a", "b"]);
  }

  #[test]
  fn chat_sort_ties_are_deterministic() {
    let mut chats = vec![chat("z", 0, Some(10)), chat("a", 0, Some(10))];
    sort_chats(&mut chats);
    assert_eq!(chats[0].id, "a");
  }

  #[test]
  fn working_indicator_staleness() {
    let now = Utc::now();
    // Fresh working session shows.
    let fresh = session("c", SessionStatus::Working, 10, now);
    assert_eq!(effective_indicator(Some(&fresh), now), Indicator::Working);
    // Stale working session is suppressed — crashed backend, not eternal spinner.
    let stale = session("c", SessionStatus::Working, 46, now);
    assert_eq!(effective_indicator(Some(&stale), now), Indicator::None);
    // Exactly at the boundary still shows (strictly-older-than semantics).
    let edge = session("c", SessionStatus::Working, 45, now);
    assert_eq!(effective_indicator(Some(&edge), now), Indicator::Working);
    // Future timestamps (clock skew) count as fresh.
    let skewed = session("c", SessionStatus::Working, -30, now);
    assert_eq!(effective_indicator(Some(&skewed), now), Indicator::Working);
  }

  #[test]
  fn indicator_kinds() {
    let now = Utc::now();
    assert_eq!(effective_indicator(None, now), Indicator::None);
    let idle = session("c", SessionStatus::Idle, 0, now);
    assert_eq!(effective_indicator(Some(&idle), now), Indicator::None);
    // Errored is not staleness-gated: the error stays visible.
    let errored = session("c", SessionStatus::Errored, 600, now);
    assert_eq!(effective_indicator(Some(&errored), now), Indicator::Errored);
    let awaiting = session("c", SessionStatus::AwaitingInput, 5, now);
    assert_eq!(
      effective_indicator(Some(&awaiting), now),
      Indicator::AwaitingInput
    );
    let awaiting_stale = session("c", SessionStatus::AwaitingInput, 300, now);
    assert_eq!(
      effective_indicator(Some(&awaiting_stale), now),
      Indicator::None
    );
  }

  #[test]
  fn display_status_derivation() {
    let now = Utc::now();
    let mut c = chat("c", 0, Some(10));
    // Live states win regardless of seen.
    let working = session("c", SessionStatus::Working, 5, now);
    assert_eq!(
      display_status(&c, Some(&working), now),
      ChatIndicator::Working
    );
    let awaiting = session("c", SessionStatus::AwaitingInput, 5, now);
    assert_eq!(
      display_status(&c, Some(&awaiting), now),
      ChatIndicator::AwaitingInput
    );
    // Finished + unseen = Completed (no session row at all).
    assert_eq!(display_status(&c, None, now), ChatIndicator::Completed);
    // Idle session + unseen = Completed.
    let idle = session("c", SessionStatus::Idle, 5, now);
    assert_eq!(
      display_status(&c, Some(&idle), now),
      ChatIndicator::Completed
    );
    // Stale working session falls back to the seen check.
    let stale = session("c", SessionStatus::Working, 300, now);
    assert_eq!(
      display_status(&c, Some(&stale), now),
      ChatIndicator::Completed
    );
    // Seen after the last message = Idle.
    c.last_seen_at = c.last_message_at.map(|t| t + TimeDelta::minutes(1));
    assert_eq!(display_status(&c, Some(&idle), now), ChatIndicator::Idle);
    // Errored + unseen = Errored; seen clears it to Idle.
    let errored = session("c", SessionStatus::Errored, 600, now);
    assert_eq!(display_status(&c, Some(&errored), now), ChatIndicator::Idle);
    c.last_seen_at = None;
    assert_eq!(
      display_status(&c, Some(&errored), now),
      ChatIndicator::Errored
    );
    // No messages at all: nothing to see — Idle.
    let fresh = chat("f", 0, None);
    assert_eq!(display_status(&fresh, None, now), ChatIndicator::Idle);
  }

  #[test]
  fn active_list_sorts_by_recency_only_status_never_moves_rows() {
    let a = chat("a", 0, Some(10)); // Completed (older)
    let b = chat("b", 0, Some(20)); // Completed (newer)
    let c = chat("c", 0, Some(5)); // AwaitingInput
    let d = chat("d", 0, Some(1)); // Working
    let mut rows = vec![
      (ChatIndicator::Completed, &a),
      (ChatIndicator::Completed, &b),
      (ChatIndicator::AwaitingInput, &c),
      (ChatIndicator::Working, &d),
    ];
    sort_active(&mut rows);
    let order: Vec<&str> = rows.iter().map(|(_, c)| c.id.as_str()).collect();
    assert_eq!(order, ["b", "a", "c", "d"], "recency desc, status ignored");

    // Opening a completed session (completed → seen → idle) must NOT
    // change its position (user report: rows jumped under the pointer).
    let mut seen = vec![
      (ChatIndicator::Idle, &a),
      (ChatIndicator::Completed, &b),
      (ChatIndicator::AwaitingInput, &c),
      (ChatIndicator::Working, &d),
    ];
    sort_active(&mut seen);
    let order_after: Vec<&str> = seen.iter().map(|(_, c)| c.id.as_str()).collect();
    assert_eq!(order, order_after);
  }

  #[test]
  fn tabs_order_by_creation_not_activity() {
    let a = chat("a", 5, Some(100)); // created later, very active
    let b = chat("b", 1, Some(2));
    let mut tabs = vec![&a, &b];
    sort_tabs(&mut tabs);
    let order: Vec<&str> = tabs.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(order, ["b", "a"]);
  }

  #[test]
  fn apply_spaces_sorts_and_heals_selection() {
    let mut state = AppState::new();
    state.apply_spaces(vec![
      space("s2", "dev", "/b", 2),
      space("s1", "dev", "/a", 1),
    ]);
    let ids: Vec<&str> = state.spaces.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["s1", "s2"]);
    // First frame auto-selects the first space.
    assert_eq!(state.selected_space.as_deref(), Some("s1"));
    state.selected_space = Some("s2".into());
    // Vanished selection heals to the first space.
    state.apply_spaces(vec![space("s1", "dev", "/a", 1)]);
    assert_eq!(state.selected_space.as_deref(), Some("s1"));
    // No spaces at all: selection clears.
    state.apply_spaces(vec![]);
    assert_eq!(state.selected_space, None);
  }

  #[test]
  fn chats_in_space_filters_and_orders() {
    let mut state = AppState::new();
    state.apply_spaces(vec![space("s1", "dev", "/a", 1)]);
    let mut in_space_new = chat("new", 5, None);
    in_space_new.space_id = Some("s1".into());
    let mut in_space_old = chat("old", 1, Some(50)); // active but created first
    in_space_old.space_id = Some("s1".into());
    let mut other = chat("other", 2, None);
    other.space_id = Some("s2".into());
    let mut archived = chat("gone", 0, None);
    archived.space_id = Some("s1".into());
    archived.archived = true;
    let dangling = chat("dangling", 3, None); // no space id
    state.apply_chats(vec![in_space_new, in_space_old, other, archived, dangling]);
    let ids: Vec<&str> = state
      .chats_in_space("s1")
      .iter()
      .map(|c| c.id.as_str())
      .collect();
    assert_eq!(ids, ["old", "new"]);
    // The overview shows every live-space chat (idle included) PLUS
    // project-less chats (first-class since the project selectors);
    // chats of unknown spaces stay hidden. Completed ("old") outranks
    // idle ("new"/"dangling").
    let now = Utc::now();
    let overview: Vec<&str> = state
      .overview_chats(now)
      .iter()
      .map(|(_, c)| c.id.as_str())
      .collect();
    assert_eq!(overview, ["old", "new", "dangling"]);
  }

  #[test]
  fn apply_chats_drops_vanished_selection() {
    let mut state = AppState::new();
    state.apply_chats(vec![chat("a", 0, None), chat("b", 1, None)]);
    state.selected_chat = Some("a".into());
    state.transcript = vec![];
    state.apply_chats(vec![chat("b", 1, None)]);
    assert_eq!(state.selected_chat, None);
    // Still-present selection survives.
    state.selected_chat = Some("b".into());
    state.apply_chats(vec![chat("b", 1, None), chat("c", 2, None)]);
    assert_eq!(state.selected_chat.as_deref(), Some("b"));
  }

  #[test]
  fn apply_chat_config_stamps_the_row() {
    let mut state = AppState::new();
    state.apply_chats(vec![chat("a", 0, None), chat("b", 1, None)]);
    let config = helix_proto::ChatConfig {
      harness: HarnessId::ClaudeCode,
      model: Some("claude-fable-5".into()),
      reasoning: Some(helix_proto::ReasoningLevel::XHigh),
      model_options: serde_json::Map::new(),
      sandbox: helix_proto::SandboxLevel::WorkspaceWrite,
    };
    state.apply_chat_config("a", config.clone());
    assert_eq!(
      state.chats.iter().find(|c| c.id == "a").unwrap().config,
      Some(config)
    );
    assert!(
      state
        .chats
        .iter()
        .find(|c| c.id == "b")
        .unwrap()
        .config
        .is_none()
    );
    // Unknown chat: no-op, no panic.
    state.apply_chat_config(
      "missing",
      helix_proto::ChatConfig {
        harness: HarnessId::ClaudeCode,
        model: None,
        reasoning: None,
        model_options: serde_json::Map::new(),
        sandbox: helix_proto::SandboxLevel::WorkspaceWrite,
      },
    );
  }

  #[test]
  fn visible_chats_filters_archived() {
    let mut state = AppState::new();
    let mut archived = chat("a", 0, Some(99));
    archived.archived = true;
    state.apply_chats(vec![archived, chat("b", 1, None)]);
    let visible: Vec<&str> = state.visible_chats().map(|c| c.id.as_str()).collect();
    assert_eq!(visible, ["b"]);
  }

  #[test]
  fn echoes_show_until_doc_frame_confirms() {
    let mut state = AppState::new();
    state.selected_chat = Some("c1".into());
    let echo = SessionMessageEntry {
      id: "m1".into(),
      role: helix_doc::MessageRole::User,
      parts: vec![],
      created_at: 0,
      device_id: "local".into(),
      status: None,
      continuation_of: None,
    };
    state.push_echo("c1", echo.clone());
    // Duplicate pushes dedupe.
    state.push_echo("c1", echo.clone());
    assert_eq!(state.pending_echoes().len(), 1);
    // Frames without the id keep the echo.
    state.apply_transcript(vec![]);
    assert_eq!(state.pending_echoes().len(), 1);
    // The confirming frame prunes it.
    state.apply_transcript(vec![SessionMessageEntry {
      id: "m1".into(),
      ..echo.clone()
    }]);
    assert!(state.pending_echoes().is_empty());
    // Failure path: explicit removal.
    state.push_echo(
      "c1",
      SessionMessageEntry {
        id: "m2".into(),
        ..echo.clone()
      },
    );
    state.remove_echo("c1", "m2");
    assert!(state.pending_echoes().is_empty());
    // Echoes are per chat.
    state.push_echo(
      "other",
      SessionMessageEntry {
        id: "m3".into(),
        ..echo
      },
    );
    assert!(state.pending_echoes().is_empty());
  }

  fn chat_with_cwd(id: &str, created_min: i64, cwd: Option<&str>) -> Chat {
    let mut c = chat(id, created_min, None);
    c.cwd = cwd.map(str::to_string);
    c
  }

  #[test]
  fn project_labels_from_cwd() {
    assert_eq!(project_label(Some("/home/w/dev/helix")), "helix");
    assert_eq!(project_label(Some("/home/w/dev/helix/")), "helix");
    assert_eq!(project_label(None), "No project");
    assert_eq!(project_label(Some("   ")), "No project");
    assert_eq!(project_label(Some("/")), "/");
  }

  #[test]
  fn grouped_sidebar_preserves_recency_order() {
    // Input is sidebar-sorted (most recent first).
    let chats = [
      chat_with_cwd("a", 9, Some("/dev/helix")),
      chat_with_cwd("b", 8, Some("/dev/zed")),
      chat_with_cwd("c", 7, Some("/dev/helix")),
      chat_with_cwd("d", 6, None),
    ];
    let groups = group_chats(chats.iter());
    let labels: Vec<&str> = groups.iter().map(|g| g.label.as_str()).collect();
    // Groups ordered by their most recent chat; rows keep order.
    assert_eq!(labels, ["helix", "zed", "No project"]);
    let helix_ids: Vec<&str> = groups[0].chats.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(helix_ids, ["a", "c"]);
    assert!(group_chats(std::iter::empty()).is_empty());
  }

  #[test]
  fn relative_times_match_helix_format() {
    let now = Utc::now();
    let ago = |secs: i64| now - chrono::Duration::seconds(secs);
    assert_eq!(format_time_ago(ago(0), now), "now");
    assert_eq!(format_time_ago(ago(59), now), "now");
    assert_eq!(format_time_ago(ago(60), now), "1m");
    assert_eq!(format_time_ago(ago(59 * 60), now), "59m");
    assert_eq!(format_time_ago(ago(60 * 60), now), "1h");
    assert_eq!(format_time_ago(ago(23 * 3600 + 3599), now), "23h");
    assert_eq!(format_time_ago(ago(24 * 3600), now), "1d");
    assert_eq!(format_time_ago(ago(6 * 86400), now), "6d");
    assert_eq!(format_time_ago(ago(7 * 86400), now), "1w");
    assert_eq!(format_time_ago(ago(30 * 86400), now), "4w");
    assert_eq!(format_time_ago(ago(35 * 86400), now), "1mo");
    assert_eq!(format_time_ago(ago(400 * 86400), now), "1y");
    // Clock skew (future timestamps) clamps to "now".
    assert_eq!(
      format_time_ago(now + chrono::Duration::hours(2), now),
      "now"
    );
  }

  #[test]
  fn chat_location_joins_project_and_branch() {
    let mut c = chat_with_cwd("x", 1, Some("/home/w/dev/soccertcg"));
    c.branch = Some("helix/rebalance".into());
    assert_eq!(
      chat_location(&c).as_deref(),
      Some("soccertcg · helix/rebalance")
    );
    c.branch = None;
    assert_eq!(chat_location(&c).as_deref(), Some("soccertcg"));
    c.cwd = None;
    c.branch = Some("main".into());
    assert_eq!(chat_location(&c).as_deref(), Some("main"));
    c.branch = Some("   ".into());
    assert_eq!(chat_location(&c), None);
    c.branch = None;
    assert_eq!(chat_location(&c), None);
  }
}
