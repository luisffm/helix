const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

pub fn of(text: &str) -> u64 {
  let mut hash = FNV_OFFSET;

  for byte in text.as_bytes() {
    hash ^= *byte as u64;
    hash = hash.wrapping_mul(FNV_PRIME);
  }

  hash ^ (text.len() as u64).rotate_left(32)
}

#[cfg(test)]
mod tests {
  use super::of;

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
}
