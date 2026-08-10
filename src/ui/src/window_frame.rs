#![allow(unexpected_cfgs)]

/// Hands window geometry to AppKit's own frame autosave, which persists the
/// frame in `NSUserDefaults` and clamps it back onto a visible screen when the
/// display layout changed since the last run.
///
/// Returns whether a saved frame was found and applied.
#[cfg(target_os = "macos")]
pub fn restore_and_autosave(name: &str) -> bool {
  use objc::runtime::{BOOL, Object, YES};
  use objc::{class, msg_send, sel, sel_impl};

  unsafe {
    let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
    let windows: *mut Object = msg_send![app, windows];
    let count: usize = msg_send![windows, count];

    let Some(window) = (0..count)
      .map(|ix| -> *mut Object { msg_send![windows, objectAtIndex: ix] })
      .find(|window| !window.is_null())
    else {
      return false;
    };

    let autosave_name: *mut Object = msg_send![class!(NSString), alloc];
    let autosave_name: *mut Object = msg_send![
      autosave_name,
      initWithBytes: name.as_ptr()
      length: name.len()
      encoding: 4usize // NSUTF8StringEncoding
    ];

    let restored: BOOL = msg_send![window, setFrameUsingName: autosave_name];
    let _: BOOL = msg_send![window, setFrameAutosaveName: autosave_name];

    restored == YES
  }
}

#[cfg(not(target_os = "macos"))]
pub fn restore_and_autosave(_name: &str) -> bool {
  false
}
