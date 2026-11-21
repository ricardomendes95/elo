//! Elo wire protocol: the control handshake spoken with the relay, plus a
//! length-prefixed frame codec shared by every component.
//!
//! Design note: the relay is *untrusted* for confidentiality. It only ever
//! parses [`ControlMsg`] during rendezvous; once two peers are paired it
//! copies opaque bytes between them. The actual session is end-to-end
//! encrypted between the two clients (handshake lives in the client crate),
//! so the relay never sees screen data, input, files or chat in the clear.

use serde::{Deserialize, Serialize};

pub mod session;
pub use session::*;

/// A device address shown to the user. Nine digits, grouped as `123 456 789`.
/// Generated randomly by a host when it first registers.
pub type DeviceId = String;

/// Public key fingerprint material (raw 32-byte X25519/Ed25519 key). The relay
/// forwards it verbatim so each side can pin/verify the other; the relay does
/// not and cannot use it to decrypt anything.
pub type PubKey = Vec<u8>;

/// Role a connection takes on the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// The machine being controlled; waits to be reached by its `DeviceId`.
    Host,
    /// The machine doing the controlling; asks to reach a `DeviceId`.
    Controller,
}

/// Messages exchanged with the relay before pairing. After a successful
/// pairing the socket carries only opaque session frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlMsg {
    /// Host announces itself and stays available under `device_id`.
    RegisterHost { device_id: DeviceId, pubkey: PubKey },
    /// Relay confirms the host is registered and waiting.
    Registered,
    /// Controller asks to be routed to `target_id`.
    Connect { target_id: DeviceId, pubkey: PubKey },
    /// Relay tells both sides they are paired, handing over the peer's pubkey.
    /// Everything after this message on the socket is the E2E session.
    Paired { peer_pubkey: PubKey },
    /// Relay rejects the request (unknown id, busy, malformed, ...).
    Error { reason: String },

    // ---- directory (short numeric id -> P2P node id) ----
    /// Host publishes its short id -> iroh node id and stays connected; the
    /// mapping lives while this connection is open. Relay replies `Registered`.
    Register { device_id: DeviceId, node_id: String },
    /// Controller resolves a short id to a node id.
    Resolve { device_id: DeviceId },
    /// Relay's answer to `Resolve`.
    Resolved { node_id: String },
}

/// Largest control or session frame the codec will accept (16 MiB). Bounds
/// memory and makes a hostile length prefix cheap to reject.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("frame of {0} bytes exceeds MAX_FRAME ({MAX_FRAME})")]
    TooLarge(usize),
    #[error("serialize/deserialize failed: {0}")]
    Serde(#[from] bincode::Error),
}

/// Serialize a control message into a length-prefixed frame
/// (`u32` big-endian length, then the bincode body).
pub fn encode(msg: &ControlMsg) -> Result<Vec<u8>, CodecError> {
    let body = bincode::serialize(msg)?;
    if body.len() > MAX_FRAME {
        return Err(CodecError::TooLarge(body.len()));
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decode a control message from a frame body (length prefix already stripped).
pub fn decode(body: &[u8]) -> Result<ControlMsg, CodecError> {
    if body.len() > MAX_FRAME {
        return Err(CodecError::TooLarge(body.len()));
    }
    Ok(bincode::deserialize(body)?)
}

/// Format a raw 9-digit id as `123 456 789` for display.
pub fn format_id(id: &DeviceId) -> String {
    id.as_bytes()
        .chunks(3)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_roundtrip() {
        let msgs = [
            ControlMsg::RegisterHost {
                device_id: "123456789".into(),
                pubkey: vec![1, 2, 3, 4],
            },
            ControlMsg::Registered,
            ControlMsg::Connect {
                target_id: "987654321".into(),
                pubkey: vec![9, 9],
            },
            ControlMsg::Paired {
                peer_pubkey: vec![7; 32],
            },
            ControlMsg::Error {
                reason: "unknown id".into(),
            },
        ];
        for m in msgs {
            let frame = encode(&m).unwrap();
            let len = u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize;
            assert_eq!(len, frame.len() - 4);
            assert_eq!(decode(&frame[4..]).unwrap(), m);
        }
    }

    #[test]
    fn rejects_oversize() {
        let huge = vec![0u8; MAX_FRAME + 1];
        assert!(matches!(decode(&huge), Err(CodecError::TooLarge(_))));
    }

    #[test]
    fn id_formatting() {
        assert_eq!(format_id(&"123456789".to_string()), "123 456 789");
    }
}
