use helix_models::SessionKind;
use rustc_hash::FxHashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

#[derive(Clone, Debug, PartialEq)]
pub struct SessionUsage {
  pub title: String,
  pub kind: SessionKind,
  pub pid: u32,
  pub cpu: f32,
  pub rss_mb: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectUsage {
  pub name: String,
  pub root: PathBuf,
  pub cpu: f32,
  pub rss_mb: f32,
  pub sessions: Vec<SessionUsage>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UsageSnapshot {
  pub projects: Vec<ProjectUsage>,
  pub app_cpu: f32,
  pub app_rss_mb: f32,
  pub total_cpu: f32,
  pub total_rss_mb: f32,
}

pub type UsageTargets = Vec<(String, PathBuf, Vec<(String, SessionKind, u32)>)>;

/// CPU is only meaningful over an interval, so the process table outlives a
/// single sample and each refresh reports the share used since the previous one.
/// Dead processes are dropped on every refresh, which keeps the table bounded
/// and stops it from pinning handles for processes that are already gone.
static SYSTEM: OnceLock<Mutex<System>> = OnceLock::new();

fn process_refresh() -> ProcessRefreshKind {
  ProcessRefreshKind::nothing()
    .with_cpu()
    .with_memory()
    .without_tasks()
}

pub fn sample(targets: UsageTargets) -> UsageSnapshot {
  let system = SYSTEM.get_or_init(|| {
    Mutex::new(System::new_with_specifics(
      RefreshKind::nothing().with_processes(process_refresh()),
    ))
  });

  let Ok(mut system) = system.lock() else {
    return UsageSnapshot::default();
  };

  system.refresh_processes_specifics(ProcessesToUpdate::All, true, process_refresh());

  let mut info: FxHashMap<u32, (f32, f32)> = FxHashMap::default();
  let mut children: FxHashMap<u32, Vec<u32>> = FxHashMap::default();

  for (pid, process) in system.processes() {
    let pid = pid.as_u32();

    info.insert(pid, (process.memory() as f32 / 1024.0, process.cpu_usage()));

    if let Some(parent) = process.parent() {
      children.entry(parent.as_u32()).or_default().push(pid);
    }
  }

  let subtree = |start: u32| -> (f32, f32) {
    let mut rss = 0.0;
    let mut cpu = 0.0;
    let mut queue = vec![start];
    let mut visited = 0;

    while let Some(pid) = queue.pop() {
      visited += 1;

      if visited > 512 {
        break;
      }

      if let Some((r, c)) = info.get(&pid) {
        rss += r;
        cpu += c;
      }

      if let Some(kids) = children.get(&pid) {
        queue.extend(kids.iter().copied());
      }
    }

    (rss / 1024.0, cpu)
  };

  let mut projects = Vec::new();
  let mut total_rss = 0.0;
  let mut total_cpu = 0.0;

  for (name, root, sessions) in targets {
    let mut project = ProjectUsage {
      name,
      root,
      cpu: 0.0,
      rss_mb: 0.0,
      sessions: Vec::new(),
    };

    for (title, kind, pid) in sessions {
      let (rss_mb, cpu) = subtree(pid);

      project.cpu += cpu;
      project.rss_mb += rss_mb;

      project.sessions.push(SessionUsage {
        title,
        kind,
        pid,
        cpu,
        rss_mb,
      });
    }

    total_rss += project.rss_mb;
    total_cpu += project.cpu;

    projects.push(project);
  }

  projects.sort_by(|a, b| b.rss_mb.total_cmp(&a.rss_mb));

  let app_pid = std::process::id();

  let (app_rss_mb, app_cpu) = info
    .get(&app_pid)
    .map(|(rss, cpu)| (rss / 1024.0, *cpu))
    .unwrap_or_default();
  total_rss += app_rss_mb;
  total_cpu += app_cpu;

  UsageSnapshot {
    projects,
    app_cpu,
    app_rss_mb,
    total_cpu,
    total_rss_mb: total_rss,
  }
}

pub fn status_summary(snapshot: &UsageSnapshot) -> String {
  format!(
    "{} · {:.1}%",
    format_rss(snapshot.total_rss_mb),
    snapshot.total_cpu
  )
}

pub fn format_rss(rss_mb: f32) -> String {
  if rss_mb >= 1024.0 {
    format!("{:.1} GB", rss_mb / 1024.0)
  } else {
    format!("{:.1} MB", rss_mb)
  }
}
