pub mod language;
pub mod signature;

use anyhow::Result;
use std::io::Read;
use std::path::Path;

pub const MAX_TEXT_FILE_BYTES: u64 = 50 * 1024 * 1024;
pub const BINARY_PROBE_BYTES: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileContent {
  Text { text: String, signature: u64 },
  Image { bytes: Vec<u8>, mime: &'static str },
  Binary,
  TooLarge { bytes: u64 },
}

impl FileContent {
  pub fn as_text(&self) -> Option<&str> {
    match self {
      FileContent::Text { text, .. } => Some(text),
      _ => None,
    }
  }
}

const IMAGE_MIMES: [(&str, &str); 8] = [
  ("png", "image/png"),
  ("jpg", "image/jpeg"),
  ("jpeg", "image/jpeg"),
  ("gif", "image/gif"),
  ("webp", "image/webp"),
  ("bmp", "image/bmp"),
  ("ico", "image/x-icon"),
  ("svg", "image/svg+xml"),
];

pub fn image_mime(path: &Path) -> Option<&'static str> {
  let ext = path.extension()?.to_str()?.to_ascii_lowercase();
  IMAGE_MIMES
    .iter()
    .find(|(candidate, _)| *candidate == ext)
    .map(|(_, mime)| *mime)
}

pub fn read(path: &Path) -> Result<FileContent> {
  let metadata = std::fs::metadata(path)?;
  let len = metadata.len();

  if len > MAX_TEXT_FILE_BYTES {
    return Ok(FileContent::TooLarge { bytes: len });
  }

  if let Some(mime) = image_mime(path) {
    if mime == "image/svg+xml" {
      return read_text_or_binary(path, len);
    }

    return Ok(FileContent::Image {
      bytes: std::fs::read(path)?,
      mime,
    });
  }

  read_text_or_binary(path, len)
}

/// The head of the file decides whether the rest is worth reading at all, and
/// it comes off the same handle, so a text file is not opened twice and a large
/// binary never gets pulled into memory whole.
fn read_text_or_binary(path: &Path, len: u64) -> Result<FileContent> {
  let mut file = std::fs::File::open(path)?;
  let mut bytes = Vec::with_capacity(len as usize);

  file
    .by_ref()
    .take(BINARY_PROBE_BYTES as u64)
    .read_to_end(&mut bytes)?;

  if looks_binary(&bytes) {
    return Ok(FileContent::Binary);
  }

  file.read_to_end(&mut bytes)?;

  Ok(from_bytes(bytes))
}

pub fn from_bytes(bytes: Vec<u8>) -> FileContent {
  if looks_binary(&bytes) {
    return FileContent::Binary;
  }

  match String::from_utf8(bytes) {
    Ok(text) => {
      let signature = signature::of(&text);
      FileContent::Text { text, signature }
    }
    Err(_) => FileContent::Binary,
  }
}

fn looks_binary(bytes: &[u8]) -> bool {
  let window = &bytes[..bytes.len().min(BINARY_PROBE_BYTES)];

  window.contains(&0)
}

pub fn write(path: &Path, text: &str) -> Result<u64> {
  std::fs::write(path, text)?;
  Ok(signature::of(text))
}

/// Saving straight from a chunked buffer keeps the whole file from being copied
/// into one string just to be handed to the filesystem.
pub fn write_chunks<'a>(path: &Path, chunks: impl Iterator<Item = &'a str>) -> Result<u64> {
  use std::io::Write;

  let mut writer = std::io::BufWriter::new(std::fs::File::create(path)?);
  let mut hasher = signature::Hasher::new();

  for chunk in chunks {
    writer.write_all(chunk.as_bytes())?;
    hasher.write(chunk);
  }

  writer.flush()?;

  Ok(hasher.finish())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn nul_byte_is_binary() {
    assert_eq!(from_bytes(vec![b'a', 0, b'b']), FileContent::Binary);
  }

  #[test]
  fn utf8_is_text() {
    let content = from_bytes("olá".as_bytes().to_vec());
    assert_eq!(content.as_text(), Some("olá"));
  }

  #[test]
  fn invalid_utf8_is_binary() {
    assert_eq!(from_bytes(vec![0xff, 0xfe]), FileContent::Binary);
  }
}
