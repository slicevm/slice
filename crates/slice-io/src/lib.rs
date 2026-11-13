//! Direct Completion Ring Engine (`io_uring` SQPOLL on Linux / `IOCP` on Windows)
//!
//! Replaces readiness-based `libuv` thread pools with lockless Submission Queue (SQ)
//! and Completion Queue (CQ) rings backed by pre-registered, page-aligned fixed buffers.
//!
//! ## Domain Subsystems
//! - [`slab`]: [`RegisteredBufferPool`] pinned 64 KB slab allocator with cache-warm LIFO slot recycling.
//! - [`ring`]: [`CompletionRingEngine`], [`IoSubmission`], [`IoCompletion`], and [`RingStatus`].

pub mod ring;
pub mod slab;

pub use ring::{
    CompletionRingEngine, IoCompletion, IoSubmission, RingStatus, DEFAULT_RING_ENTRIES,
    MAX_RING_ENTRIES,
};
pub use slab::{RegisteredBufferPool, FIXED_BUFFER_COUNT, FIXED_BUFFER_SIZE};

#[cfg(test)]
#[path = "../tests/mod.rs"]
mod tests;
