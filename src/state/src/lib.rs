pub mod config;
pub mod terminal_font;

use helix_models::AgentStatus;
use std::time::{Duration, Instant};

const RUNNING_WINDOW: Duration = Duration::from_secs(2);

pub fn activity_status(last_activity: Instant, exited: Option<i32>) -> AgentStatus {
  match exited {
    Some(0) => AgentStatus::Finished,
    Some(_) => AgentStatus::Error,
    None => {
      if last_activity.elapsed() < RUNNING_WINDOW {
        AgentStatus::Running
      } else {
        AgentStatus::Idle
      }
    }
  }
}
