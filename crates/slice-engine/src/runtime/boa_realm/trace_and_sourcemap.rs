//! Chrome DevTools / Perfetto Trace Recording and Zero-Copy Source-Map Stack Translation.

use super::{
    BOA_ACTIVE_SOURCE_FUSED_SPEC, BOA_ACTIVE_SOURCE_PTR_LEN, BOA_TRACE_ENABLED, BOA_TRACE_EVENTS,
};

/// Chrome DevTools / Perfetto Trace Event (`ph: "X"` complete duration event).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChromeTraceEvent {
    pub name: String,
    pub cat: String,
    pub ph: String,
    pub ts: u64,
    pub dur: u64,
    pub pid: u32,
    pub tid: u32,
    pub args: serde_json::Value,
}

#[inline]
pub(crate) fn is_trace_recording_enabled() -> bool {
    BOA_TRACE_ENABLED.with(|c| c.get())
}

pub(crate) fn record_trace_event(
    name: impl Into<String>,
    cat: impl Into<String>,
    ts: u64,
    dur: u64,
    args: serde_json::Value,
) {
    BOA_TRACE_EVENTS.with(|ev| {
        ev.borrow_mut().push(ChromeTraceEvent {
            name: name.into(),
            cat: cat.into(),
            ph: "X".to_string(),
            ts,
            dur: dur.max(1),
            pid: 1,
            tid: 1,
            args,
        });
    });
}

/// Zero-allocation `!Send` RAII guard that binds the currently executing JS bundle slice `(ptr, len)`
/// to `BOA_ACTIVE_SOURCE_PTR_LEN` so lazy `Error.prototype.stack` accesses can translate frames
/// without copying or allocating the source string on the hot execution path.
pub(crate) struct ActiveSourceSliceGuard<'a> {
    prev: (usize, usize),
    prev_fused_spec: u64,
    _marker: std::marker::PhantomData<*const &'a [u8]>,
}

impl<'a> ActiveSourceSliceGuard<'a> {
    #[inline]
    pub(crate) fn new(src: &'a str) -> Self {
        Self::new_bytes(src.as_bytes())
    }

    #[inline]
    pub(crate) fn new_bytes(bytes: &'a [u8]) -> Self {
        let prev = BOA_ACTIVE_SOURCE_PTR_LEN.with(|c| {
            let p = c.get();
            c.set((bytes.as_ptr() as usize, bytes.len()));
            p
        });
        let prev_fused_spec = BOA_ACTIVE_SOURCE_FUSED_SPEC.with(|c| c.replace(0));
        Self {
            prev,
            prev_fused_spec,
            _marker: std::marker::PhantomData,
        }
    }
}

impl Drop for ActiveSourceSliceGuard<'_> {
    #[inline]
    fn drop(&mut self) {
        BOA_ACTIVE_SOURCE_PTR_LEN.with(|c| c.set(self.prev));
        BOA_ACTIVE_SOURCE_FUSED_SPEC.with(|c| c.set(self.prev_fused_spec));
    }
}

#[inline]
pub(crate) fn encode_fused_source_spec(shards_mask: u32, init_lazy: bool) -> u64 {
    (1u64 << 63) | ((u64::from(init_lazy)) << 32) | u64::from(shards_mask)
}

#[inline]
pub(crate) fn set_active_source_fused_spec(spec: u64) {
    BOA_ACTIVE_SOURCE_FUSED_SPEC.with(|c| c.set(spec));
}

#[cold]
fn active_source_line_offset() -> usize {
    let spec = BOA_ACTIVE_SOURCE_FUSED_SPEC.with(|c| c.get());
    if (spec & (1u64 << 63)) == 0 {
        return 0;
    }
    let shards_mask = spec as u32;
    let init_lazy = ((spec >> 32) & 1) != 0;
    crate::js_preludes::count_fused_prelude_newlines(shards_mask, init_lazy)
}

#[cold]
#[allow(unsafe_code)]
fn with_active_source_str<R>(f: impl FnOnce(&str) -> R) -> Option<R> {
    let (ptr, len) = BOA_ACTIVE_SOURCE_PTR_LEN.with(|c| c.get());
    if ptr == 0 || len == 0 {
        return None;
    }
    // SAFETY:
    // 1. `BOA_ACTIVE_SOURCE_PTR_LEN` is only written by `ActiveSourceSliceGuard::new_bytes`,
    //    which carries `PhantomData<*const &'a [u8]>` (`!Send + !Sync`) so the guard cannot cross
    //    threads and is pinned to the current thread's call stack for the slice's lifetime `'a`.
    // 2. `ActiveSourceSliceGuard::drop` unconditionally restores `prev` on normal return or panic
    //    unwind before `'a` ends.
    // 3. `bytes` originates from an immutable `&'a [u8]` borrow and `active_src` does not escape
    //    the synchronous closure `f(active_src)`.
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    let active_src = simdutf8::basic::from_utf8(bytes).ok()?;
    Some(f(active_src))
}

#[cold]
pub(crate) fn map_stack_with_active_source(raw_stack: &str) -> String {
    with_active_source_str(|active_src| {
        let line_offset = active_source_line_offset();
        slice_cas::map_bundled_stack_trace_with_offset(active_src, raw_stack, line_offset)
    })
    .unwrap_or_else(|| raw_stack.to_string())
}

#[cold]
pub(crate) fn map_error_with_active_source(raw_error: &str) -> String {
    with_active_source_str(|active_src| {
        let line_offset = active_source_line_offset();
        slice_cas::map_bundled_error_location_with_offset(active_src, raw_error, line_offset)
    })
    .unwrap_or_else(|| raw_error.to_string())
}
