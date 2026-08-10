use helix_models::SessionId;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum HelixEvent {
  FsBatch(Vec<PathBuf>),
  SessionOpened(SessionId),
  SessionClosed(SessionId),
  SessionExited { id: SessionId, code: i32 },
  SessionRetitled { id: SessionId, title: String },
}

pub type EventTx = tokio::sync::mpsc::UnboundedSender<HelixEvent>;
pub type EventRx = tokio::sync::mpsc::UnboundedReceiver<HelixEvent>;

pub fn channel() -> (EventTx, EventRx) {
  tokio::sync::mpsc::unbounded_channel()
}
