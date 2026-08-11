const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Hashing in pieces lets a caller holding a chunked buffer avoid flattening it
/// into one string first. The digest matches `of` over the same bytes.
pub struct Hasher {
  hash: u64,
  len: usize,
}

impl Hasher {
  pub fn new() -> Self {
    Self {
      hash: FNV_OFFSET,
      len: 0,
    }
  }

  pub fn write(&mut self, text: &str) {
    for byte in text.as_bytes() {
      self.hash ^= *byte as u64;
      self.hash = self.hash.wrapping_mul(FNV_PRIME);
    }

    self.len += text.len();
  }

  pub fn finish(self) -> u64 {
    self.hash ^ (self.len as u64).rotate_left(32)
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
