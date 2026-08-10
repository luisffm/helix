use alacritty_terminal::term::TermMode;
use gpui::Keystroke;

pub fn to_pty_bytes(keystroke: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
  let mods = keystroke.modifiers;

  if mods.platform || mods.function {
    return None;
  }

  let app_cursor = mode.contains(TermMode::APP_CURSOR);
  let any_mod = mods.shift || mods.alt || mods.control;
  let code = modifier_code(&keystroke.modifiers);

  let cursor_key = |ch: char| -> Vec<u8> {
    if any_mod {
      format!("\x1b[1;{code}{ch}").into_bytes()
    } else if app_cursor {
      format!("\x1bO{ch}").into_bytes()
    } else {
      format!("\x1b[{ch}").into_bytes()
    }
  };

  let tilde_key = |n: u8| -> Vec<u8> {
    if any_mod {
      format!("\x1b[{n};{code}~").into_bytes()
    } else {
      format!("\x1b[{n}~").into_bytes()
    }
  };

  match keystroke.key.as_str() {
    "enter" => {
      return Some(if mods.alt {
        b"\x1b\r".to_vec()
      } else {
        b"\r".to_vec()
      });
    }
    "escape" => return Some(b"\x1b".to_vec()),
    "tab" => {
      return Some(if mods.shift {
        b"\x1b[Z".to_vec()
      } else {
        b"\t".to_vec()
      });
    }
    "backspace" => {
      let mut bytes = Vec::new();

      if mods.alt {
        bytes.push(0x1b);
      }

      bytes.push(if mods.control { 0x08 } else { 0x7f });

      return Some(bytes);
    }
    "up" => return Some(cursor_key('A')),
    "down" => return Some(cursor_key('B')),
    "right" => return Some(cursor_key('C')),
    "left" => return Some(cursor_key('D')),
    "home" => {
      return Some(if any_mod {
        format!("\x1b[1;{code}H").into_bytes()
      } else if app_cursor {
        b"\x1bOH".to_vec()
      } else {
        b"\x1b[H".to_vec()
      });
    }
    "end" => {
      return Some(if any_mod {
        format!("\x1b[1;{code}F").into_bytes()
      } else if app_cursor {
        b"\x1bOF".to_vec()
      } else {
        b"\x1b[F".to_vec()
      });
    }
    "pageup" => return Some(tilde_key(5)),
    "pagedown" => return Some(tilde_key(6)),
    "insert" => return Some(tilde_key(2)),
    "delete" => return Some(tilde_key(3)),
    "f1" => return Some(fn_key(1, any_mod, code)),
    "f2" => return Some(fn_key(2, any_mod, code)),
    "f3" => return Some(fn_key(3, any_mod, code)),
    "f4" => return Some(fn_key(4, any_mod, code)),
    "f5" => return Some(tilde_key(15)),
    "f6" => return Some(tilde_key(17)),
    "f7" => return Some(tilde_key(18)),
    "f8" => return Some(tilde_key(19)),
    "f9" => return Some(tilde_key(20)),
    "f10" => return Some(tilde_key(21)),
    "f11" => return Some(tilde_key(23)),
    "f12" => return Some(tilde_key(24)),
    _ => {}
  }

  if mods.control {
    let key = if keystroke.key.as_str() == "space" {
      " "
    } else {
      keystroke.key.as_str()
    };

    if key.chars().count() == 1 {
      let ch = key.chars().next().unwrap().to_ascii_lowercase();

      let byte = match ch {
        'a'..='z' => Some(ch as u8 - b'a' + 1),
        ' ' | '@' | '2' => Some(0),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' => Some(0x1f),
        '?' => Some(0x7f),
        _ => None,
      };

      if let Some(byte) = byte {
        let mut bytes = Vec::new();

        if mods.alt {
          bytes.push(0x1b);
        }

        bytes.push(byte);

        return Some(bytes);
      }
    }

    return None;
  }

  let text = keystroke.key_char.clone().or_else(|| {
    if keystroke.key.chars().count() == 1 {
      Some(keystroke.key.clone())
    } else if keystroke.key.as_str() == "space" {
      Some(" ".to_string())
    } else {
      None
    }
  })?;

  let mut bytes = Vec::new();

  if mods.alt {
    bytes.push(0x1b);
  }

  bytes.extend_from_slice(text.as_bytes());

  Some(bytes)
}

fn fn_key(n: u8, any_mod: bool, code: u8) -> Vec<u8> {
  let ch = [b'P', b'Q', b'R', b'S'][(n - 1) as usize] as char;

  if any_mod {
    format!("\x1b[1;{code}{ch}").into_bytes()
  } else {
    format!("\x1bO{ch}").into_bytes()
  }
}

fn modifier_code(mods: &gpui::Modifiers) -> u8 {
  let mut code = 1;

  if mods.shift {
    code += 1;
  }

  if mods.alt {
    code += 2;
  }

  if mods.control {
    code += 4;
  }

  code
}
