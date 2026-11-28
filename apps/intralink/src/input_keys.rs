//! Explicit MiniFB key mapping for the focused input pad. No global hook.
use intralink_core::input::Key;
use minifb::Key as Local;

pub fn key(local: Local) -> Option<Key> {
    use Local::*;
    let usage = match local {
        A => 4,
        B => 5,
        C => 6,
        D => 7,
        E => 8,
        F => 9,
        G => 10,
        H => 11,
        I => 12,
        J => 13,
        K => 14,
        L => 15,
        M => 16,
        N => 17,
        O => 18,
        P => 19,
        Q => 20,
        R => 21,
        S => 22,
        T => 23,
        U => 24,
        V => 25,
        W => 26,
        X => 27,
        Y => 28,
        Z => 29,
        Key1 => 30,
        Key2 => 31,
        Key3 => 32,
        Key4 => 33,
        Key5 => 34,
        Key6 => 35,
        Key7 => 36,
        Key8 => 37,
        Key9 => 38,
        Key0 => 39,
        Enter => 40,
        Backspace => 42,
        Tab => 43,
        Space => 44,
        Minus => 45,
        Equal => 46,
        LeftBracket => 47,
        RightBracket => 48,
        Backslash => 49,
        Semicolon => 51,
        Apostrophe => 52,
        Backquote => 53,
        Comma => 54,
        Period => 55,
        Slash => 56,
        CapsLock => 57,
        F10 => 67,
        F11 => 68,
        ScrollLock => 71,
        Insert => 73,
        Home => 74,
        PageUp => 75,
        Delete => 76,
        End => 77,
        PageDown => 78,
        Right => 79,
        Left => 80,
        Down => 81,
        Up => 82,
        NumLock => 83,
        NumPadSlash => 84,
        NumPadAsterisk => 85,
        NumPadMinus => 86,
        NumPadPlus => 87,
        NumPadEnter => 88,
        NumPad1 => 89,
        NumPad2 => 90,
        NumPad3 => 91,
        NumPad4 => 92,
        NumPad5 => 93,
        NumPad6 => 94,
        NumPad7 => 95,
        NumPad8 => 96,
        NumPad9 => 97,
        NumPad0 => 98,
        NumPadDot => 99,
        LeftCtrl => 224,
        LeftShift => 225,
        LeftAlt => 226,
        LeftSuper => 227,
        RightCtrl => 228,
        RightShift => 229,
        RightAlt => 230,
        RightSuper => 231,
        // F1-F8 select a host, F12 returns local, Escape closes the pad.
        _ => return None,
    };
    Key::new(usage)
}

/// Desktop-edge capture does not reserve the manual F1-F9 slot keys.
/// OS-global modifiers and lock toggles are deliberately not forwarded
/// by this temporary foreground-only prototype.
pub(super) fn desktop_key(local: Local) -> Option<Key> {
    use Local::*;
    let usage = match local {
        F1 => 58,
        F2 => 59,
        F3 => 60,
        F4 => 61,
        F5 => 62,
        F6 => 63,
        F7 => 64,
        F8 => 65,
        F9 => 66,
        LeftAlt | RightAlt | LeftSuper | RightSuper | CapsLock | NumLock | ScrollLock => {
            return None;
        }
        _ => return key(local),
    };
    Key::new(usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reserved_local_controls_never_become_remote_keys() {
        for k in [
            Local::F1,
            Local::F2,
            Local::F3,
            Local::F4,
            Local::F5,
            Local::F6,
            Local::F7,
            Local::F8,
            Local::F9,
            Local::F12,
            Local::Escape,
            Local::Unknown,
        ] {
            assert!(key(k).is_none());
        }
        assert_eq!(key(Local::A).unwrap().usage(), 4);
        assert_eq!(key(Local::RightCtrl).unwrap().usage(), 228);
    }
}
