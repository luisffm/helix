#![allow(unexpected_cfgs)]

const SOURCE_ICON: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/icon.png");

#[cfg(target_os = "macos")]
pub fn apply_unbundled_icon() {
  use objc::runtime::Object;
  use objc::{class, msg_send, sel, sel_impl};

  let Ok(bytes) = std::fs::read(SOURCE_ICON) else {
    return;
  };

  unsafe {
    let data: *mut Object = msg_send![class!(NSData), alloc];
    let data: *mut Object = msg_send![data, initWithBytes: bytes.as_ptr() length: bytes.len()];
    if data.is_null() {
      return;
    }
    let image: *mut Object = msg_send![class!(NSImage), alloc];
    let image: *mut Object = msg_send![image, initWithData: data];
    if image.is_null() {
      return;
    }
    let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
    let _: () = msg_send![app, setApplicationIconImage: image];
  }
}

#[cfg(not(target_os = "macos"))]
pub fn apply_unbundled_icon() {}
