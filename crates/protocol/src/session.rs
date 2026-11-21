//! Messages carried *inside* the end-to-end encrypted session (after the relay
//! has paired the two peers and the Noise handshake has completed). The relay
//! never sees any of these; only the two clients serialize/deserialize them.

use serde::{Deserialize, Serialize};

/// What the controller is allowed to do this session. The host owns these and
/// sends them right after auth; a view-only session sets `control = false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permissions {
    pub control: bool,
    pub file_transfer: bool,
    pub clipboard: bool,
    pub audio: bool,
}

impl Permissions {
    pub const VIEW_ONLY: Permissions = Permissions {
        control: false,
        file_transfer: false,
        clipboard: false,
        audio: false,
    };
    pub const FULL: Permissions = Permissions {
        control: true,
        file_transfer: true,
        clipboard: true,
        audio: true,
    };
}

/// A changed rectangle of the screen. `data` is zlib-compressed BGRA pixels,
/// row-major, `w * h * 4` bytes once inflated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

/// One encoded video frame. A key frame carries every tile; a delta carries
/// only the tiles that changed since the previous frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub key: bool,
    pub seq: u64,
    pub rects: Vec<Rect>,
}

/// Mouse buttons we forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// An input event the controller sends to the host. Coordinates are absolute
/// pixels in the host's screen space (0..width, 0..height).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputEvent {
    MouseMove { x: i32, y: i32 },
    MouseButton { button: MouseButton, down: bool },
    Scroll { dx: i32, dy: i32 },
    /// `code` is a Windows virtual-key code (VK_*). `down` is press vs release.
    Key { code: u16, down: bool },
    /// A typed character (Unicode scalar value), for text the VK table cannot
    /// express (ç, á, ñ, €…) regardless of the two keyboards' layouts.
    Char { code: u32 },
}

/// A file-transfer identifier, unique within a session.
pub type TransferId = u32;

/// Everything that can travel over the encrypted channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionMsg {
    // ---- authentication (runs inside the encrypted, key-pinned channel) ----
    /// Host -> controller: prove knowledge of the session password.
    AuthRequired { salt: [u8; 16] },
    /// Controller -> host: `SHA256(salt || password)`.
    AuthSubmit { proof: Vec<u8> },
    AuthOk,
    AuthFailed,

    // ---- session setup ----
    Permissions(Permissions),
    ScreenInfo { width: u32, height: u32 },

    // ---- live session ----
    Frame(VideoFrame),
    Input(InputEvent),
    Chat(String),

    // ---- file transfer ----
    FileOffer {
        id: TransferId,
        name: String,
        size: u64,
    },
    FileAccept {
        id: TransferId,
    },
    FileChunk {
        id: TransferId,
        offset: u64,
        data: Vec<u8>,
    },
    FileDone {
        id: TransferId,
    },
    FileCancel {
        id: TransferId,
        reason: String,
    },

    // ---- keepalive ----
    Ping,
    Pong,

    // ---- added in protocol v2 (appended: earlier indices are unchanged) ----
    /// Host -> controller: after the password, an authenticator (TOTP) code is
    /// also required.
    TwoFaRequired,
    /// Controller -> host: the 6-digit TOTP code.
    TwoFaSubmit { code: String },
    /// Controller -> host, first message after the Noise handshake: who is
    /// connecting (shown in the approval prompt and the "connected" banner).
    Hello { name: String },
    /// Controller -> host: the operator's pointer position (host screen pixels).
    /// Shown on the host when the operator cannot control.
    Pointer { x: i32, y: i32 },
    /// Text clipboard sync, either direction.
    Clipboard(String),
    /// Host -> controller: the host's monitors (and which one is captured).
    Monitors { list: Vec<MonitorInfo> },
    /// Controller -> host: capture another monitor.
    SelectMonitor { index: u32 },
    /// Controller -> host: send Ctrl+Alt+Del (secure attention sequence).
    SendSas,
    /// Controller -> host, right after `AuthOk`: what the controller supports.
    Caps { h264: bool, audio: bool },
    /// Host -> controller: an H.264 (Annex-B) encoded frame.
    FrameH264 { width: u32, height: u32, key: bool, data: Vec<u8> },
    /// Host -> controller: PCM audio, signed 16-bit little-endian, interleaved.
    Audio { rate: u32, channels: u16, pcm: Vec<u8> },
}

/// One of the host's displays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub index: u32,
    pub name: String,
    /// Position in the virtual desktop (pixels).
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
    /// True for the monitor currently being captured.
    pub current: bool,
}

/// Serialize a session message with bincode (infallible for these types).
pub fn encode_msg(m: &SessionMsg) -> Vec<u8> {
    bincode::serialize(m).expect("SessionMsg serialization is infallible")
}

/// Deserialize a session message.
pub fn decode_msg(bytes: &[u8]) -> Result<SessionMsg, bincode::Error> {
    bincode::deserialize(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_msg_roundtrips_via_bincode() {
        let msgs = vec![
            SessionMsg::AuthRequired { salt: [7u8; 16] },
            SessionMsg::AuthSubmit { proof: vec![1, 2, 3] },
            SessionMsg::AuthOk,
            SessionMsg::Permissions(Permissions::VIEW_ONLY),
            SessionMsg::ScreenInfo { width: 1920, height: 1080 },
            SessionMsg::Frame(VideoFrame {
                width: 2,
                height: 2,
                key: true,
                seq: 42,
                rects: vec![Rect { x: 0, y: 0, w: 2, h: 2, data: vec![9; 8] }],
            }),
            SessionMsg::Input(InputEvent::MouseMove { x: 10, y: 20 }),
            SessionMsg::Input(InputEvent::Key { code: 0x41, down: true }),
            SessionMsg::Chat("olá".into()),
            SessionMsg::FileOffer { id: 1, name: "a.txt".into(), size: 5 },
            SessionMsg::FileChunk { id: 1, offset: 0, data: vec![1, 2, 3, 4, 5] },
            SessionMsg::FileDone { id: 1 },
        ];
        for m in msgs {
            let bytes = bincode::serialize(&m).unwrap();
            let back: SessionMsg = bincode::deserialize(&bytes).unwrap();
            assert_eq!(back, m);
        }
    }
}
