pub mod cache;
pub mod config;
pub mod terminal_font;

use helix_models::{AgentStatus, TitleStatus};
use std::time::{Duration, Instant};

const RUNNING_WINDOW: Duration = Duration::from_secs(2);

/// An agent that states its own status in the title is believed, since a TUI
/// redrawing itself looks exactly like work from the pty's side. Only a session
/// that says nothing falls back to recent output.
pub fn activity_status(
  last_activity: Instant,
  exited: Option<i32>,
  title: Option<TitleStatus>,
) -> AgentStatus {
  match exited {
    Some(0) => AgentStatus::Finished,
    Some(_) => AgentStatus::Error,
    None => match title {
      Some(TitleStatus::Working) => AgentStatus::Running,
      Some(TitleStatus::Idle) => AgentStatus::Idle,
      None if last_activity.elapsed() < RUNNING_WINDOW => AgentStatus::Running,
      None => AgentStatus::Idle,
    },
  }
}
