use std::path::Path;

pub fn abbreviate_home(path: &Path) -> String {
  let text = path.display().to_string();
  let home = std::env::var("HOME").unwrap_or_default();

  if home.is_empty() || !text.starts_with(&home) {
    return text;
  }

  format!("~{}", &text[home.len()..])
}
