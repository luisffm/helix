use std::path::Path;

const BY_EXTENSION: [(&str, &str); 40] = [
  ("rs", "rust"),
  ("ts", "typescript"),
  ("tsx", "tsx"),
  ("js", "javascript"),
  ("jsx", "javascript"),
  ("mjs", "javascript"),
  ("cjs", "javascript"),
  ("json", "json"),
  ("jsonc", "json"),
  ("py", "python"),
  ("pyi", "python"),
  ("go", "go"),
  ("rb", "ruby"),
  ("java", "java"),
  ("kt", "java"),
  ("c", "c"),
  ("h", "c"),
  ("cc", "cpp"),
  ("cpp", "cpp"),
  ("cxx", "cpp"),
  ("hpp", "cpp"),
  ("cs", "c_sharp"),
  ("swift", "swift"),
  ("scala", "scala"),
  ("zig", "zig"),
  ("ex", "elixir"),
  ("exs", "elixir"),
  ("sh", "bash"),
  ("bash", "bash"),
  ("zsh", "bash"),
  ("fish", "bash"),
  ("toml", "toml"),
  ("yaml", "yaml"),
  ("yml", "yaml"),
  ("md", "markdown"),
  ("markdown", "markdown"),
  ("html", "html"),
  ("htm", "html"),
  ("css", "css"),
  ("sql", "sequel"),
];

const BY_FILENAME: [(&str, &str); 8] = [
  ("Cargo.lock", "toml"),
  ("Dockerfile", "bash"),
  ("Makefile", "make"),
  ("makefile", "make"),
  (".gitignore", "diff"),
  (".zshrc", "bash"),
  (".bashrc", "bash"),
  ("CMakeLists.txt", "cmake"),
];

pub fn of(path: &Path) -> &'static str {
  if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
    if let Some((_, language)) = BY_FILENAME.iter().find(|(candidate, _)| *candidate == name) {
      return language;
    }
  }
  let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
    return "text";
  };
  let ext = ext.to_ascii_lowercase();
  BY_EXTENSION
    .iter()
    .find(|(candidate, _)| *candidate == ext)
    .map(|(_, language)| *language)
    .unwrap_or("text")
}

#[cfg(test)]
mod tests {
  use super::of;
  use std::path::Path;

  #[test]
  fn maps_extension() {
    assert_eq!(of(Path::new("/a/b/main.rs")), "rust");
    assert_eq!(of(Path::new("main.TSX")), "tsx");
  }

  #[test]
  fn filename_wins_over_extension() {
    assert_eq!(of(Path::new("/repo/Cargo.lock")), "toml");
  }

  #[test]
  fn unknown_is_text() {
    assert_eq!(of(Path::new("blob.qqq")), "text");
    assert_eq!(of(Path::new("LICENSE")), "text");
  }
}
