use std::path::Path;

pub fn quote_path(path: &Path) -> String {
  let raw = path.to_string_lossy();

  if !raw.is_empty() && raw.chars().all(is_literal) {
    return raw.into_owned();
  }

  format!("'{}'", raw.replace('\'', r"'\''"))
}

fn is_literal(ch: char) -> bool {
  ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '@' | '+' | ',' | ':' | '=')
}
