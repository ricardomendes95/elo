//! Map egui keys and modifiers to Windows virtual-key codes (VK_*), so the
//! controller can forward real keyboard input to the host.

use egui::Key;

/// Windows VK code for an egui key, if we forward it.
pub fn vk(key: Key) -> Option<u16> {
    use Key::*;
    let code: u16 = match key {
        A => 0x41, B => 0x42, C => 0x43, D => 0x44, E => 0x45, F => 0x46,
        G => 0x47, H => 0x48, I => 0x49, J => 0x4A, K => 0x4B, L => 0x4C,
        M => 0x4D, N => 0x4E, O => 0x4F, P => 0x50, Q => 0x51, R => 0x52,
        S => 0x53, T => 0x54, U => 0x55, V => 0x56, W => 0x57, X => 0x58,
        Y => 0x59, Z => 0x5A,

        Num0 => 0x30, Num1 => 0x31, Num2 => 0x32, Num3 => 0x33, Num4 => 0x34,
        Num5 => 0x35, Num6 => 0x36, Num7 => 0x37, Num8 => 0x38, Num9 => 0x39,

        Enter => 0x0D,
        Escape => 0x1B,
        Backspace => 0x08,
        Tab => 0x09,
        Space => 0x20,
        Delete => 0x2E,
        Insert => 0x2D,
        Home => 0x24,
        End => 0x23,
        PageUp => 0x21,
        PageDown => 0x22,

        ArrowLeft => 0x25,
        ArrowUp => 0x26,
        ArrowRight => 0x27,
        ArrowDown => 0x28,

        Minus => 0xBD,
        Equals => 0xBB,
        Semicolon => 0xBA,
        Comma => 0xBC,
        Period => 0xBE,
        Slash => 0xBF,
        Backslash => 0xDC,
        OpenBracket => 0xDB,
        CloseBracket => 0xDD,
        Backtick => 0xC0,
        Quote => 0xDE,

        F1 => 0x70, F2 => 0x71, F3 => 0x72, F4 => 0x73, F5 => 0x74, F6 => 0x75,
        F7 => 0x76, F8 => 0x77, F9 => 0x78, F10 => 0x79, F11 => 0x7A, F12 => 0x7B,

        _ => return None,
    };
    Some(code)
}

/// VK codes for the modifier keys we track by diffing egui's modifier state.
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12; // Alt
