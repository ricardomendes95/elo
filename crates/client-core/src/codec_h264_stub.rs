//! Stand-in for [`codec_h264`](crate::codec_h264) when the `h264` feature is
//! off (e.g. the Android library, which decodes H.264 with `MediaCodec`).
//! Same API; [`available`] is `false` and constructors fail, so the session
//! loops fall back to the tile codec or to the raw-media events.

use anyhow::{bail, Result};

/// Always `false`: this build has no OpenH264.
pub fn available() -> bool {
    false
}

pub struct H264Encoder;

impl H264Encoder {
    pub fn new(_width: u32, _height: u32, _fps: u32, _bitrate_kbps: u32) -> Result<Self> {
        bail!("H.264 is not compiled into this build (feature `h264`)")
    }
    pub fn encode(&mut self, _bgra: &[u8], _force_key: bool) -> Result<(Vec<u8>, bool)> {
        bail!("H.264 is not compiled into this build (feature `h264`)")
    }
}

pub struct H264Decoder;

impl H264Decoder {
    pub fn new() -> Result<Self> {
        bail!("H.264 is not compiled into this build (feature `h264`)")
    }
    pub fn decode(&mut self, _data: &[u8]) -> Result<Option<(u32, u32, Vec<u8>)>> {
        bail!("H.264 is not compiled into this build (feature `h264`)")
    }
}
