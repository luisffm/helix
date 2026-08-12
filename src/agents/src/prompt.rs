/// A coding agent rings the terminal bell when it wants the user back, but the
/// bell alone does not say whether it is reporting or asking. Claude Code asks
/// by drawing a chooser — a cursor line against numbered options — or by asking
/// for a plain yes or no, and that shape is what separates the two.
///
/// `❯` on its own is the session's own prompt, so it never counts by itself:
/// the marker has to sit in front of a numbered option.
pub fn awaits_answer(lines: &[String]) -> bool {
  lines.iter().any(|line| {
    let line = line.trim();

    is_numbered_option(line) || is_yes_no(line)
  })
}

fn is_numbered_option(line: &str) -> bool {
  let Some(rest) = line.strip_prefix('❯') else {
    return false;
  };

  let rest = rest.trim_start();
  let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();

  !digits.is_empty() && rest[digits.len()..].starts_with('.')
}

fn is_yes_no(line: &str) -> bool {
  let lower = line.to_lowercase();

  lower.contains("(y/n)") || lower.contains("[y/n]") || lower.starts_with("do you want")
}

#[cfg(test)]
mod tests {
  use super::awaits_answer;

  fn lines(text: &[&str]) -> Vec<String> {
    text.iter().map(|line| line.to_string()).collect()
  }

  #[test]
  fn a_chooser_is_a_question() {
    assert!(awaits_answer(&lines(&[
      "Do you want to proceed?",
      "❯ 1. Yes",
      "  2. No, and tell Claude what to do differently",
    ])));
  }

  #[test]
  fn the_cursor_alone_is_the_prompt_not_a_question() {
    assert!(!awaits_answer(&lines(&["❯"])));
    assert!(!awaits_answer(&lines(&["❯ write the tests first"])));
    assert!(!awaits_answer(&lines(&["  ❯ 1 is not numbered"])));
  }

  #[test]
  fn a_yes_no_counts_either_way_round() {
    assert!(awaits_answer(&lines(&["Overwrite the file? (y/n)"])));
    assert!(awaits_answer(&lines(&["Continue [y/N]"])));
  }

  #[test]
  fn a_finished_answer_asks_nothing() {
    assert!(!awaits_answer(&lines(&[
      "● Done. The parser now rejects trailing commas.",
      "  └ 3 files changed",
      "❯",
    ])));
    assert!(!awaits_answer(&[]));
  }
}
