//! System audio: host captures what the speakers play (WASAPI loopback) and the
//! controller plays it back. Real implementation on Windows (cpal); stubs on
//! other platforms.
//!
//! CONTRACT (signatures are fixed; bodies are the implementation):
//! * wire format: PCM signed 16-bit little-endian, interleaved;
//! * capture delivers **mono, 24000 Hz**, chunks of ~20 ms (480 samples), to keep
//!   bandwidth low (~384 kbit/s);
//! * `AudioCapture` and `AudioPlayer` are `Send` (the cpal stream lives on its own
//!   thread); dropping them stops the stream.

use anyhow::Result;
use tokio::sync::mpsc;

/// Wire sample rate of captured audio.
#[allow(dead_code)]
const CAPTURE_RATE: u32 = 24_000;
/// Samples per captured chunk (20 ms at 24 kHz, mono).
#[allow(dead_code)]
const CHUNK_SAMPLES: usize = 480;

/// Pure DSP helpers, platform independent (unit-tested on every OS).
#[allow(dead_code)]
mod dsp {
    pub fn i16_to_f32(s: i16) -> f32 {
        s as f32 / 32768.0
    }

    pub fn f32_to_i16(s: f32) -> i16 {
        (s.clamp(-1.0, 1.0) * 32767.0).round() as i16
    }

    /// Average interleaved frames of `ch` channels down to mono.
    pub fn downmix_to_mono(data: &[f32], ch: usize) -> Vec<f32> {
        if ch <= 1 {
            return data.to_vec();
        }
        data.chunks_exact(ch)
            .map(|f| f.iter().sum::<f32>() / ch as f32)
            .collect()
    }

    /// Map interleaved audio from `in_ch` to `out_ch` channels.
    pub fn remap_channels(data: &[f32], in_ch: usize, out_ch: usize) -> Vec<f32> {
        if in_ch == 0 || out_ch == 0 {
            return Vec::new();
        }
        if in_ch == out_ch {
            return data.to_vec();
        }
        if out_ch == 1 {
            return downmix_to_mono(data, in_ch);
        }
        let mut out = Vec::with_capacity(data.len() / in_ch * out_ch);
        for f in data.chunks_exact(in_ch) {
            if in_ch == 1 {
                out.extend(std::iter::repeat(f[0]).take(out_ch));
            } else {
                for c in 0..out_ch {
                    out.push(f[c % in_ch]);
                }
            }
        }
        out
    }

    /// Streaming linear-interpolation resampler for interleaved audio. The
    /// fractional read position and the not-yet-consumed input are kept
    /// between calls, so block boundaries are seamless.
    pub struct Resampler {
        ch: usize,
        step: f64,
        pos: f64,
        buf: Vec<f32>,
    }

    impl Resampler {
        pub fn new(in_rate: u32, out_rate: u32, ch: usize) -> Self {
            Self {
                ch: ch.max(1),
                step: in_rate as f64 / out_rate.max(1) as f64,
                pos: 0.0,
                buf: Vec::new(),
            }
        }

        pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
            let ch = self.ch;
            self.buf.extend_from_slice(&input[..input.len() / ch * ch]);
            let frames = self.buf.len() / ch;
            let mut out = Vec::new();
            while (self.pos as usize) + 1 < frames {
                let i = self.pos as usize;
                let f = (self.pos - i as f64) as f32;
                for c in 0..ch {
                    let a = self.buf[i * ch + c];
                    let b = self.buf[(i + 1) * ch + c];
                    out.push(a + (b - a) * f);
                }
                self.pos += self.step;
            }
            let consumed = (self.pos as usize).min(frames);
            self.buf.drain(..consumed * ch);
            self.pos -= consumed as f64;
            out
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn available() -> bool {
        false
    }

    pub struct AudioCapture {
        _priv: (),
    }

    impl AudioCapture {
        pub fn start(_tx: mpsc::Sender<(u32, u16, Vec<u8>)>) -> Result<Self> {
            anyhow::bail!("audio capture is not available")
        }
    }

    pub struct AudioPlayer {
        _priv: (),
    }

    impl AudioPlayer {
        pub fn new() -> Result<Self> {
            anyhow::bail!("audio playback is not available")
        }

        pub fn push(&mut self, _rate: u32, _channels: u16, _pcm: &[u8]) {}
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use super::dsp::*;
    use anyhow::anyhow;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::Duration;

    pub fn available() -> bool {
        std::panic::catch_unwind(|| cpal::default_host().default_output_device().is_some())
            .unwrap_or(false)
    }

    /// Spawn a thread that builds a stream with `build` and keeps it alive
    /// until `stop` is set (cpal streams may be !Send).
    fn spawn_stream_thread<F>(stop: Arc<AtomicBool>, build: F) -> Result<JoinHandle<()>>
    where
        F: FnOnce() -> Result<cpal::Stream> + Send + 'static,
    {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        let handle = std::thread::Builder::new()
            .name("elo-audio".into())
            .spawn(move || {
                let stream = match build() {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                if let Err(e) = stream.play() {
                    let _ = ready_tx.send(Err(anyhow!("starting stream: {e}")));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                drop(stream);
            })?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(handle),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => Err(anyhow!("timed out starting audio stream")),
        }
    }

    // ---------------------------------------------------------------- capture

    struct CaptureState {
        ch: usize,
        resampler: Resampler,
        pending: Vec<f32>,
        tx: mpsc::Sender<(u32, u16, Vec<u8>)>,
    }

    impl CaptureState {
        fn feed<T>(&mut self, data: &[T])
        where
            T: SizedSample,
            f32: FromSample<T>,
        {
            let f: Vec<f32> = data.iter().map(|&s| f32::from_sample(s)).collect();
            let mono = downmix_to_mono(&f, self.ch);
            let out = self.resampler.process(&mono);
            self.pending.extend_from_slice(&out);
            while self.pending.len() >= CHUNK_SAMPLES {
                let mut bytes = Vec::with_capacity(CHUNK_SAMPLES * 2);
                for &s in &self.pending[..CHUNK_SAMPLES] {
                    bytes.extend_from_slice(&f32_to_i16(s).to_le_bytes());
                }
                self.pending.drain(..CHUNK_SAMPLES);
                // Never block the audio callback: drop the chunk when full.
                let _ = self.tx.try_send((CAPTURE_RATE, 1, bytes));
            }
        }
    }

    fn build_capture_stream<T>(
        device: &cpal::Device,
        config: StreamConfig,
        mut st: CaptureState,
    ) -> Result<cpal::Stream>
    where
        T: SizedSample + Send + 'static,
        f32: FromSample<T>,
    {
        device
            .build_input_stream::<T, _, _>(
                config,
                move |data, _| st.feed(data),
                |_e| {},
                None,
            )
            .map_err(|e| anyhow!("building loopback stream: {e}"))
    }

    /// Running loopback capture. Drop to stop.
    pub struct AudioCapture {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl AudioCapture {
        pub fn start(tx: mpsc::Sender<(u32, u16, Vec<u8>)>) -> Result<Self> {
            let stop = Arc::new(AtomicBool::new(false));
            let thread = spawn_stream_thread(stop.clone(), move || {
                let device = cpal::default_host()
                    .default_output_device()
                    .ok_or_else(|| anyhow!("no default output device"))?;
                // An input stream on an OUTPUT device enables WASAPI loopback.
                let supported = device
                    .default_input_config()
                    .map_err(|e| anyhow!("loopback config: {e}"))?;
                let config = supported.config();
                let st = CaptureState {
                    ch: config.channels.max(1) as usize,
                    resampler: Resampler::new(config.sample_rate, CAPTURE_RATE, 1),
                    pending: Vec::new(),
                    tx,
                };
                match supported.sample_format() {
                    SampleFormat::F32 => build_capture_stream::<f32>(&device, config, st),
                    SampleFormat::I16 => build_capture_stream::<i16>(&device, config, st),
                    SampleFormat::U16 => build_capture_stream::<u16>(&device, config, st),
                    SampleFormat::I32 => build_capture_stream::<i32>(&device, config, st),
                    SampleFormat::F64 => build_capture_stream::<f64>(&device, config, st),
                    SampleFormat::I8 => build_capture_stream::<i8>(&device, config, st),
                    SampleFormat::U8 => build_capture_stream::<u8>(&device, config, st),
                    SampleFormat::U32 => build_capture_stream::<u32>(&device, config, st),
                    other => Err(anyhow!("unsupported sample format {other:?}")),
                }
            })?;
            Ok(Self { stop, thread: Some(thread) })
        }
    }

    impl Drop for AudioCapture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    // ----------------------------------------------------------------- player

    type Ring = Arc<Mutex<VecDeque<f32>>>;

    fn fill_output<T>(ring: &Ring, out: &mut [T])
    where
        T: SizedSample + FromSample<f32>,
    {
        let mut q = match ring.lock() {
            Ok(q) => q,
            Err(p) => p.into_inner(),
        };
        for o in out.iter_mut() {
            *o = T::from_sample(q.pop_front().unwrap_or(0.0));
        }
    }

    fn build_player_stream<T>(
        device: &cpal::Device,
        config: StreamConfig,
        ring: Ring,
    ) -> Result<cpal::Stream>
    where
        T: SizedSample + FromSample<f32> + Send + 'static,
    {
        device
            .build_output_stream::<T, _, _>(
                config,
                move |data, _| fill_output(&ring, data),
                |_e| {},
                None,
            )
            .map_err(|e| anyhow!("building output stream: {e}"))
    }

    /// Plays PCM chunks on the default output device. Drop to stop.
    pub struct AudioPlayer {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
        ring: Ring,
        dev_rate: u32,
        dev_ch: usize,
        max_samples: usize,
        /// Input format the resampler was built for, plus the resampler.
        resampler: Option<((u32, u16), Resampler)>,
    }

    impl AudioPlayer {
        pub fn new() -> Result<Self> {
            let stop = Arc::new(AtomicBool::new(false));
            let ring: Ring = Arc::new(Mutex::new(VecDeque::new()));
            let (fmt_tx, fmt_rx) = std::sync::mpsc::channel::<(u32, usize)>();
            let ring2 = ring.clone();
            let thread = spawn_stream_thread(stop.clone(), move || {
                let device = cpal::default_host()
                    .default_output_device()
                    .ok_or_else(|| anyhow!("no default output device"))?;
                let supported = device
                    .default_output_config()
                    .map_err(|e| anyhow!("output config: {e}"))?;
                let config = supported.config();
                let _ = fmt_tx.send((config.sample_rate, config.channels.max(1) as usize));
                match supported.sample_format() {
                    SampleFormat::F32 => build_player_stream::<f32>(&device, config, ring2),
                    SampleFormat::I16 => build_player_stream::<i16>(&device, config, ring2),
                    SampleFormat::U16 => build_player_stream::<u16>(&device, config, ring2),
                    SampleFormat::I32 => build_player_stream::<i32>(&device, config, ring2),
                    SampleFormat::F64 => build_player_stream::<f64>(&device, config, ring2),
                    SampleFormat::I8 => build_player_stream::<i8>(&device, config, ring2),
                    SampleFormat::U8 => build_player_stream::<u8>(&device, config, ring2),
                    SampleFormat::U32 => build_player_stream::<u32>(&device, config, ring2),
                    other => Err(anyhow!("unsupported sample format {other:?}")),
                }
            })?;
            let (dev_rate, dev_ch) = fmt_rx
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| anyhow!("output format unavailable"))?;
            // ~250 ms of audio in device format.
            let max_samples = dev_rate as usize / 4 * dev_ch;
            Ok(Self {
                stop,
                thread: Some(thread),
                ring,
                dev_rate,
                dev_ch,
                max_samples,
                resampler: None,
            })
        }

        pub fn push(&mut self, rate: u32, channels: u16, pcm: &[u8]) {
            if rate == 0 || channels == 0 {
                return;
            }
            let samples: Vec<f32> = pcm
                .chunks_exact(2)
                .map(|b| i16_to_f32(i16::from_le_bytes([b[0], b[1]])))
                .collect();
            let mapped = remap_channels(&samples, channels as usize, self.dev_ch);
            let key = (rate, channels);
            let (dev_rate, dev_ch) = (self.dev_rate, self.dev_ch);
            let rs = match &mut self.resampler {
                Some((k, rs)) if *k == key => rs,
                slot => {
                    *slot = Some((key, Resampler::new(rate, dev_rate, dev_ch)));
                    &mut slot.as_mut().unwrap().1
                }
            };
            let out = rs.process(&mapped);
            let mut q = match self.ring.lock() {
                Ok(q) => q,
                Err(p) => p.into_inner(),
            };
            q.extend(out);
            if q.len() > self.max_samples {
                // Drop the oldest whole frames to bound latency.
                let excess = (q.len() - self.max_samples) / self.dev_ch * self.dev_ch;
                q.drain(..excess);
            }
        }
    }

    impl Drop for AudioPlayer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
}

pub use imp::{available, AudioCapture, AudioPlayer};

#[cfg(test)]
mod tests {
    use super::dsp::*;

    fn sine(rate: u32, freq: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn resample_48k_to_24k_halves_length() {
        let mut r = Resampler::new(48_000, 24_000, 1);
        let out = r.process(&sine(48_000, 440.0, 4800));
        assert!((out.len() as i64 - 2400).abs() <= 1, "len {}", out.len());
    }

    #[test]
    fn resample_keeps_state_across_blocks() {
        let input = sine(48_000, 440.0, 4800);
        let mut r = Resampler::new(48_000, 24_000, 1);
        let mut out = Vec::new();
        for c in input.chunks(333) {
            out.extend(r.process(c));
        }
        assert!((out.len() as i64 - 2400).abs() <= 1, "len {}", out.len());
    }

    #[test]
    fn resample_sine_keeps_amplitude() {
        let mut r = Resampler::new(48_000, 24_000, 1);
        let out = r.process(&sine(48_000, 440.0, 4800));
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.5).abs() < 0.03, "peak {peak}");
    }

    #[test]
    fn upsample_doubles_length() {
        let mut r = Resampler::new(24_000, 48_000, 2);
        let out = r.process(&vec![0.25; 2 * 480]);
        assert!((out.len() as i64 - 2 * 960).abs() <= 4, "len {}", out.len());
    }

    #[test]
    fn stereo_to_mono_averages() {
        assert_eq!(downmix_to_mono(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
    }

    #[test]
    fn remap_mono_to_stereo_duplicates() {
        assert_eq!(remap_channels(&[0.1, 0.2], 1, 2), vec![0.1, 0.1, 0.2, 0.2]);
    }

    #[test]
    fn i16_round_trip() {
        for s in [-32768i16, -12345, -1, 0, 1, 777, 32767] {
            let back = f32_to_i16(i16_to_f32(s));
            assert!((back as i32 - s as i32).abs() <= 1, "{s} -> {back}");
        }
    }
}
