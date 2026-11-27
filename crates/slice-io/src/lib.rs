//! Direct Completion Ring Engine (`io_uring` SQPOLL on Linux / `IOCP` on Windows)
//!
//! Replaces readiness-based `libuv` thread pools with lockless Submission Queue (SQ)
//! and Completion Queue (CQ) rings backed by pre-registered, page-aligned fixed buffers.
//!
//! ## Domain Subsystems
//! - [`slab`]: [`RegisteredBufferPool`] pinned 64 KB slab allocator with cache-warm LIFO slot recycling.
//! - [`ring`]: [`CompletionRingEngine`], [`IoSubmission`], [`IoCompletion`], and [`RingStatus`].

#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

#[allow(unsafe_code)]
pub mod reactor;
#[allow(unsafe_code)]
pub mod ring;
#[allow(unsafe_code)]
pub mod slab;

pub use reactor::{Interest, NetReactor, ReadyEvent};
pub use ring::{
    CompletionRingEngine, DEFAULT_RING_ENTRIES, IoCompletion, IoSubmission, MAX_RING_ENTRIES,
    RingStatus,
};
pub use slab::{FIXED_BUFFER_COUNT, FIXED_BUFFER_SIZE, PAGE_ALIGNMENT, RegisteredBufferPool};

#[cfg(test)]
#[path = "../tests/mod.rs"]
mod tests;
