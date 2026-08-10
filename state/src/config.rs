use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorktreeConfig {
  pub path: PathBuf,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub display_name: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub issue: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub pr: Option<String>,
}

impl WorktreeConfig {
  pub fn new(path: PathBuf) -> Self {
    Self {
      path,
      display_name: None,
      issue: None,
      pr: None,
    }
  }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WorktreeConfigCompat {
  Plain(PathBuf),
  Full(WorktreeConfig),
}

fn deserialize_worktrees<'de, D>(deserializer: D) -> Result<Vec<WorktreeConfig>, D::Error>
where
  D: serde::Deserializer<'de>,
{
  let raw: Vec<WorktreeConfigCompat> = Vec::deserialize(deserializer)?;
  Ok(
    raw
      .into_iter()
      .map(|entry| match entry {
        WorktreeConfigCompat::Plain(path) => WorktreeConfig::new(path),
        WorktreeConfigCompat::Full(config) => config,
      })
      .collect(),
  )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectConfig {
  pub path: PathBuf,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub emoji: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub icon: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub display_name: Option<String>,
  #[serde(
    default,
    skip_serializing_if = "Vec::is_empty",
    deserialize_with = "deserialize_worktrees"
  )]
  pub worktrees: Vec<WorktreeConfig>,
}

impl ProjectConfig {
  fn new(path: PathBuf) -> Self {
    Self {
      path,
      emoji: None,
      icon: None,
      display_name: None,
      worktrees: Vec::new(),
    }
  }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HelixConfig {
  #[serde(default)]
  pub projects: Vec<ProjectConfig>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub terminal_font: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub terminal_font_size: Option<f32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub blur_level: Option<String>,
}

pub fn set_terminal_font(font: Option<String>) {
  let mut config = load();
  config.terminal_font = font.filter(|f| !f.trim().is_empty());
  save(&config);
}

pub fn set_terminal_font_size(size: f32) {
  let mut config = load();
  config.terminal_font_size = Some(size.clamp(9.0, 22.0));
  save(&config);
}

pub fn set_blur_level(level: &str) {
  let mut config = load();
  config.blur_level = Some(level.to_string());
  save(&config);
}

pub fn app_dir() -> Option<PathBuf> {
  Some(dirs::config_dir()?.join("helix"))
}

fn config_path() -> Option<PathBuf> {
  Some(app_dir()?.join("config.json"))
}

pub fn load() -> HelixConfig {
  let Some(path) = config_path() else {
    return HelixConfig::default();
  };
  std::fs::read_to_string(path)
    .ok()
    .and_then(|content| serde_json::from_str(&content).ok())
    .unwrap_or_default()
}

pub fn save(config: &HelixConfig) {
  let Some(path) = config_path() else { return };
  if let Some(parent) = path.parent() {
    let _ = std::fs::create_dir_all(parent);
  }
  if let Ok(json) = serde_json::to_string_pretty(config) {
    let _ = std::fs::write(path, json);
  }
}

pub fn ensure_project(root: &Path) {
  let mut config = load();
  if !config.projects.iter().any(|p| p.path == root) {
    config.projects.push(ProjectConfig::new(root.to_path_buf()));
    save(&config);
  }
}

pub fn remove_worktree(project_root: &Path, worktree: &Path) {
  update_project(project_root, |project| {
    project.worktrees.retain(|w| w.path != worktree);
  });
}

pub fn worktree_config_for(project_root: &Path, worktree: &Path) -> Option<WorktreeConfig> {
  project_for(project_root)?
    .worktrees
    .into_iter()
    .find(|w| w.path == worktree)
}

pub fn set_worktree_meta(
  project_root: &Path,
  worktree: &Path,
  display_name: Option<String>,
  issue: Option<String>,
  pr: Option<String>,
) {
  update_project(project_root, |project| {
    let entry = match project.worktrees.iter_mut().find(|w| w.path == worktree) {
      Some(entry) => entry,
      None => {
        project
          .worktrees
          .push(WorktreeConfig::new(worktree.to_path_buf()));
        project.worktrees.last_mut().unwrap()
      }
    };
    entry.display_name = display_name.filter(|v| !v.trim().is_empty());
    entry.issue = issue.filter(|v| !v.trim().is_empty());
    entry.pr = pr.filter(|v| !v.trim().is_empty());
  });
}

pub fn remove_project(root: &Path) {
  let mut config = load();
  config.projects.retain(|p| p.path != root);
  save(&config);
}

pub fn project_for(root: &Path) -> Option<ProjectConfig> {
  load().projects.into_iter().find(|p| p.path == root)
}

fn update_project(root: &Path, apply: impl FnOnce(&mut ProjectConfig)) {
  let mut config = load();
  match config.projects.iter_mut().find(|p| p.path == root) {
    Some(project) => apply(project),
    None => {
      let mut project = ProjectConfig::new(root.to_path_buf());
      apply(&mut project);
      config.projects.push(project);
    }
  }
  save(&config);
}

pub fn set_display_name(root: &Path, name: Option<String>) {
  update_project(root, |project| project.display_name = name);
}

pub fn set_icon(root: &Path, icon: &str) {
  update_project(root, |project| {
    project.icon = Some(icon.to_string());
    project.emoji = None;
  });
}

pub fn add_worktree(project_root: &Path, worktree: &Path) {
  update_project(project_root, |project| {
    if !project.worktrees.iter().any(|w| w.path == worktree) {
      project
        .worktrees
        .push(WorktreeConfig::new(worktree.to_path_buf()));
    }
  });
}

pub fn worktrees_for(project_root: &Path) -> Vec<PathBuf> {
  load()
    .projects
    .iter()
    .find(|p| p.path == project_root)
    .map(|p| p.worktrees.iter().map(|w| w.path.clone()).collect())
    .unwrap_or_default()
}

pub fn emoji_for(root: &Path) -> Option<String> {
  load()
    .projects
    .iter()
    .find(|p| p.path == root)
    .and_then(|p| p.emoji.clone())
}

pub fn set_emoji(root: &Path, emoji: &str) {
  update_project(root, |project| {
    project.emoji = Some(emoji.to_string());
    project.icon = None;
  });
}
