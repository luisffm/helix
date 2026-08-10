use helix_models::SessionKind;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct SessionUsage {
  pub title: String,
  pub kind: SessionKind,
  pub cpu: f32,
  pub rss_mb: f32,
}

#[derive(Clone, Debug)]
pub struct ProjectUsage {
  pub name: String,
  pub root: PathBuf,
  pub cpu: f32,
  pub rss_mb: f32,
  pub sessions: Vec<SessionUsage>,
}

#[derive(Clone, Debug, Default)]
pub struct UsageSnapshot {
  pub projects: Vec<ProjectUsage>,
  pub app_cpu: f32,
  pub app_rss_mb: f32,
  pub total_cpu: f32,
  pub total_rss_mb: f32,
}

pub type UsageTargets = Vec<(String, PathBuf, Vec<(String, SessionKind, u32)>)>;

pub fn sample(targets: UsageTargets) -> UsageSnapshot {
  let Ok(output) = std::process::Command::new("ps")
    .args(["-axo", "pid=,ppid=,rss=,pcpu="])
    .output()
  else {
    return UsageSnapshot::default();
  };
  let text = String::from_utf8_lossy(&output.stdout);

  let mut info: HashMap<u32, (f32, f32)> = HashMap::new();
  let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
  for line in text.lines() {
    let mut parts = line.split_whitespace();
    let (Some(pid), Some(ppid), Some(rss), Some(cpu)) =
      (parts.next(), parts.next(), parts.next(), parts.next())
    else {
      continue;
    };
    let (Ok(pid), Ok(ppid), Ok(rss), Ok(cpu)) = (
      pid.parse::<u32>(),
      ppid.parse::<u32>(),
      rss.parse::<f32>(),
      cpu.parse::<f32>(),
    ) else {
      continue;
    };
    info.insert(pid, (rss, cpu));
    children.entry(ppid).or_default().push(pid);
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

pub fn format_rss(rss_mb: f32) -> String {
  if rss_mb >= 1024.0 {
    format!("{:.1} GB", rss_mb / 1024.0)
  } else {
    format!("{:.1} MB", rss_mb)
  }
}
