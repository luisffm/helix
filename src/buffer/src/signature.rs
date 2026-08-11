use xxhash_rust::xxh3::Xxh3;

/// Only ever compared against another digest taken in the same run, never
/// stored, so the hash is chosen for speed alone: a byte-at-a-time FNV read
/// 1MB in 2.8ms, which is too long for a keystroke to wait on.
///
/// Hashing in pieces lets a caller holding a chunked buffer avoid flattening it
/// into one string first. The digest matches `of` over the same bytes.
pub struct Hasher {
  inner: Xxh3,
}

impl Hasher {
  pub fn new() -> Self {
    Self { inner: Xxh3::new() }
  }

  pub fn write(&mut self, text: &str) {
    self.inner.update(text.as_bytes());
  }

  pub fn finish(self) -> u64 {
    self.inner.digest()
  }
}

impl Default for Hasher {
  fn default() -> Self {
    Self::new()
  }
}

pub fn of(text: &str) -> u64 {
  of_chunks(std::iter::once(text))
}

pub fn of_chunks<'a>(chunks: impl Iterator<Item = &'a str>) -> u64 {
  let mut hasher = Hasher::new();

  for chunk in chunks {
    hasher.write(chunk);
  }

  hasher.finish()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn differs_on_content() {
    assert_ne!(of("a"), of("b"));
  }

  #[test]
  fn differs_on_length() {
    assert_ne!(of("aa"), of("aaa"));
  }

  #[test]
  fn stable() {
    assert_eq!(of("fn main() {}"), of("fn main() {}"));
  }

  #[test]
  fn chunking_does_not_change_the_digest() {
    let whole = "fn main() {\n  println!(\"hi\");\n}\n";

    assert_eq!(
      of_chunks(["fn main() {\n", "  println!(\"hi\");\n", "}\n"].into_iter()),
      of(whole)
    );
  }
}
