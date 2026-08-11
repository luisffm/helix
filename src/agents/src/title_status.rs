use helix_models::TitleStatus;

const BRAILLE_FIRST: char = '\u{2800}';
const BRAILLE_LAST: char = '\u{28ff}';

const CLAUDE_IDLE: char = '✳';

/// A coding agent paints its own state into the OSC title: Claude Code prefixes
/// an idle title with `✳` and animates braille frames while it works. That title
/// is the only honest activity signal, because a running TUI keeps redrawing
/// itself — and so keeps the pty busy — with nothing to report.
///
/// Claiming idle wrongly would freeze a spinner that should turn, so only the
/// two documented idle markers claim it and every other decorative glyph
/// returns `None`, leaving the caller on its own activity heuristic.
pub fn from_title(title: &str) -> Option<TitleStatus> {
  let title = title.trim_start();

  if title.chars().any(is_braille_frame) {
    return Some(TitleStatus::Working);
  }

  let mut chars = title.chars();
  let first = chars.next()?;

  if first == CLAUDE_IDLE {
    return Some(TitleStatus::Idle);
  }

  match (first, chars.next()) {
    ('.', Some(' ')) => Some(TitleStatus::Working),
    ('*', Some(' ')) => Some(TitleStatus::Idle),
    _ => None,
  }
}

fn is_braille_frame(c: char) -> bool {
  (BRAILLE_FIRST..=BRAILLE_LAST).contains(&c)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn braille_frames_mean_working() {
    assert_eq!(
      from_title("⠋ Fixing the parser"),
      Some(TitleStatus::Working)
    );
    assert_eq!(from_title("⣾ helix"), Some(TitleStatus::Working));
  }

  #[test]
  fn the_claude_glyph_means_idle() {
    assert_eq!(from_title("✳ Fix the auth bug"), Some(TitleStatus::Idle));
    assert_eq!(from_title("✳"), Some(TitleStatus::Idle));
    assert_eq!(from_title("  ✳ ~/projects/helix"), Some(TitleStatus::Idle));
  }

  #[test]
  fn other_decorative_glyphs_claim_nothing() {
    assert_eq!(from_title("✻ Thinking…"), None);
    assert_eq!(from_title("✽ helix"), None);
  }

  #[test]
  fn prefix_conventions_are_honored() {
    assert_eq!(from_title(". running tests"), Some(TitleStatus::Working));
    assert_eq!(from_title("* waiting for input"), Some(TitleStatus::Idle));
  }

  #[test]
  fn a_plain_shell_title_carries_no_status() {
    assert_eq!(from_title(""), None);
    assert_eq!(from_title("~/projects/helix"), None);
    assert_eq!(from_title("*.log cleanup"), None);
    assert_eq!(from_title("node index.js"), None);
  }
}
