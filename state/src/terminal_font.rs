use std::path::PathBuf;

const KNOWN_NERD_FONTS: [&str; 12] = [
  "JetBrainsMonoNL Nerd Font Mono",
  "JetBrainsMonoNL Nerd Font",
  "JetBrainsMono Nerd Font Mono",
  "JetBrainsMono Nerd Font",
  "MesloLGS Nerd Font Mono",
  "MesloLGS NF",
  "Hack Nerd Font Mono",
  "Hack Nerd Font",
  "FiraCode Nerd Font Mono",
  "FiraCode Nerd Font",
  "CaskaydiaCove Nerd Font Mono",
  "Symbols Nerd Font Mono",
];

pub fn detect(available: &[String]) -> Option<String> {
  let mut candidates: Vec<String> = Vec::new();
  if let Some(configured) = crate::config::load().terminal_font {
    candidates.push(configured);
  }
  candidates.extend(ghostty_font());
  candidates.extend(kitty_font());
  candidates.extend(alacritty_font());
  candidates.extend(wezterm_font());
  candidates.extend(iterm2_font());
  candidates.extend(KNOWN_NERD_FONTS.iter().map(|s| s.to_string()));

  candidates
    .iter()
    .find_map(|candidate| resolve(candidate, available))
}

fn normalize(value: &str) -> String {
  value
    .chars()
    .filter(|c| c.is_alphanumeric())
    .collect::<String>()
    .to_lowercase()
}

fn resolve(candidate: &str, available: &[String]) -> Option<String> {
  let target = normalize(candidate);
  if target.is_empty() {
    return None;
  }
  if let Some(exact) = available.iter().find(|family| normalize(family) == target) {
    return Some(exact.clone());
  }
  available
    .iter()
    .filter(|family| {
      let family_norm = normalize(family);
      !family_norm.is_empty() && target.starts_with(&family_norm)
    })
    .max_by_key(|family| normalize(family).len())
    .cloned()
}

fn home() -> Option<PathBuf> {
  std::env::var_os("HOME").map(PathBuf::from)
}

fn ghostty_font() -> Option<String> {
  let home = home()?;
  let paths = [
    home.join(".config/ghostty/config"),
    home.join("Library/Application Support/com.mitchellh.ghostty/config"),
    home.join("Library/Application Support/com.mitchellh.ghostty/config.ghostty"),
  ];
  for path in paths {
    let Ok(content) = std::fs::read_to_string(&path) else {
      continue;
    };
    for line in content.lines() {
      let line = line.trim();
      if let Some(rest) = line.strip_prefix("font-family") {
        let value = rest
          .trim_start_matches(['=', ' ', '\t'])
          .trim()
          .trim_matches('"');
        if !value.is_empty() {
          return Some(value.to_string());
        }
      }
    }
  }
  None
}

fn kitty_font() -> Option<String> {
  let content = std::fs::read_to_string(home()?.join(".config/kitty/kitty.conf")).ok()?;
  for line in content.lines() {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix("font_family") {
      let value = rest.trim();
      if !value.is_empty() && value != "auto" {
        return Some(value.to_string());
      }
    }
  }
  None
}

fn alacritty_font() -> Option<String> {
  let content = std::fs::read_to_string(home()?.join(".config/alacritty/alacritty.toml")).ok()?;
  let mut in_font_section = false;
  for line in content.lines() {
    let line = line.trim();
    if line.starts_with('[') {
      in_font_section = line.starts_with("[font");
      continue;
    }
    if in_font_section && line.starts_with("family") {
      if let Some(value) = line.split('"').nth(1) {
        return Some(value.to_string());
      }
    }
  }
  None
}

fn wezterm_font() -> Option<String> {
  let home = home()?;
  let paths = [
    home.join(".wezterm.lua"),
    home.join(".config/wezterm/wezterm.lua"),
  ];
  for path in paths {
    let Ok(content) = std::fs::read_to_string(&path) else {
      continue;
    };
    for marker in ["font(\"", "font('"] {
      if let Some(start) = content.find(marker) {
        let rest = &content[start + marker.len()..];
        let quote = marker.chars().last().unwrap();
        if let Some(end) = rest.find(quote) {
          return Some(rest[..end].to_string());
        }
      }
    }
  }
  None
}

fn iterm2_font() -> Option<String> {
  let output = std::process::Command::new("defaults")
    .args(["read", "com.googlecode.iterm2"])
    .output()
    .ok()?;
  let text = String::from_utf8_lossy(&output.stdout);
  for line in text.lines() {
    if line.contains("\"Normal Font\"") {
      let value = line.split('=').nth(1)?.trim().trim_matches([';', '"', ' ']);
      let font = match value.rsplit_once(' ') {
        Some((name, size)) if size.chars().all(|c| c.is_ascii_digit()) => name,
        _ => value,
      };
      if !font.is_empty() {
        return Some(font.to_string());
      }
    }
  }
  None
}
