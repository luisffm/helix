//! DocHost — per-chat `SessionDoc` handles: snapshot persistence (debounced) and
//! the durable command executor.
//!
//! Pragmatic port of helix's `session-docs.ts` + the `main.ts` executor (spec:
//! feature-inventory §3.3, ARCHITECTURE §2 "command plane"):
//! - the doc IS the outbox: commands and user entries commit locally;
//! - on every doc change the handle re-emits the joined transcript to watchers,
//!   drains pending commands, and schedules a snapshot save;
//! - command drain: evaluate via `evaluate_command` (with the DocsStore processed
//!   ledger), mark processed BEFORE execute, execute through the sessions engine, then
//!   write the outcome status back into the doc as the sole outcome writer.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use helix_doc::{
  COMMAND_DEFAULT_TTL_MS, CommandBasedOn, CommandDisposition, DocError, EvaluationContext,
  MessagePart, MessageRole, MessageStatus, SessionCommandEntry, SessionCommandPayload,
  SessionCommandStatus, SessionDoc, SessionMessageEntry, evaluate_command,
  join_continuation_entries,
};
use helix_proto::{HarnessId, UserInputAnswer, UserInputQuestion};
use helix_store::DocsStore;

use crate::sessions::{SessionsEngine, SteerOutcome};
use crate::workspace_host::WorkspaceHost;
use crate::{EngineError, new_id, now_ms};

/// Debounce window for local snapshot saves after a doc change.
const SNAPSHOT_DEBOUNCE_MS: u64 = 1_000;

/// Warm-doc LRU: how many unwatched, run-less docs stay fully open. Everything
/// beyond this (and beyond [`helix_doc::DOC_LRU_BYTE_BUDGET`]) is evicted
/// oldest-access-first — reopening from the SQLite snapshot measured within
/// ~11ms of a warm doc, so the cap trades no perceptible open latency.
const WARM_DOC_CAP: usize = 12;

/// Resident-memory estimate per compressed snapshot byte. Loro snapshots are
/// columnar+compressed; the in-memory doc plus mirror runs well above the blob
/// size. A rough multiplier is enough here — the budget is a safety ceiling,
/// the count cap does the day-to-day work.
const RESIDENT_BYTES_PER_SNAPSHOT_BYTE: usize = 6;

/// Floor per open doc (room socket buffers, tasks) regardless of content size.
const DOC_RESIDENT_FLOOR_BYTES: usize = 512 * 1024;

/// Docs touched this recently are never evicted. Closes the open→attach race:
/// `open()` returns a handle, and until the caller's `watch_messages` lands
/// the doc is unwatched and unpinned — a concurrent eviction would orphan the
/// watcher on a roomless doc that renders once and never updates again.
const EVICT_MIN_IDLE_MS: i64 = 30_000;

#[derive(Debug, Clone)]
pub struct DocHostConfig {
  pub device_id: String,
  /// Harness for doc-command runs on chats without a workspace `config` row.
  pub default_harness: HarnessId,
}

struct DocHostInner {
  store: Arc<DocsStore>,
  config: DocHostConfig,
  /// Set-once (first wins), cleared by `shutdown_workers`: sessions and
  /// doc-host reference each other through Arcs, so a retired runtime's
  /// graph only drops once this back-edge is severed.
  sessions: Mutex<Option<SessionsEngine>>,
  workspace: OnceLock<WorkspaceHost>,
  /// Cancels every worker spawned through `spawn_worker` — the loops'
  /// own exit conditions (weak handle death, closed channels) don't cover
  /// runtime replacement, where a retired host's tasks must stop even
  /// while something still pins the graph.
  shutdown: CancellationToken,
  /// Tracks every spawned worker so `shutdown_workers` can await them.
  tasks: TaskTracker,
  handles: Mutex<HashMap<String, Arc<ChatDocHandle>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
  mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone)]
pub struct DocHost {
  inner: Arc<DocHostInner>,
}

/// One open chat doc: the `SessionDoc` and its change plumbing.
pub struct ChatDocHandle {
  chat_id: String,
  device_id: String,
  doc: Arc<SessionDoc>,
  messages_tx: watch::Sender<Vec<SessionMessageEntry>>,
  /// True when the doc changed while nobody watched: the mirror rebuild is
  /// deferred to the next `watch_messages` attach instead of paid per commit.
  mirror_dirty: AtomicBool,
  /// Epoch ms of the last open/watch touch — the LRU eviction key.
  last_access: AtomicI64,
  /// Last known snapshot blob size — the eviction budget estimate's input.
  snapshot_bytes: AtomicUsize,
  /// Doc subscription (drop = unsubscribe) — bumps the change watch on every commit.
  _sub: loro::Subscription,
}

impl ChatDocHandle {
  pub fn chat_id(&self) -> &str {
    &self.chat_id
  }

  pub fn doc(&self) -> &SessionDoc {
    &self.doc
  }

  pub fn doc_arc(&self) -> Arc<SessionDoc> {
    self.doc.clone()
  }

  /// Joined transcript watch — re-sent on every doc change (WatchDocMessages).
  ///
  /// Attach-time refresh: the mirror is only maintained while watched, so a
  /// doc that changed unwatched materializes here, once, instead of on every
  /// commit it sat through in the background.
  pub fn watch_messages(&self) -> watch::Receiver<Vec<SessionMessageEntry>> {
    self.touch();
    // Subscribe BEFORE the dirty check: a commit racing this attach then
    // sees a live receiver and publishes, instead of re-marking dirty
    // after our refresh and leaving the new watcher a cleared mirror.
    let rx = self.messages_tx.subscribe();
    if self.mirror_dirty.load(Ordering::Acquire) {
      self.publish_messages();
    }
    rx
  }

  fn touch(&self) {
    self.last_access.store(now_ms(), Ordering::Relaxed);
  }

  /// Write a complete user message entry, idempotent by id (the client-minted message
  /// id — a re-executed command or optimistic echo never duplicates the entry).
  pub fn write_user_message(
    &self,
    message_id: &str,
    text: &str,
    created_at: i64,
  ) -> Result<(), DocError> {
    if self.doc.read_entries()?.iter().any(|e| e.id == message_id) {
      return Ok(());
    }
    self.doc.push_message(&SessionMessageEntry {
      id: message_id.to_string(),
      role: MessageRole::User,
      parts: vec![MessagePart::Text {
        id: "t0".into(),
        text: text.to_string(),
      }],
      created_at,
      device_id: self.device_id.clone(),
      status: Some(MessageStatus::Complete),
      continuation_of: None,
    })
  }

  /// Recovery sweep: stamp this device's abandoned `streaming` entries `aborted`, appending
  /// `note` as a visible error part so the transcript says WHY the turn
  /// ended (helix folded "Run interrupted by backend restart" the same
  /// way). Returns the stamped entries' `(id, created_at)` — recovery uses
  /// them for the resume-freshness check.
  pub fn mark_abandoned_streams(&self, note: &str) -> Result<Vec<(String, i64)>, DocError> {
    let mut stamped = Vec::new();
    for entry in self.doc.read_entries()? {
      if entry.role == MessageRole::Assistant
        && entry.status == Some(MessageStatus::Streaming)
        && entry.device_id == self.device_id
        && self
          .doc
          .set_message_status(&entry.id, MessageStatus::Aborted)?
      {
        let part_id = format!("{}-recovery", entry.id);
        if let Err(err) = self.doc.append_error_part(&entry.id, &part_id, note) {
          tracing::warn!(chat = %self.chat_id, error = %err, "recovery note append failed");
        }
        stamped.push((entry.id.clone(), entry.created_at));
      }
    }
    if !stamped.is_empty() {
      self.publish_messages();
    }
    Ok(stamped)
  }

  fn publish_messages(&self) {
    self.mirror_dirty.store(false, Ordering::Release);
    match self.doc.read_entries() {
      Ok(entries) => {
        let joined = join_continuation_entries(entries);
        // send_replace: update the watch even with no subscribers yet, so a
        // late subscriber's first borrow sees the current transcript.
        self.messages_tx.send_replace(joined);
      }
      Err(err) => {
        tracing::warn!(chat = %self.chat_id, error = %err, "transcript read failed");
      }
    }
  }

  /// Per-commit publish path: unwatched docs just mark the mirror dirty —
  /// rebuilding a full transcript nobody reads was a per-tick cost on every
  /// open doc (and kept a second transcript copy hot).
  fn publish_messages_if_watched(&self) {
    if self.messages_tx.receiver_count() == 0 {
      self.mirror_dirty.store(true, Ordering::Release);
      // Shrink the stale mirror: watch_messages rebuilds on attach.
      self.messages_tx.send_replace(Vec::new());
    } else {
      self.publish_messages();
    }
  }

  /// Rough resident cost for the LRU budget.
  fn resident_estimate(&self) -> usize {
    (self.snapshot_bytes.load(Ordering::Relaxed) * RESIDENT_BYTES_PER_SNAPSHOT_BYTE)
      .max(DOC_RESIDENT_FLOOR_BYTES)
  }
}

impl DocHost {
  pub fn new(store: Arc<DocsStore>, config: DocHostConfig) -> Self {
    Self {
      inner: Arc::new(DocHostInner {
        store,
        config,
        sessions: Mutex::new(None),
        workspace: OnceLock::new(),
        shutdown: CancellationToken::new(),
        tasks: TaskTracker::new(),
        handles: Mutex::new(HashMap::new()),
      }),
    }
  }

  /// Every background task rides the tracker, raced against the shutdown
  /// token: the loops' own exits stay authoritative in normal operation;
  /// the token is the retirement override.
  fn spawn_worker(&self, fut: impl std::future::Future<Output = ()> + Send + 'static) {
    let cancel = self.inner.shutdown.clone();
    self.inner.tasks.spawn(async move {
      tokio::select! {
          _ = cancel.cancelled() => {}
          _ = fut => {}
      }
    });
  }

  /// The sessions engine, once wired. `None` before assembly or after
  /// `shutdown_workers` — callers treat both as "executor unavailable".
  fn sessions(&self) -> Option<SessionsEngine> {
    lock(&self.inner.sessions).clone()
  }

  /// Wire the sessions engine (engine assembly; see `SessionsEngine::set_doc_host`).
  pub fn set_sessions(&self, sessions: SessionsEngine) {
    {
      // First set wins (the OnceLock contract this slot replaced).
      let mut slot = lock(&self.inner.sessions);
      if slot.is_none() {
        *slot = Some(sessions);
      }
    }
    // Commands may already be pending in warm-opened docs.
    let handles: Vec<_> = lock(&self.inner.handles).values().cloned().collect();
    for handle in handles {
      let host = self.clone();
      self.spawn_worker(async move { host.drain_commands(&handle).await });
    }
  }

  /// Retire this host's workers (runtime replacement): cancel
  /// and await every spawned task, drop every open chat handle (ending the
  /// weak-keyed room/join loops and watcher streams), and sever the sessions
  /// back-edge so the replaced engine graph can actually drop. Idempotent.
  pub async fn shutdown_workers(&self) {
    self.inner.shutdown.cancel();
    self.inner.tasks.close();
    self.inner.tasks.wait().await;
    // Snapshot open docs BEFORE releasing their handles: the handles map
    // holds the only strong doc refs, and an unflushed doc dies with it.
    self.flush_all();
    // Take the map under the lock, drop the handles outside it.
    let handles = std::mem::take(&mut *lock(&self.inner.handles));
    drop(handles);
    lock(&self.inner.sessions).take();
  }

  /// Test-only retirement sentinel: reports true once the doc-host graph
  /// has actually been freed.
  #[doc(hidden)]
  pub fn retirement_probe(&self) -> Box<dyn Fn() -> bool + Send + Sync> {
    let weak = Arc::downgrade(&self.inner);
    Box::new(move || weak.upgrade().is_none())
  }

  /// Wire the workspace host (engine assembly) — the source of chat-ownership rows.
  pub fn set_workspace(&self, workspace: WorkspaceHost) {
    let _ = self.inner.workspace.set(workspace);
  }

  /// The workspace host, once wired (tests may assemble a DocHost without one).
  pub fn workspace(&self) -> Option<&WorkspaceHost> {
    self.inner.workspace.get()
  }

  pub fn device_id(&self) -> &str {
    &self.inner.config.device_id
  }

  /// Open (or return) the chat's doc handle: load the local snapshot (or init
  /// fresh) and start the change-driven task.
  pub fn open(&self, chat_id: &str) -> Result<Arc<ChatDocHandle>, EngineError> {
    {
      let handles = lock(&self.inner.handles);
      if let Some(handle) = handles.get(chat_id) {
        handle.touch();
        return Ok(handle.clone());
      }
    }
    let stored = self.inner.store.load_snapshot(chat_id)?;
    let mut snapshot_len = 0usize;
    let doc = match stored {
      Some(bytes) => {
        snapshot_len = bytes.len();
        let raw = loro::LoroDoc::new();
        raw
          .import(&bytes)
          .map_err(|e| EngineError::Other(format!("snapshot import failed: {e}")))?;
        SessionDoc::from_doc(raw)
      }
      None => SessionDoc::init(chat_id)?,
    };
    let doc = Arc::new(doc);

    let (changed_tx, changed_rx) = watch::channel(0u64);
    let sub = doc.doc().subscribe_root(Arc::new(move |_diff| {
      changed_tx.send_modify(|v| *v = v.wrapping_add(1));
    }));
    // The mirror starts dirty and empty: many opens (command queueing,
    // drains) never watch the transcript, and the first watch_messages
    // attach materializes it on demand.
    let (messages_tx, _) = watch::channel(Vec::new());

    let handle = Arc::new(ChatDocHandle {
      chat_id: chat_id.to_string(),
      device_id: self.inner.config.device_id.clone(),
      doc,
      messages_tx,
      mirror_dirty: AtomicBool::new(true),
      last_access: AtomicI64::new(now_ms()),
      snapshot_bytes: AtomicUsize::new(snapshot_len),
      _sub: sub,
    });
    {
      let mut handles = lock(&self.inner.handles);
      if let Some(existing) = handles.get(chat_id) {
        return Ok(existing.clone()); // racing open — keep the first
      }
      handles.insert(chat_id.to_string(), handle.clone());
    }

    self.spawn_worker(chat_task(self.clone(), Arc::downgrade(&handle), changed_rx));
    self.evict_over_budget();
    Ok(handle)
  }

  /// LRU eviction: while the warm set exceeds [`WARM_DOC_CAP`] or the
  /// resident estimate exceeds `DOC_LRU_BYTE_BUDGET`, close the
  /// least-recently-touched unpinned docs. Pinned (never evicted):
  /// - watched docs (`messages_tx` has receivers — a UI transcript);
  /// - docs with a live writer (`Arc<SessionDoc>` held outside the handle —
  ///   a run streaming into it);
  /// - host-side docs with pending commands (the executor owes them work).
  ///
  /// Eviction flushes a final snapshot, so reopen loses nothing; missed
  /// remote updates re-arrive through the room join's VV backfill.
  fn evict_over_budget(&self) {
    let mut by_age: Vec<(i64, String)> = {
      let handles = lock(&self.inner.handles);
      handles
        .values()
        .map(|h| (h.last_access.load(Ordering::Relaxed), h.chat_id.clone()))
        .collect()
    };
    by_age.sort_unstable();
    for (last_access, chat_id) in by_age {
      if now_ms() - last_access < EVICT_MIN_IDLE_MS {
        // Sorted oldest-first: everything after this is younger.
        return;
      }
      let (count, estimate) = {
        let handles = lock(&self.inner.handles);
        (
          handles.len(),
          handles
            .values()
            .map(|h| h.resident_estimate())
            .sum::<usize>(),
        )
      };
      if count <= WARM_DOC_CAP && estimate <= helix_doc::DOC_LRU_BYTE_BUDGET {
        return;
      }
      let evicted = {
        let mut handles = lock(&self.inner.handles);
        match handles.get(&chat_id) {
          Some(handle) if !self.pinned(handle) => handles.remove(&chat_id),
          _ => None,
        }
      };
      if let Some(handle) = evicted {
        // Final flush outside the map lock; ≤1s of changes could be
        // pending in the snapshot debounce.
        self.save_snapshot(&handle);
        tracing::debug!(chat = %handle.chat_id, "doc evicted (LRU)");
      }
    }
  }

  fn pinned(&self, handle: &Arc<ChatDocHandle>) -> bool {
    if handle.messages_tx.receiver_count() > 0 {
      return true;
    }
    // The handle itself holds one doc ref; more means a live writer.
    if Arc::strong_count(&handle.doc) > 1 {
      return true;
    }
    if self.is_host(&handle.chat_id) {
      let is_processed = |id: &str| self.inner.store.is_processed(id).unwrap_or(false);
      match handle.doc.read_commands() {
        Ok(commands) => commands
          .iter()
          .any(|c| c.status == SessionCommandStatus::Pending && !is_processed(&c.id)),
        // Unreadable ledger: keep the doc, never evict blind.
        Err(_) => true,
      }
    } else {
      false
    }
  }

  /// Drop a chat's doc unconditionally and delete its local snapshot — the
  /// chat is gone (DeleteChat / DeleteSpace cascade). Watchers see the
  /// stream end; a racing writer keeps its orphaned doc until the run ends.
  pub fn purge_chat(&self, chat_id: &str) {
    let removed = lock(&self.inner.handles).remove(chat_id);
    drop(removed);
    if let Err(err) = self.inner.store.delete_snapshot(chat_id) {
      tracing::warn!(chat = %chat_id, error = %err, "snapshot delete failed");
    }
  }

  /// Composer path: append an immutable pending command entry (rule 1). Durable by
  /// construction — the change subscription kicks the drain, so a local host executes
  /// immediately and an offline doc simply holds the entry until it syncs.
  pub fn queue_command(
    &self,
    chat_id: &str,
    payload: SessionCommandPayload,
  ) -> Result<String, EngineError> {
    let handle = self.open(chat_id)?;
    let id = new_id();
    let now = now_ms();
    let based_on = handle.doc.read_entries()?.last().map(|m| CommandBasedOn {
      turn_id: Some(m.id.clone()),
      frontier: None,
    });
    let is_message = matches!(
      payload,
      SessionCommandPayload::Run { .. } | SessionCommandPayload::Steer { .. }
    );
    handle.doc.queue_command(&SessionCommandEntry {
      id: id.clone(),
      payload,
      issued_by: self.inner.config.device_id.clone(),
      issued_at: now,
      based_on,
      expires_at: Some(now + COMMAND_DEFAULT_TTL_MS),
      status: SessionCommandStatus::Pending,
      resolution: None,
    })?;
    // Sending a message revives an archived chat: the user is acting in it
    // again, so the LWW row flips back to active on every device. Best-
    // effort — the command itself is durable regardless.
    if is_message {
      if let Some(workspace) = self.workspace() {
        match workspace.chat(chat_id) {
          Ok(Some(chat)) if chat.archived => {
            if let Err(err) = workspace.set_chat_archived(chat_id, false) {
              tracing::warn!(chat = %chat_id, error = %err, "unarchive on send failed");
            }
          }
          _ => {}
        }
      }
    }
    Ok(id)
  }

  /// §2.2 writer discipline: we host a chat iff its workspace row's `deviceId` is
  /// ours; a chat with no row is claimable (claim-on-first-command). Without a
  /// wired workspace host (bare-DocHost tests) every open chat is ours — M2's
  /// behavior, now the degenerate case.
  fn is_host(&self, chat_id: &str) -> bool {
    self.workspace().is_none_or(|ws| ws.is_host(chat_id))
  }

  /// Chat-config harness when the workspace row carries one, else the default.
  pub(crate) fn harness_for(&self, chat_id: &str) -> HarnessId {
    self
      .workspace()
      .and_then(|ws| ws.chat_config(chat_id))
      .map(|config| config.harness)
      .unwrap_or(self.inner.config.default_harness)
  }

  /// The harness a request dispatches on: the request's own pick when it
  /// carries one (rides the command plane, immune to registry-row races),
  /// else [`Self::harness_for`].
  pub(crate) fn harness_for_request(
    &self,
    chat_id: &str,
    request: &helix_proto::RunRequest,
  ) -> HarnessId {
    request.harness.unwrap_or_else(|| self.harness_for(chat_id))
  }

  /// Drain pending commands (host-only): evaluate → mark processed BEFORE execute →
  /// execute → write the outcome as the sole outcome writer.
  pub async fn drain_commands(&self, handle: &Arc<ChatDocHandle>) {
    let Some(sessions) = self.sessions() else {
      return; // executor not wired yet (or retired); the set_sessions kick re-drains
    };
    if !self.is_host(&handle.chat_id) {
      return;
    }
    // Entries this pass decided to leave alone (processed dedupe hits).
    let mut skipped: HashSet<String> = HashSet::new();
    loop {
      let commands = match handle.doc.read_commands() {
        Ok(commands) => commands,
        Err(err) => {
          tracing::warn!(chat = %handle.chat_id, error = %err, "command read failed");
          return;
        }
      };
      let is_processed = |id: &str| self.inner.store.is_processed(id).unwrap_or(false);
      let Some(entry) = commands
        .iter()
        .find(|c| {
          c.status == SessionCommandStatus::Pending
            && !skipped.contains(&c.id)
            && !is_processed(&c.id)
        })
        .cloned()
      else {
        return;
      };
      let messages = handle.doc.read_entries().unwrap_or_default();
      let current_turn_id = messages.last().map(|m| m.id.clone());
      let turn_is_past = |turn_id: &str| messages.iter().any(|m| m.id == turn_id);
      let disposition = evaluate_command(
        &entry,
        &EvaluationContext {
          is_processed: &is_processed,
          now_ms: now_ms(),
          entries: &commands,
          current_turn_id: current_turn_id.as_deref(),
          turn_is_past: &turn_is_past,
        },
      );
      // Mark BEFORE executing: a crash mid-execution must never double-run a
      // command whose side effect may already have happened.
      if let Err(err) = self.inner.store.mark_processed(&entry.id) {
        tracing::error!(chat = %handle.chat_id, error = %err, "processed-ledger write failed; halting drain");
        return;
      }
      match disposition {
        CommandDisposition::Skip => {
          skipped.insert(entry.id.clone());
        }
        CommandDisposition::Expired => {
          self.resolve_command(handle, &entry.id, SessionCommandStatus::Expired, None);
        }
        CommandDisposition::Superseded => {
          self.resolve_command(handle, &entry.id, SessionCommandStatus::Superseded, None);
        }
        CommandDisposition::Execute => {
          let (status, resolution) = match self.execute(&sessions, handle, &entry).await {
            Ok(outcome) => outcome,
            Err(err) => (SessionCommandStatus::Rejected, Some(err.to_string())),
          };
          self.resolve_command(handle, &entry.id, status, resolution.as_deref());
        }
      }
    }
  }

  /// Host-only outcome write (ledger rule 2).
  fn resolve_command(
    &self,
    handle: &ChatDocHandle,
    command_id: &str,
    status: SessionCommandStatus,
    resolution: Option<&str>,
  ) {
    if let Err(err) = handle
      .doc
      .set_command_status(command_id, status, resolution)
    {
      tracing::warn!(
          chat = %handle.chat_id,
          command = %command_id,
          error = %err,
          "command outcome write failed"
      );
    }
  }

  async fn execute(
    &self,
    sessions: &SessionsEngine,
    handle: &Arc<ChatDocHandle>,
    entry: &SessionCommandEntry,
  ) -> Result<(SessionCommandStatus, Option<String>), EngineError> {
    let chat_id = &handle.chat_id;
    match &entry.payload {
      SessionCommandPayload::Run {
        request,
        message_id,
      } => {
        // Claim-on-first-command: a run for a chat with no workspace row
        // creates the row under our device id (we are about to host it).
        if let Some(ws) = self.workspace() {
          ws.claim_chat(chat_id, Some(&request.cwd))?;
        }
        let harness = self.harness_for_request(chat_id, request);
        // A row with no config renders no harness glyph (and every
        // later dispatch falls back to the engine default), so stamp
        // what this run actually executes with. Claimed rows and
        // catalog-not-loaded createChats both land here; the racing
        // real createChat carries the same picked values.
        if let Some(ws) = self.workspace()
          && ws.chat_config(chat_id).is_none()
        {
          let config = helix_proto::ChatConfig {
            harness,
            model: request.model.clone(),
            reasoning: request.reasoning,
            model_options: request.model_options.clone(),
            sandbox: request.sandbox,
          };
          if let Err(err) = ws.set_chat_config(chat_id, &config) {
            tracing::warn!(chat = %chat_id, error = %err, "run-config backfill failed");
          }
        }
        sessions
          .dispatch(chat_id, harness, request.clone(), Some(message_id.clone()))
          .await?;
        Ok((SessionCommandStatus::Applied, None))
      }
      SessionCommandPayload::Steer { prompt, message_id } => {
        match sessions.steer(chat_id, prompt, message_id.clone()).await? {
          SteerOutcome::Accepted => Ok((SessionCommandStatus::Applied, None)),
          SteerOutcome::NotSteerable => {
            // No live steerable run: the durable command still delivers —
            // run it as the next turn (helix's fallback, executor-side).
            // After an engine restart `last_request` is empty too, so
            // rebuild the run config from the chat's workspace row
            // (helix derived dispatch config from the chat row the
            // same way — sessions.ts:601-620); dispatch's engine-owned
            // resume then reattaches the prior harness conversation.
            let request = sessions
              .last_request(chat_id)
              .or_else(|| self.request_from_chat_row(chat_id, prompt));
            let Some(mut request) = request else {
              return Ok((
                SessionCommandStatus::Rejected,
                Some("no live run and no prior run config".into()),
              ));
            };
            request.prompt = prompt.clone();
            request.resume = None; // dispatch re-derives the harness session
            // A reused config must not re-inline the PREVIOUS
            // turn's images; this steer's own refs (if any) already
            // ride the prompt text.
            request.attachments = Vec::new();
            let harness = self.harness_for_request(chat_id, &request);
            sessions
              .dispatch(chat_id, harness, request, message_id.clone())
              .await?;
            Ok((
              SessionCommandStatus::Applied,
              Some("queued as new turn".into()),
            ))
          }
        }
      }
      SessionCommandPayload::Interrupt {} => {
        sessions.interrupt(chat_id).await?;
        Ok((SessionCommandStatus::Applied, None))
      }
      SessionCommandPayload::RespondInput {
        request_id,
        answers,
      } => {
        if sessions.respond_input(chat_id, request_id, answers.clone())? {
          return Ok((SessionCommandStatus::Applied, None));
        }
        // No live resolver. Only a request id the doc shows as an
        // OPEN question on a SETTLED entry gets the orphan fallback:
        // a mismatched or already-resolved id is a stale/buggy answer
        // and must still reject, and a still-streaming entry's
        // question belongs to the live run (a just-consumed resolver
        // racing a second answer must not spawn a duplicate turn).
        let questions = handle.doc.read_entries().ok().and_then(|entries| {
          entries
            .iter()
            .rev()
            .filter(|e| e.status != Some(MessageStatus::Streaming))
            .find_map(|e| {
              e.parts.iter().find_map(|p| match p {
                MessagePart::Input {
                  request_id: rid,
                  questions,
                  resolved: false,
                  ..
                } if rid == request_id => Some(questions.clone()),
                _ => None,
              })
            })
        });
        let Some(questions) = questions else {
          return Ok((
            SessionCommandStatus::Rejected,
            Some("no pending input request".into()),
          ));
        };
        // The run died under the question (engine restart, crash).
        // The question is still open in the doc and the command is
        // durable, so honor it anyway — stamp the part resolved and
        // deliver the answers as the next (resumed) turn, the same
        // fallback a dead-run steer takes. The question UI stays up
        // until the user answers (user requirement); this is what
        // makes that answer still WORK.
        let request = sessions
          .last_request(chat_id)
          .or_else(|| self.request_from_chat_row(chat_id, ""));
        let Some(mut request) = request else {
          return Ok((
            SessionCommandStatus::Rejected,
            Some("no pending input request and no prior run config".into()),
          ));
        };
        request.prompt = respond_input_prompt(&questions, answers);
        request.resume = None; // dispatch re-derives the harness session
        request.attachments = Vec::new();
        if let Err(err) = handle.doc.resolve_input(request_id) {
          tracing::warn!(chat = %chat_id, request = %request_id, error = %err,
                        "orphaned input resolve failed");
        }
        let harness = self.harness_for_request(chat_id, &request);
        sessions.dispatch(chat_id, harness, request, None).await?;
        Ok((
          SessionCommandStatus::Applied,
          Some("answered as new turn".into()),
        ))
      }
    }
  }

  /// A steer-turned-run with no in-process `last_request` (engine restarted
  /// since the last turn): rebuild the run config from the chat's workspace
  /// row — cwd from the row, model/reasoning/options/sandbox from its config
  /// (composer defaults otherwise). `None` without a workspace host or row.
  // (Also the RespondInput dead-run fallback's config source.)
  pub(crate) fn request_from_chat_row(
    &self,
    chat_id: &str,
    prompt: &str,
  ) -> Option<helix_proto::RunRequest> {
    let workspace = self.workspace()?;
    let chat = match workspace.chat(chat_id) {
      Ok(chat) => chat?,
      Err(err) => {
        tracing::warn!(chat = %chat_id, error = %err, "workspace chat read failed");
        return None;
      }
    };
    let config = chat.config;
    Some(helix_proto::RunRequest {
      prompt: prompt.to_string(),
      harness: config.as_ref().map(|c| c.harness),
      model: config.as_ref().and_then(|c| c.model.clone()),
      reasoning: config.as_ref().and_then(|c| c.reasoning),
      model_options: config
        .as_ref()
        .map(|c| c.model_options.clone())
        .unwrap_or_default(),
      cwd: chat.cwd.unwrap_or_default(),
      sandbox: config
        .as_ref()
        .map(|c| c.sandbox)
        .unwrap_or(helix_proto::SandboxLevel::WorkspaceWrite),
      auto_approve: false,
      attachments: Vec::new(),
      resume: None,
    })
  }

  fn save_snapshot(&self, handle: &ChatDocHandle) {
    match handle.doc.export_snapshot() {
      Ok(bytes) => {
        handle.snapshot_bytes.store(bytes.len(), Ordering::Relaxed);
        if let Err(err) = self.inner.store.save_snapshot(&handle.chat_id, &bytes) {
          tracing::warn!(chat = %handle.chat_id, error = %err, "snapshot save failed");
        }
      }
      Err(err) => {
        tracing::warn!(chat = %handle.chat_id, error = %err, "snapshot export failed");
      }
    }
  }

  /// Persist every open doc now (shutdown path; bypasses the debounce).
  pub fn flush_all(&self) {
    let handles: Vec<_> = lock(&self.inner.handles).values().cloned().collect();
    for handle in handles {
      self.save_snapshot(&handle);
    }
  }
}

/// The resumed-turn prompt for answers to a question whose run died: each
/// answer paired with its question text so the reattached conversation reads
/// naturally. Pure.
pub fn respond_input_prompt(
  questions: &[UserInputQuestion],
  answers: &[UserInputAnswer],
) -> String {
  let mut lines = vec!["Answering your earlier question:".to_string()];
  for answer in answers {
    let picked = answer.labels.join(", ");
    let question = questions
      .iter()
      .find(|q| q.id == answer.question_id)
      .map(|q| q.question.trim())
      .filter(|q| !q.is_empty());
    match question {
      Some(question) => lines.push(format!("{question} — {picked}")),
      None => lines.push(picked),
    }
  }
  lines.join("\n")
}

/// Per-chat background task: reacts to doc changes (local commits and remote imports)
/// by re-publishing the transcript watch, draining commands, and debouncing snapshots.
/// Holds only a weak handle so a dropped host tears the task down.
async fn chat_task(host: DocHost, weak: Weak<ChatDocHandle>, mut changed_rx: watch::Receiver<u64>) {
  // Initial pass: the snapshot may already carry pending commands. The
  // mirror stays lazy — it materializes on the first watch attach.
  {
    let Some(handle) = weak.upgrade() else { return };
    host.drain_commands(&handle).await;
  }
  let mut save_deadline: Option<tokio::time::Instant> = None;
  loop {
    let sleep_until = save_deadline.unwrap_or_else(tokio::time::Instant::now);
    tokio::select! {
        changed = changed_rx.changed() => {
            if changed.is_err() {
                break; // doc handle (and its change sender) is gone
            }
            let Some(handle) = weak.upgrade() else { break };
            handle.publish_messages_if_watched();
            host.drain_commands(&handle).await;
            if save_deadline.is_none() {
                save_deadline = Some(
                    tokio::time::Instant::now()
                        + std::time::Duration::from_millis(SNAPSHOT_DEBOUNCE_MS),
                );
            }
        }
        _ = tokio::time::sleep_until(sleep_until), if save_deadline.is_some() => {
            save_deadline = None;
            let Some(handle) = weak.upgrade() else { break };
            host.save_snapshot(&handle);
            // Post-quiesce eviction pass: sizes just refreshed.
            host.evict_over_budget();
        }
    }
  }
}
