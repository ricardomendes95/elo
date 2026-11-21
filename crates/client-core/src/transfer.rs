//! File-transfer bookkeeping used by both session loops. The wire messages
//! live in `elo_protocol` (`FileOffer`/`FileAccept`/`FileChunk`/`FileDone`).
//! This module tracks an in-progress *incoming* transfer: it validates offsets
//! and sizes so a peer cannot write outside the declared length.

use anyhow::{bail, Result};
use elo_protocol::TransferId;

/// Preferred chunk payload size for outgoing files (bytes).
pub const CHUNK: usize = 60 * 1024;

/// An incoming file being reassembled in memory.
pub struct IncomingFile {
    pub id: TransferId,
    pub name: String,
    pub size: u64,
    buf: Vec<u8>,
}

impl IncomingFile {
    pub fn new(id: TransferId, name: String, size: u64) -> Self {
        Self { id, name, size, buf: Vec::with_capacity(size.min(16 * 1024 * 1024) as usize) }
    }

    /// Accept a chunk. Enforces contiguous, in-bounds writes.
    pub fn push(&mut self, offset: u64, data: &[u8]) -> Result<()> {
        if offset != self.buf.len() as u64 {
            bail!("non-contiguous chunk: got offset {offset}, expected {}", self.buf.len());
        }
        if offset + data.len() as u64 > self.size {
            bail!("chunk overruns declared size {}", self.size);
        }
        self.buf.extend_from_slice(data);
        Ok(())
    }

    pub fn is_complete(&self) -> bool {
        self.buf.len() as u64 == self.size
    }

    /// Consume the finished file's bytes.
    pub fn finish(self) -> Result<Vec<u8>> {
        if !self.is_complete() {
            bail!("file {} incomplete: {}/{} bytes", self.name, self.buf.len(), self.size);
        }
        Ok(self.buf)
    }
}

/// Split a buffer into `(offset, slice)` chunks of at most [`CHUNK`] bytes.
pub fn chunks(data: &[u8]) -> impl Iterator<Item = (u64, &[u8])> {
    data.chunks(CHUNK)
        .enumerate()
        .map(|(i, c)| ((i * CHUNK) as u64, c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reassembles_in_order() {
        let mut f = IncomingFile::new(1, "x".into(), 5);
        f.push(0, &[1, 2, 3]).unwrap();
        f.push(3, &[4, 5]).unwrap();
        assert!(f.is_complete());
        assert_eq!(f.finish().unwrap(), vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn rejects_gap_and_overrun() {
        let mut f = IncomingFile::new(1, "x".into(), 3);
        assert!(f.push(1, &[1]).is_err()); // gap
        assert!(f.push(0, &[1, 2, 3, 4]).is_err()); // overrun
    }

    #[test]
    fn chunking_covers_everything() {
        let data: Vec<u8> = (0..(CHUNK as u32 * 2 + 7)).map(|i| i as u8).collect();
        let mut seen = 0u64;
        let mut reassembled = Vec::new();
        for (off, c) in chunks(&data) {
            assert_eq!(off, seen);
            seen += c.len() as u64;
            reassembled.extend_from_slice(c);
        }
        assert_eq!(reassembled, data);
    }
}
