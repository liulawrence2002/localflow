//! A fixed-size queue of audio samples shared between a device callback and a worker thread.
//!
//! The audio device calls back on a real-time thread, where taking a lock or allocating
//! memory can cause an audible glitch. This ring does neither: one side writes, the other
//! reads, and they coordinate through two atomic counters. It is written entirely in safe
//! code (each slot is an atomic holding the sample's bits), so misuse can scramble audio but
//! cannot corrupt memory.
//!
//! One thread may push and one thread may pop at the same time. The counters only ever
//! grow; a slot index is the counter modulo the capacity.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

pub struct SampleRing {
    slots: Box<[AtomicU32]>,
    /// Total samples ever written.
    written: AtomicUsize,
    /// Total samples ever read or discarded.
    read: AtomicUsize,
}

impl SampleRing {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            slots: (0..capacity).map(|_| AtomicU32::new(0)).collect(),
            written: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Samples waiting to be read.
    pub fn len(&self) -> usize {
        self.written
            .load(Ordering::Acquire)
            .wrapping_sub(self.read.load(Ordering::Acquire))
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Room left before the ring is full.
    pub fn free(&self) -> usize {
        self.capacity() - self.len().min(self.capacity())
    }

    /// Writer side: add one sample. Returns `false`, dropping the sample, when full.
    pub fn push(&self, sample: f32) -> bool {
        let written = self.written.load(Ordering::Relaxed);
        if written.wrapping_sub(self.read.load(Ordering::Acquire)) >= self.capacity() {
            return false;
        }
        self.slots[written % self.capacity()].store(sample.to_bits(), Ordering::Relaxed);
        self.written
            .store(written.wrapping_add(1), Ordering::Release);
        true
    }

    /// Writer side: add as many of `samples` as fit and return how many were taken.
    pub fn push_slice(&self, samples: &[f32]) -> usize {
        let written = self.written.load(Ordering::Relaxed);
        let used = written.wrapping_sub(self.read.load(Ordering::Acquire));
        let count = samples
            .len()
            .min(self.capacity() - used.min(self.capacity()));
        for (offset, sample) in samples[..count].iter().enumerate() {
            self.slots[written.wrapping_add(offset) % self.capacity()]
                .store(sample.to_bits(), Ordering::Relaxed);
        }
        self.written
            .store(written.wrapping_add(count), Ordering::Release);
        count
    }

    /// Reader side: take the oldest sample.
    pub fn pop(&self) -> Option<f32> {
        let read = self.read.load(Ordering::Relaxed);
        if read == self.written.load(Ordering::Acquire) {
            return None;
        }
        let sample = f32::from_bits(self.slots[read % self.capacity()].load(Ordering::Relaxed));
        self.read.store(read.wrapping_add(1), Ordering::Release);
        Some(sample)
    }

    /// Reader side: append everything waiting to `out` and return how many samples that was.
    pub fn pop_all(&self, out: &mut Vec<f32>) -> usize {
        let read = self.read.load(Ordering::Relaxed);
        let count = self.written.load(Ordering::Acquire).wrapping_sub(read);
        out.reserve(count);
        for offset in 0..count {
            out.push(f32::from_bits(
                self.slots[read.wrapping_add(offset) % self.capacity()].load(Ordering::Relaxed),
            ));
        }
        self.read.store(read.wrapping_add(count), Ordering::Release);
        count
    }

    /// Reader side: throw away everything waiting and return how many samples that was.
    pub fn discard_all(&self) -> usize {
        let read = self.read.load(Ordering::Relaxed);
        let written = self.written.load(Ordering::Acquire);
        self.read.store(written, Ordering::Release);
        written.wrapping_sub(read)
    }

    /// Reader side: how many samples have ever been read or discarded.
    pub fn read_position(&self) -> usize {
        self.read.load(Ordering::Acquire)
    }

    /// Writer side: how many samples have ever been written. Taken before a flush, it marks
    /// where the audio to throw away ends, so [`Self::discard_until`] keeps anything written
    /// after it.
    pub fn write_position(&self) -> usize {
        self.written.load(Ordering::Acquire)
    }

    /// Reader side: throw away what was written before `position` (a value of
    /// [`Self::write_position`]) and has not been read yet. Returns how many samples that was.
    pub fn discard_until(&self, position: usize) -> usize {
        let read = self.read.load(Ordering::Relaxed);
        let written = self.written.load(Ordering::Acquire);
        let waiting = written.wrapping_sub(read);
        let to_drop = position.wrapping_sub(read);
        // Nothing to do when everything before `position` was already read (`position` is
        // behind the read counter, which wraps `to_drop` past `waiting`).
        if to_drop == 0 || to_drop > waiting {
            return 0;
        }
        self.read.store(position, Ordering::Release);
        to_drop
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread};

    use super::*;

    #[test]
    fn discarding_up_to_a_mark_keeps_what_was_written_after_it() {
        let ring = SampleRing::new(16);
        ring.push_slice(&[1.0, 2.0, 3.0]);
        let mark = ring.write_position();
        ring.push_slice(&[4.0, 5.0]);

        assert_eq!(ring.discard_until(mark), 3);
        let mut rest = Vec::new();
        ring.pop_all(&mut rest);
        assert_eq!(rest, vec![4.0, 5.0]);
    }

    #[test]
    fn a_mark_already_read_past_discards_nothing() {
        let ring = SampleRing::new(16);
        ring.push_slice(&[1.0, 2.0]);
        let mark = ring.write_position();
        ring.push_slice(&[3.0, 4.0]);
        assert_eq!(ring.pop(), Some(1.0));
        assert_eq!(ring.pop(), Some(2.0));
        assert_eq!(ring.pop(), Some(3.0));

        assert_eq!(ring.discard_until(mark), 0);
        assert_eq!(ring.pop(), Some(4.0));
        assert_eq!(ring.discard_until(ring.write_position()), 0);
    }

    #[test]
    fn samples_come_out_in_the_order_they_went_in() {
        let ring = SampleRing::new(8);

        assert_eq!(ring.push_slice(&[1.0, 2.0, 3.0]), 3);
        assert!(ring.push(4.0));
        assert_eq!(ring.len(), 4);
        assert_eq!(ring.pop(), Some(1.0));
        let mut rest = Vec::new();
        assert_eq!(ring.pop_all(&mut rest), 3);

        assert_eq!(rest, vec![2.0, 3.0, 4.0]);
        assert!(ring.is_empty());
        assert_eq!(ring.pop(), None);
    }

    #[test]
    fn a_full_ring_refuses_more_instead_of_overwriting() {
        let ring = SampleRing::new(4);

        assert_eq!(ring.push_slice(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]), 4);
        assert!(!ring.push(7.0));
        assert_eq!(ring.free(), 0);

        assert_eq!(ring.pop(), Some(1.0));
        assert_eq!(ring.free(), 1);
        assert!(ring.push(8.0));
        let mut all = Vec::new();
        ring.pop_all(&mut all);
        assert_eq!(all, vec![2.0, 3.0, 4.0, 8.0]);
    }

    #[test]
    fn it_wraps_around_many_times_without_losing_order() {
        let ring = SampleRing::new(7);
        let mut expected = 0u32;
        let mut next = 0u32;
        let mut out = Vec::new();

        for round in 0..500 {
            let batch: Vec<f32> = (0..(round % 6) + 1)
                .map(|_| {
                    next += 1;
                    next as f32
                })
                .collect();
            let taken = ring.push_slice(&batch);
            next -= (batch.len() - taken) as u32;
            out.clear();
            ring.pop_all(&mut out);
            for sample in &out {
                expected += 1;
                assert_eq!(*sample, expected as f32);
            }
        }
        assert!(expected > 500);
    }

    #[test]
    fn discarding_empties_the_ring_and_new_audio_is_kept() {
        let ring = SampleRing::new(16);
        ring.push_slice(&[0.5; 10]);

        assert_eq!(ring.discard_all(), 10);
        assert!(ring.is_empty());

        ring.push_slice(&[0.25, 0.75]);
        let mut out = Vec::new();
        ring.pop_all(&mut out);
        assert_eq!(out, vec![0.25, 0.75]);
    }

    #[test]
    fn one_writer_and_one_reader_on_separate_threads_lose_nothing() {
        let ring = Arc::new(SampleRing::new(64));
        const TOTAL: u32 = 200_000;

        let writer = {
            let ring = ring.clone();
            thread::spawn(move || {
                let mut next = 1u32;
                while next <= TOTAL {
                    if ring.push(next as f32) {
                        next += 1;
                    } else {
                        thread::yield_now();
                    }
                }
            })
        };

        let mut expected = 1u32;
        let mut out = Vec::new();
        while expected <= TOTAL {
            out.clear();
            if ring.pop_all(&mut out) == 0 {
                thread::yield_now();
            }
            for sample in &out {
                assert_eq!(*sample, expected as f32);
                expected += 1;
            }
        }
        writer.join().unwrap();
        assert!(ring.is_empty());
    }
}
