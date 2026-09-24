//! Shared-memory ring of RGBA8 frames, written by the stage and read by tools.
//!
//! Layout: `FRAME_SLOTS` slots, each a 64-byte header (an atomic sequence
//! number) followed by `stride * height` bytes of pixels. The writer zeroes
//! the sequence number while writing and publishes the real one afterwards;
//! the reader checks it before and after copying, so a slot overwritten
//! mid-read is detected and dropped instead of shown torn.

use memmap2::MmapMut;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub const FRAME_SLOTS: u32 = 3;
const SLOT_HEADER: usize = 64;

pub struct FrameRing {
    map: MmapMut,
    stride: usize,
    height: usize,
}

impl FrameRing {
    /// Creates (or truncates) the backing file and maps it. Stage side.
    pub fn create(path: &Path, stride: u32, height: u32) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        let len = Self::file_len(stride, height);
        file.set_len(len as u64)?;
        Self::map(&file, stride, height)
    }

    /// Maps an existing ring. Tools side.
    pub fn open(path: &Path, stride: u32, height: u32) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        if file.metadata()?.len() < Self::file_len(stride, height) as u64 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "frame ring too small"));
        }
        Self::map(&file, stride, height)
    }

    fn map(file: &File, stride: u32, height: u32) -> io::Result<Self> {
        // SAFETY: the file is private to the two cooperating processes, and
        // all concurrent access to shared bytes goes through the seqlock.
        let map = unsafe { MmapMut::map_mut(file)? };
        Ok(Self { map, stride: stride as usize, height: height as usize })
    }

    fn file_len(stride: u32, height: u32) -> usize {
        FRAME_SLOTS as usize * (SLOT_HEADER + stride as usize * height as usize)
    }

    fn slot_offset(&self, slot: u32) -> usize {
        assert!(slot < FRAME_SLOTS, "slot {slot} out of range");
        slot as usize * (SLOT_HEADER + self.stride * self.height)
    }

    fn seq(&self, slot: u32) -> &AtomicU64 {
        let ptr = self.map[self.slot_offset(slot)..].as_ptr() as *mut u64;
        // SAFETY: slot offsets are 64-byte aligned within a page-aligned
        // mapping, and the header bytes are only accessed atomically.
        unsafe { AtomicU64::from_ptr(ptr) }
    }

    pub fn frame_len(&self) -> usize {
        self.stride * self.height
    }

    /// Copies `pixels` (exactly [`Self::frame_len`] bytes) into `slot` and
    /// publishes it as `seq`, which must be non-zero.
    pub fn write(&mut self, slot: u32, seq: u64, pixels: &[u8]) {
        assert_ne!(seq, 0, "seq 0 means 'being written'");
        assert_eq!(pixels.len(), self.frame_len());
        self.seq(slot).store(0, Ordering::Release);
        let start = self.slot_offset(slot) + SLOT_HEADER;
        let len = self.frame_len();
        self.map[start..start + len].copy_from_slice(pixels);
        self.seq(slot).store(seq, Ordering::Release);
    }

    /// Copies the frame in `slot` if it is still the one published as `seq`.
    pub fn read(&self, slot: u32, seq: u64) -> Option<Vec<u8>> {
        if self.seq(slot).load(Ordering::Acquire) != seq {
            return None;
        }
        let start = self.slot_offset(slot) + SLOT_HEADER;
        let pixels = self.map[start..start + self.frame_len()].to_vec();
        (self.seq(slot).load(Ordering::Acquire) == seq).then_some(pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read() {
        let dir = std::env::temp_dir().join(format!("backstage-ring-test-{}", std::process::id()));
        let mut writer = FrameRing::create(&dir, 8, 2).unwrap();
        let reader = FrameRing::open(&dir, 8, 2).unwrap();
        writer.write(1, 7, &[42; 16]);
        assert_eq!(reader.read(1, 7).unwrap(), vec![42; 16]);
        assert!(reader.read(1, 8).is_none(), "stale seq must be rejected");
        writer.write(1, 9, &[1; 16]);
        assert!(reader.read(1, 7).is_none(), "overwritten slot must be rejected");
        std::fs::remove_file(dir).unwrap();
    }
}
