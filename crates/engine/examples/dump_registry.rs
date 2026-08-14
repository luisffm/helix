//! Print a data dir's registry rows — projects, chats, sessions.
//!
//! Read-only on purpose: it opens the SQLite store and decodes the snapshot
//! instead of assembling an engine, so it takes no data-dir lock and can never
//! write over the rows it is meant to inspect.
//!
//! Usage: cargo run -p helix-engine --example dump_registry -- <data_dir>
use helix_doc::RegistryDoc;
use helix_store::DocsStore;

fn main() -> anyhow::Result<()> {
  let data_dir = std::env::args()
    .nth(1)
    .ok_or_else(|| anyhow::anyhow!("usage: dump_registry <data_dir>"))?;
  let dir = std::path::Path::new(&data_dir)
    .join("profiles")
    .join("local");
  let store = DocsStore::open(&dir)?;
  let Some(bytes) = store.load_snapshot("registry1")? else {
    println!("no registry snapshot under {}", dir.display());
    return Ok(());
  };
  let doc = RegistryDoc::from_bytes(&bytes, "dump")?;
  let state = doc.read_all()?;
  println!("spaces: {}", state.spaces.len());
  for space in &state.spaces {
    println!(
      "  {} path={} git={}",
      space.display_name(),
      space.path,
      space.git_detected
    );
  }
  println!("chats: {}", state.chats.len());
  for chat in &state.chats {
    println!(
      "  {:?} space={:?} cwd={:?} archived={}",
      chat.title, chat.space_id, chat.cwd, chat.archived
    );
  }
  println!("sessions: {}", state.sessions.len());
  Ok(())
}
