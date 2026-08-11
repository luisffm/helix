use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};

/// The matcher, the parsed pattern and the scratch buffers outlive a single
/// query, so filtering a list once per keystroke allocates nothing after the
/// first run. Scores are opaque: only their order carries meaning.
pub struct Ranker {
  matcher: Matcher,
  pattern: Pattern,
  chars: Vec<char>,
  scored: Vec<(u32, usize)>,
  empty_query: bool,
}

impl Ranker {
  pub fn new() -> Self {
    Self::with_config(Config::DEFAULT)
  }

  /// Path bonuses reward matches after a separator, which is what makes a query
  /// land on the file name rather than deep inside the directory chain.
  pub fn for_paths() -> Self {
    Self::with_config(Config::DEFAULT.match_paths())
  }

  fn with_config(config: Config) -> Self {
    Self {
      matcher: Matcher::new(config),
      pattern: Pattern::default(),
      chars: Vec::new(),
      scored: Vec::new(),
      empty_query: true,
    }
  }

  pub fn set_query(&mut self, query: &str) {
    self
      .pattern
      .reparse(query, CaseMatching::Smart, Normalization::Smart);

    self.empty_query = query.trim().is_empty();
  }

  pub fn score(&mut self, candidate: &str) -> Option<u32> {
    if self.empty_query {
      return Some(0);
    }

    self
      .pattern
      .score(Utf32Str::new(candidate, &mut self.chars), &mut self.matcher)
  }

  /// Fills `out` with the indices of the candidates that matched, best first.
  /// Equal scores keep the caller's order so a listing stays stable.
  pub fn rank_into<'a>(
    &mut self,
    candidates: impl IntoIterator<Item = &'a str>,
    out: &mut Vec<usize>,
  ) {
    self.scored.clear();
    out.clear();

    for (ix, candidate) in candidates.into_iter().enumerate() {
      if let Some(score) = self.score(candidate) {
        self.scored.push((score, ix));
      }
    }

    if !self.empty_query {
      self
        .scored
        .sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }

    out.extend(self.scored.iter().map(|(_, ix)| *ix));
  }
}

impl Default for Ranker {
  fn default() -> Self {
    Self::new()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const CANDIDATES: [&str; 4] = [
    "sidebar_right.rs",
    "search.rs",
    "terminal_view.rs",
    "src/ui/src/theme.rs",
  ];

  fn ranked(ranker: &mut Ranker, query: &str) -> Vec<&'static str> {
    let mut out = Vec::new();

    ranker.set_query(query);
    ranker.rank_into(CANDIDATES, &mut out);

    out.into_iter().map(|ix| CANDIDATES[ix]).collect()
  }

  #[test]
  fn an_empty_query_keeps_every_candidate_in_order() {
    let mut ranker = Ranker::new();

    assert_eq!(ranked(&mut ranker, ""), CANDIDATES);
    assert_eq!(ranked(&mut ranker, "   "), CANDIDATES);
  }

  #[test]
  fn a_subsequence_matches_without_being_contiguous() {
    let mut ranker = Ranker::new();

    assert_eq!(ranked(&mut ranker, "sbr"), vec!["sidebar_right.rs"]);
  }

  #[test]
  fn a_closer_match_ranks_first() {
    let mut ranker = Ranker::new();
    let found = ranked(&mut ranker, "sea");

    assert_eq!(found.first(), Some(&"search.rs"));
  }

  #[test]
  fn a_query_that_matches_nothing_yields_nothing() {
    let mut ranker = Ranker::new();

    assert!(ranked(&mut ranker, "zzzz").is_empty());
  }

  #[test]
  fn path_bonuses_favor_the_file_name() {
    let mut ranker = Ranker::for_paths();
    let found = ranked(&mut ranker, "theme");

    assert_eq!(found, vec!["src/ui/src/theme.rs"]);
  }
}
