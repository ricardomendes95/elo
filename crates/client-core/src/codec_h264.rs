//! H.264 video codec (OpenH264), used instead of the tile codec when both
//! peers support it — far better compression for real screens.
//!
//! CONTRACT (signatures are fixed; bodies are the implementation):
//! * frames in/out are BGRA, row-major, `width*height*4` bytes;
//! * the bitstream is Annex-B (start-code delimited NAL units);
//! * odd sizes must work (pad internally to even for YUV 4:2:0, crop on decode).

use std::sync::OnceLock;

use anyhow::{anyhow, bail, Result};
use openh264::decoder::Decoder;
use openh264::encoder::{
    BitRate, Encoder, EncoderConfig, FrameRate, FrameType, RateControlMode, UsageType,
};
use openh264::formats::YUVSource;

/// Whether H.264 is usable in this build (probed once: a tiny encode/decode round trip).
pub fn available() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| {
        std::panic::catch_unwind(|| {
            let probe = || -> Result<bool> {
                let (w, h) = (64u32, 64u32);
                let mut enc = H264Encoder::new(w, h, 30, 500)?;
                let mut dec = H264Decoder::new()?;
                let frame = vec![128u8; (w * h * 4) as usize];
                let (bytes, _) = enc.encode(&frame, true)?;
                Ok(!bytes.is_empty() && dec.decode(&bytes)?.is_some())
            };
            probe().unwrap_or(false)
        })
        .unwrap_or(false)
    })
}

/// Planar I420 frame owned by the encoder, padded to even dimensions.
struct Yuv420 {
    w: usize,
    h: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl YUVSource for Yuv420 {
    fn dimensions(&self) -> (usize, usize) {
        (self.w, self.h)
    }
    fn strides(&self) -> (usize, usize, usize) {
        (self.w, self.w / 2, self.w / 2)
    }
    fn y(&self) -> &[u8] {
        &self.y
    }
    fn u(&self) -> &[u8] {
        &self.u
    }
    fn v(&self) -> &[u8] {
        &self.v
    }
}

pub struct H264Encoder {
    enc: Encoder,
    width: usize,
    height: usize,
    yuv: Yuv420,
}

// `Encoder` already declares itself Send; this struct only adds plain Vecs.
impl H264Encoder {
    /// Create an encoder for `width`×`height` at roughly `fps` / `bitrate_kbps`.
    ///
    /// Odd sizes are padded (edge replication) to even dimensions internally;
    /// the stream then carries the even size.
    pub fn new(width: u32, height: u32, fps: u32, bitrate_kbps: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            bail!("invalid frame size {width}x{height}");
        }
        let (w, h) = (width as usize, height as usize);
        let (pw, ph) = ((w + 1) & !1, (h + 1) & !1);
        let cfg = EncoderConfig::new()
            .bitrate(BitRate::from_bps(bitrate_kbps.max(1).saturating_mul(1000)))
            .max_frame_rate(FrameRate::from_hz(fps.max(1) as f32))
            .usage_type(UsageType::ScreenContentRealTime)
            .rate_control_mode(RateControlMode::Bitrate)
            .skip_frames(true);
        let enc = Encoder::with_api_config(openh264::OpenH264API::from_source(), cfg)
            .map_err(|e| anyhow!("openh264 encoder init: {e}"))?;
        Ok(Self {
            enc,
            width: w,
            height: h,
            yuv: Yuv420 {
                w: pw,
                h: ph,
                y: vec![0; pw * ph],
                u: vec![128; pw * ph / 4],
                v: vec![128; pw * ph / 4],
            },
        })
    }

    /// Encode one BGRA frame. Returns the Annex-B bytes and whether it is an
    /// IDR/key frame. `force_key` requests an IDR. Empty bytes = frame skipped.
    pub fn encode(&mut self, bgra: &[u8], force_key: bool) -> Result<(Vec<u8>, bool)> {
        if bgra.len() != self.width * self.height * 4 {
            bail!(
                "bad frame length {} (expected {})",
                bgra.len(),
                self.width * self.height * 4
            );
        }
        bgra_to_i420(bgra, self.width, self.height, &mut self.yuv);
        if force_key {
            self.enc.force_intra_frame();
        }
        let bs = self
            .enc
            .encode(&self.yuv)
            .map_err(|e| anyhow!("openh264 encode: {e}"))?;
        let key = matches!(bs.frame_type(), FrameType::IDR | FrameType::I);
        if matches!(bs.frame_type(), FrameType::Skip | FrameType::Invalid) {
            return Ok((Vec::new(), false));
        }
        let mut out = Vec::new();
        bs.write_vec(&mut out);
        Ok((out, key))
    }
}

/// BGRA -> I420 (BT.601 limited range). 4:2:0 chroma is the average of each
/// 2x2 block; odd right/bottom edges are padded by replicating the last pixel.
fn bgra_to_i420(bgra: &[u8], w: usize, h: usize, out: &mut Yuv420) {
    let pw = out.w;
    let ph = out.h;
    let stride = w * 4;
    for cy in 0..ph / 2 {
        let r0 = (cy * 2).min(h - 1);
        let r1 = (cy * 2 + 1).min(h - 1);
        let row0 = &bgra[r0 * stride..r0 * stride + stride];
        let row1 = &bgra[r1 * stride..r1 * stride + stride];
        for cx in 0..pw / 2 {
            let x0 = (cx * 2).min(w - 1) * 4;
            let x1 = (cx * 2 + 1).min(w - 1) * 4;
            let mut sr = 0i32;
            let mut sg = 0i32;
            let mut sb = 0i32;
            for (row, x, dy, dx) in [
                (row0, x0, 0, 0),
                (row0, x1, 0, 1),
                (row1, x0, 1, 0),
                (row1, x1, 1, 1),
            ] {
                let b = row[x] as i32;
                let g = row[x + 1] as i32;
                let r = row[x + 2] as i32;
                out.y[(cy * 2 + dy) * pw + cx * 2 + dx] =
                    (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
                sr += r;
                sg += g;
                sb += b;
            }
            let (r, g, b) = ((sr + 2) >> 2, (sg + 2) >> 2, (sb + 2) >> 2);
            out.u[cy * (pw / 2) + cx] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128) as u8;
            out.v[cy * (pw / 2) + cx] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128) as u8;
        }
    }
}

pub struct H264Decoder {
    dec: Decoder,
}

impl H264Decoder {
    pub fn new() -> Result<Self> {
        let dec = Decoder::new().map_err(|e| anyhow!("openh264 decoder init: {e}"))?;
        Ok(Self { dec })
    }

    /// Decode Annex-B bytes; returns `(width, height, bgra)` when a picture is ready.
    ///
    /// The returned size is the decoded (even) size of the stream: for an odd
    /// source size the encoder pads by one pixel and the stream does not carry
    /// the original size, so callers that know it should crop.
    pub fn decode(&mut self, data: &[u8]) -> Result<Option<(u32, u32, Vec<u8>)>> {
        let pic = self
            .dec
            .decode(data)
            .map_err(|e| anyhow!("openh264 decode: {e}"))?;
        let Some(pic) = pic else { return Ok(None) };
        let (w, h) = pic.dimensions();
        let (sy, su, sv) = pic.strides();
        let (y, u, v) = (pic.y(), pic.u(), pic.v());
        let mut out = vec![0u8; w * h * 4];
        for row in 0..h {
            let yr = &y[row * sy..row * sy + w];
            let ur = &u[(row / 2) * su..(row / 2) * su + w / 2 + (w & 1)];
            let vr = &v[(row / 2) * sv..(row / 2) * sv + w / 2 + (w & 1)];
            let dst = &mut out[row * w * 4..(row + 1) * w * 4];
            for x in 0..w {
                let c = (yr[x] as i32 - 16).max(0) * 298;
                let d = ur[x / 2] as i32 - 128;
                let e = vr[x / 2] as i32 - 128;
                let r = (c + 409 * e + 128) >> 8;
                let g = (c - 100 * d - 208 * e + 128) >> 8;
                let b = (c + 516 * d + 128) >> 8;
                let p = &mut dst[x * 4..x * 4 + 4];
                p[0] = b.clamp(0, 255) as u8;
                p[1] = g.clamp(0, 255) as u8;
                p[2] = r.clamp(0, 255) as u8;
                p[3] = 255;
            }
        }
        Ok(Some((w as u32, h as u32, out)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: usize, h: usize, t: usize) -> Vec<u8> {
        let mut f = vec![255u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                f[i] = (x * 255 / w) as u8;
                f[i + 1] = (y * 255 / h) as u8;
                f[i + 2] = 128;
            }
        }
        let bx = 10 + t * 8;
        for y in 50..90.min(h) {
            for x in bx..(bx + 40).min(w) {
                let i = (y * w + x) * 4;
                f[i..i + 3].copy_from_slice(&[20, 40, 230]);
            }
        }
        f
    }

    fn roundtrip(w: usize, h: usize) {
        assert!(available());
        let mut enc = H264Encoder::new(w as u32, h as u32, 30, 4000).unwrap();
        let mut dec = H264Decoder::new().unwrap();
        let (pw, ph) = ((w + 1) & !1, (h + 1) & !1);
        let mut decoded = 0;
        for t in 0..10 {
            let src = frame(w, h, t);
            let (bytes, key) = enc.encode(&src, t == 6).unwrap();
            assert!(!bytes.is_empty());
            if t == 0 || t == 6 {
                assert!(key, "frame {t} should be a keyframe");
            }
            if let Some((dw, dh, bgra)) = dec.decode(&bytes).unwrap() {
                decoded += 1;
                assert_eq!((dw as usize, dh as usize), (pw, ph));
                let mut sum = [0u64; 3];
                for y in 0..h {
                    for x in 0..w {
                        for c in 0..3 {
                            let a = src[(y * w + x) * 4 + c] as i32;
                            let b = bgra[(y * pw + x) * 4 + c] as i32;
                            sum[c] += (a - b).unsigned_abs() as u64;
                        }
                    }
                }
                for s in sum {
                    let mae = s as f64 / (w * h) as f64;
                    assert!(mae < 12.0, "MAE too high: {mae}");
                }
            }
        }
        assert_eq!(decoded, 10);
    }

    #[test]
    fn roundtrip_even() {
        roundtrip(320, 240);
    }

    #[test]
    fn roundtrip_odd() {
        roundtrip(321, 241);
    }
}
