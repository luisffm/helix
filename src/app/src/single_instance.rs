#![allow(unexpected_cfgs)]

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};

pub struct InstanceLock {
  _file: File,
}

pub fn acquire() -> Option<InstanceLock> {
  let dir = helix_state::config::app_dir()?;
  std::fs::create_dir_all(&dir).ok()?;

  let path = dir.join("helix.lock");

  let mut file = OpenOptions::new()
    .read(true)
    .write(true)
    .create(true)
    .truncate(false)
    .open(&path)
    .ok()?;

  #[cfg(unix)]
  {
    use std::os::unix::io::AsRawFd;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };

    if rc != 0 {
      let mut pid_text = String::new();
      let _ = file.read_to_string(&mut pid_text);

      if let Ok(pid) = pid_text.trim().parse::<i32>() {
        activate_running_instance(pid);
      }

      return None;
    }
  }

  let _ = file.set_len(0);
  let _ = file.rewind();
  let _ = write!(file, "{}", std::process::id());
  let _ = file.flush();

  Some(InstanceLock { _file: file })
}

#[cfg(target_os = "macos")]
fn activate_running_instance(pid: i32) {
  use objc::runtime::Object;
  use objc::{class, msg_send, sel, sel_impl};
  const ACTIVATE_IGNORING_OTHER_APPS: usize = 1 << 1;
  unsafe {
    let app: *mut Object = msg_send![
        class!(NSRunningApplication),
        runningApplicationWithProcessIdentifier: pid
    ];

    if !app.is_null() {
      let _: bool = msg_send![app, activateWithOptions: ACTIVATE_IGNORING_OTHER_APPS];
    }
  }
}

#[cfg(not(target_os = "macos"))]
fn activate_running_instance(_pid: i32) {}
