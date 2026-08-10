use gpui::{Action, KeyBinding, actions};
use std::path::PathBuf;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = helix, no_json)]
pub struct OpenProjectSettingsAction {
  pub root: PathBuf,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = helix, no_json)]
pub struct RemoveProjectAction {
  pub root: PathBuf,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = helix, no_json)]
pub struct EditWorktreeAction {
  pub owner: PathBuf,
  pub path: PathBuf,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = helix, no_json)]
pub struct RemoveWorktreeAction {
  pub owner: PathBuf,
  pub path: PathBuf,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = helix, no_json)]
pub struct DeleteWorktreeAction {
  pub owner: PathBuf,
  pub path: PathBuf,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = helix, no_json)]
pub struct OpenInZedAction {
  pub path: PathBuf,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = helix, no_json)]
pub struct OpenInFinderAction {
  pub path: PathBuf,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = helix, no_json)]
pub struct CopyPathAction {
  pub path: PathBuf,
}

actions!(
  helix,
  [
    NewTerminal,
    NewClaudeSession,
    CloseActiveTab,
    NextTab,
    PrevTab,
    ToggleLeftSidebar,
    ToggleRightSidebar,
    OpenSearch,
    OpenAppSettings,
    Quit,
    SaveFile,
    TerminalCopy,
    TerminalPaste,
  ]
);

pub fn default_bindings() -> Vec<KeyBinding> {
  vec![
    KeyBinding::new("cmd-t", NewTerminal, None),
    KeyBinding::new("cmd-shift-t", NewClaudeSession, None),
    KeyBinding::new("cmd-w", CloseActiveTab, None),
    KeyBinding::new("ctrl-tab", NextTab, None),
    KeyBinding::new("ctrl-shift-tab", PrevTab, None),
    KeyBinding::new("cmd-shift-]", NextTab, None),
    KeyBinding::new("cmd-shift-[", PrevTab, None),
    KeyBinding::new("cmd-b", ToggleLeftSidebar, None),
    KeyBinding::new("cmd-r", ToggleRightSidebar, None),
    KeyBinding::new("cmd-k", OpenSearch, None),
    KeyBinding::new("cmd-p", OpenSearch, None),
    KeyBinding::new("cmd-,", OpenAppSettings, None),
    KeyBinding::new("cmd-s", SaveFile, Some("Editor")),
    KeyBinding::new("cmd-q", Quit, None),
    KeyBinding::new("cmd-c", TerminalCopy, Some("Terminal")),
    KeyBinding::new("cmd-v", TerminalPaste, Some("Terminal")),
  ]
}
