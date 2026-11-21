//! Screen codec: tile-based delta compression.
//!
//! The frame is divided into fixed `TILE`×`TILE` tiles. A key frame emits
//! every tile; a delta frame emits only the tiles whose BGRA bytes differ from
//! the previously encoded frame. Each emitted tile becomes a [`Rect`] whose
//! `data` is zlib-DEFLATE of the raw BGRA pixels of that tile (row-major,
//! `w*h*4` bytes inflated). The decoder keeps a canvas and applies rects onto
//! it.
//!
//! This is a real, dependency-light codec (not as efficient as VP8/H264, which
//! can later replace it behind this same `Encoder`/`Decoder` API).

use std::io::{Read, Write};

use anyhow::{bail, Result};
use elo_protocol::{Rect, VideoFrame};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;

/// Tile edge length in pixels.
pub const TILE: u32 = 64;

/// Bytes per pixel (BGRA).
const BPP: usize = 4;

/// Extract the raw BGRA bytes of a tile from a full framebuffer, row-major.
fn extract_tile(bgra: &[u8], width: u32, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    let width = width as usize;
    let (x, y, w, h) = (x as usize, y as usize, w as usize, h as usize);
    let mut out = Vec::with_capacity(w * h * BPP);
    for row in 0..h {
        let start = ((y + row) * width + x) * BPP;
        out.extend_from_slice(&bgra[start..start + w * BPP]);
    }
    out
}

/// zlib-DEFLATE a buffer.
fn deflate(raw: &[u8]) -> Result<Vec<u8>> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(raw)?;
    Ok(enc.finish()?)
}

/// zlib-INFLATE a buffer into exactly `expected` bytes.
fn inflate(data: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut dec = ZlibDecoder::new(data);
    let mut out = Vec::with_capacity(expected);
    dec.read_to_end(&mut out)?;
    if out.len() != expected {
        bail!("inflated size {} != expected {}", out.len(), expected);
    }
    Ok(out)
}

/// Iterate tile origins and sizes covering a `width`×`height` frame.
fn tiles(width: u32, height: u32) -> impl Iterator<Item = (u32, u32, u32, u32)> {
    let cols = width.div_ceil(TILE);
    let rows = height.div_ceil(TILE);
    (0..rows).flat_map(move |ty| {
        (0..cols).map(move |tx| {
            let x = tx * TILE;
            let y = ty * TILE;
            let w = TILE.min(width - x);
            let h = TILE.min(height - y);
            (x, y, w, h)
        })
    })
}

/// Stateful encoder. Holds the previous frame to compute deltas.
pub struct Encoder {
    width: u32,
    height: u32,
    prev: Option<Vec<u8>>,
    seq: u64,
    force_key: bool,
}

impl Encoder {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            prev: None,
            seq: 0,
            force_key: true,
        }
    }

    /// Make the next `encode` emit a full key frame.
    pub fn force_keyframe(&mut self) {
        self.force_key = true;
    }

    /// Encode one BGRA framebuffer (`width*height*4` bytes).
    pub fn encode(&mut self, bgra: &[u8]) -> Result<VideoFrame> {
        let expected = self.width as usize * self.height as usize * BPP;
        if bgra.len() != expected {
            bail!(
                "framebuffer size {} != expected {} ({}x{} BGRA)",
                bgra.len(),
                expected,
                self.width,
                self.height
            );
        }

        self.seq += 1;
        let key = self.force_key || self.prev.is_none();

        let mut rects = Vec::new();
        for (x, y, w, h) in tiles(self.width, self.height) {
            let tile = extract_tile(bgra, self.width, x, y, w, h);
            let changed = match &self.prev {
                Some(prev) if !key => {
                    extract_tile(prev, self.width, x, y, w, h) != tile
                }
                _ => true,
            };
            if changed {
                rects.push(Rect {
                    x,
                    y,
                    w,
                    h,
                    data: deflate(&tile)?,
                });
            }
        }

        self.prev = Some(bgra.to_vec());
        self.force_key = false;

        Ok(VideoFrame {
            width: self.width,
            height: self.height,
            key,
            seq: self.seq,
            rects,
        })
    }
}

/// Stateful decoder. Holds the reconstructed canvas.
pub struct Decoder {
    width: u32,
    height: u32,
    canvas: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            width: 0,
            height: 0,
            canvas: Vec::new(),
        }
    }

    /// Dimensions of the current canvas (0,0 before the first key frame).
    pub fn dims(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Apply a frame and return the full BGRA canvas (`width*height*4` bytes).
    pub fn apply(&mut self, frame: &VideoFrame) -> Result<&[u8]> {
        if frame.key || self.canvas.is_empty() {
            self.width = frame.width;
            self.height = frame.height;
            self.canvas = vec![0u8; frame.width as usize * frame.height as usize * BPP];
        } else if frame.width != self.width || frame.height != self.height {
            bail!(
                "delta frame {}x{} does not match canvas {}x{}",
                frame.width,
                frame.height,
                self.width,
                self.height
            );
        }

        let cw = self.width as usize;
        let ch = self.height as usize;
        for rect in &frame.rects {
            let (x, y, w, h) = (
                rect.x as usize,
                rect.y as usize,
                rect.w as usize,
                rect.h as usize,
            );
            if x + w > cw || y + h > ch {
                bail!(
                    "rect ({x},{y},{w},{h}) out of canvas bounds {cw}x{ch}"
                );
            }
            let raw = inflate(&rect.data, w * h * BPP)?;
            for row in 0..h {
                let src = row * w * BPP;
                let dst = ((y + row) * cw + x) * BPP;
                self.canvas[dst..dst + w * BPP]
                    .copy_from_slice(&raw[src..src + w * BPP]);
            }
        }

        Ok(&self.canvas)
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random BGRA buffer.
    fn make_buf(width: u32, height: u32, seed: u64) -> Vec<u8> {
        let n = width as usize * height as usize * BPP;
        let mut buf = vec![0u8; n];
        let mut state = seed.wrapping_add(0x9e3779b97f4a7c15);
        for b in buf.iter_mut() {
            // xorshift64
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *b = (state & 0xff) as u8;
        }
        buf
    }

    #[test]
    fn codec_keyframe_roundtrip() {
        let (w, h) = (200u32, 150u32); // not a multiple of TILE
        let buf = make_buf(w, h, 1);

        let mut enc = Encoder::new(w, h);
        let frame = enc.encode(&buf).unwrap();
        assert!(frame.key);
        assert_eq!(frame.seq, 1);

        let mut dec = Decoder::new();
        let out = dec.apply(&frame).unwrap().to_vec();
        assert_eq!(dec.dims(), (w, h));
        assert_eq!(out, buf);
    }

    #[test]
    fn codec_delta_roundtrip() {
        let (w, h) = (200u32, 150u32);
        let mut buf = make_buf(w, h, 2);

        let mut enc = Encoder::new(w, h);
        let mut dec = Decoder::new();

        let key = enc.encode(&buf).unwrap();
        assert!(key.key);
        dec.apply(&key).unwrap();

        // Change a small region: a 10x10 block at (5, 5).
        for row in 5..15 {
            for col in 5..15 {
                let i = (row * w as usize + col) * BPP;
                buf[i] = buf[i].wrapping_add(123);
                buf[i + 1] = buf[i + 1].wrapping_add(45);
                buf[i + 2] = buf[i + 2].wrapping_add(67);
            }
        }

        let delta = enc.encode(&buf).unwrap();
        assert!(!delta.key);
        assert_eq!(delta.seq, 2);
        // The 10x10 change at (5,5) touches exactly one tile.
        assert_eq!(delta.rects.len(), 1);

        let out = dec.apply(&delta).unwrap();
        assert_eq!(out, &buf[..]);
    }

    #[test]
    fn codec_force_keyframe() {
        let (w, h) = (128u32, 128u32);
        let buf = make_buf(w, h, 3);

        let mut enc = Encoder::new(w, h);
        enc.encode(&buf).unwrap(); // first is key
        let d = enc.encode(&buf).unwrap();
        assert!(!d.key); // unchanged -> delta, zero rects
        assert_eq!(d.rects.len(), 0);

        enc.force_keyframe();
        let k = enc.encode(&buf).unwrap();
        assert!(k.key);
        // key frame emits every tile: 2x2 tiles here
        assert_eq!(k.rects.len(), 4);
    }
}
