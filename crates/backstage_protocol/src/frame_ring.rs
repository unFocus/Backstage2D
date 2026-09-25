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
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?;
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
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    /// A ring file removed when the test ends.
    struct TempRing(PathBuf);

    impl TempRing {
        fn new(name: &str) -> Self {
            Self(std::env::temp_dir().join(format!("backstage-ring-{name}-{}", std::process::id())))
        }
    }

    impl Drop for TempRing {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn write_then_read() {
        let tmp = TempRing::new("rw");
        let mut writer = FrameRing::create(&tmp.0, 8, 2).unwrap();
        let reader = FrameRing::open(&tmp.0, 8, 2).unwrap();
        writer.write(1, 7, &[42; 16]);
        assert_eq!(reader.read(1, 7).unwrap(), vec![42; 16]);
        assert!(reader.read(1, 8).is_none(), "stale seq must be rejected");
        writer.write(1, 9, &[1; 16]);
        assert!(reader.read(1, 7).is_none(), "overwritten slot must be rejected");
    }

    #[test]
    fn open_rejects_a_ring_that_is_too_small() {
        let tmp = TempRing::new("small");
        FrameRing::create(&tmp.0, 8, 2).unwrap();
        assert!(FrameRing::open(&tmp.0, 8, 3).is_err());
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn slot_out_of_range_panics() {
        let tmp = TempRing::new("range");
        let mut ring = FrameRing::create(&tmp.0, 4, 1).unwrap();
        ring.write(FRAME_SLOTS, 1, &[0; 4]);
    }

    #[test]
    #[should_panic(expected = "seq 0")]
    fn seq_zero_panics() {
        let tmp = TempRing::new("seq0");
        let mut ring = FrameRing::create(&tmp.0, 4, 1).unwrap();
        ring.write(0, 0, &[0; 4]);
    }

    /// The writer laps the reader constantly. Every frame the reader accepts
    /// must be exactly the one published under that seq, never a mix.
    #[test]
    fn concurrent_reader_never_sees_torn_frames() {
        const STRIDE: u32 = 4096;
        const HEIGHT: u32 = 64;
        let tmp = TempRing::new("stress");
        let mut writer = FrameRing::create(&tmp.0, STRIDE, HEIGHT).unwrap();
        let reader = FrameRing::open(&tmp.0, STRIDE, HEIGHT).unwrap();
        let latest = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));

        let writer_thread = {
            let latest = latest.clone();
            let done = done.clone();
            std::thread::spawn(move || {
                let mut frame = vec![0u8; (STRIDE * HEIGHT) as usize];
                for seq in 1..=3000u64 {
                    frame.fill(seq as u8);
                    writer.write((seq % FRAME_SLOTS as u64) as u32, seq, &frame);
                    latest.store(seq, Ordering::Release);
                }
                done.store(true, Ordering::Release);
            })
        };

        let (mut accepted, mut rejected) = (0u32, 0u32);
        while !done.load(Ordering::Acquire) {
            let seq = latest.load(Ordering::Acquire);
            if seq == 0 {
                continue;
            }
            // Reading an older seq makes collisions with the writer likely.
            let seq = seq.saturating_sub(2).max(1);
            match reader.read((seq % FRAME_SLOTS as u64) as u32, seq) {
                Some(pixels) => {
                    accepted += 1;
                    assert!(pixels.iter().all(|&b| b == seq as u8), "torn frame for seq {seq}");
                }
                None => rejected += 1,
            }
        }
        writer_thread.join().unwrap();
        assert!(accepted > 0, "reader never accepted a frame ({rejected} rejected)");
    }
}
