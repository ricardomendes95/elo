//! Time-based one-time passwords (TOTP, RFC 6238: HMAC-SHA1, 30 s, 6 digits),
//! compatible with Google Authenticator, Microsoft Authenticator, Authy, etc.

use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

/// Step length in seconds.
pub const PERIOD: u64 = 30;

/// A fresh random 160-bit secret.
pub fn generate_secret() -> Vec<u8> {
    (0..20).map(|_| rand::random::<u8>()).collect()
}

/// Base32 (no padding), the form authenticator apps accept.
pub fn secret_to_base32(secret: &[u8]) -> String {
    BASE32_NOPAD.encode(secret)
}

/// Parse a base32 secret (spaces/case ignored).
pub fn secret_from_base32(s: &str) -> Option<Vec<u8>> {
    let clean: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '=')
        .collect::<String>()
        .to_uppercase();
    BASE32_NOPAD.decode(clean.as_bytes()).ok()
}

fn hotp(secret: &[u8], counter: u64, digits: u32) -> u32 {
    let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&counter.to_be_bytes());
    let r = mac.finalize().into_bytes();
    let o = (r[r.len() - 1] & 0x0f) as usize;
    let bin = ((r[o] as u32 & 0x7f) << 24)
        | ((r[o + 1] as u32) << 16)
        | ((r[o + 2] as u32) << 8)
        | (r[o + 3] as u32);
    bin % 10u32.pow(digits)
}

/// The 6-digit code for a given unix time.
pub fn code_at(secret: &[u8], unix: u64) -> String {
    format!("{:06}", hotp(secret, unix / PERIOD, 6))
}

/// Current unix time.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Verify a code, accepting one step of clock drift either way.
pub fn verify(secret: &[u8], code: &str, unix: u64) -> bool {
    let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    if code.len() != 6 {
        return false;
    }
    [-1i64, 0, 1].iter().any(|d| {
        let t = (unix as i64 + d * PERIOD as i64).max(0) as u64;
        code_at(secret, t) == code
    })
}

/// `otpauth://` URI for enrolling in an authenticator app (render as a QR code).
pub fn otpauth_uri(secret: &[u8], account: &str, issuer: &str) -> String {
    let enc = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() || "-._~".contains(c) { c.to_string() } else { format!("%{:02X}", c as u32 & 0xff) })
            .collect()
    };
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits=6&period=30",
        enc(issuer),
        enc(account),
        secret_to_base32(secret),
        enc(issuer)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 6238 appendix B (SHA1, secret "12345678901234567890"), last 6 digits.
    #[test]
    fn rfc6238_vectors() {
        let s = b"12345678901234567890";
        assert_eq!(code_at(s, 59), "287082");
        assert_eq!(code_at(s, 1111111109), "081804");
        assert_eq!(code_at(s, 1234567890), "005924");
        assert_eq!(code_at(s, 2000000000), "279037");
    }

    #[test]
    fn verify_accepts_drift_and_rejects_wrong() {
        let s = generate_secret();
        let t = 1_700_000_000;
        assert!(verify(&s, &code_at(&s, t), t));
        assert!(verify(&s, &code_at(&s, t - 30), t));
        assert!(!verify(&s, "000000", t) || code_at(&s, t) == "000000");
        assert!(!verify(&s, "12345", t));
    }

    #[test]
    fn base32_roundtrip() {
        let s = generate_secret();
        assert_eq!(secret_from_base32(&secret_to_base32(&s)).unwrap(), s);
    }
}
