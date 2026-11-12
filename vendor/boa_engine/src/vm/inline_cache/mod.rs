use arrayvec::ArrayVec;
use itertools::Itertools;
use std::{cell::Cell, fmt};

use boa_gc::GcRefCell;
use boa_macros::{Finalize, Trace};

use crate::{
    JsString,
    object::shape::{
        Shape, WeakShape,
        slot::{Slot, SlotAttributes},
    },
};

#[cfg(test)]
mod tests;

pub(crate) const PIC_CAPACITY: usize = 4;

/// A cached shape-to-slot mapping for a polymorphic inline cache.
#[derive(Clone, Debug, Trace, Finalize)]
pub(crate) struct CacheEntry {
    /// A weak reference is kept to the shape to avoid the shape preventing deallocation.
    pub(crate) shape: WeakShape,
    #[unsafe_ignore_trace]
    pub(crate) shape_id: u64,
    #[unsafe_ignore_trace]
    pub(crate) slot: Slot,
}

/// An inline cache entry for a property access.
#[derive(Clone, Debug, Trace, Finalize)]
pub(crate) struct InlineCache {
    /// The property that is accessed.
    pub(crate) name: JsString,

    /// Multiple cached shape-to-slot entries.
    pub(crate) entries: GcRefCell<ArrayVec<CacheEntry, PIC_CAPACITY>>,

    /// Whether this access site has seen too many shapes and should no longer be cached.
    #[unsafe_ignore_trace]
    pub(crate) megamorphic: Cell<bool>,

    /// Monomorphic fast-path monotonic shape ID (`0` when empty; immune to pointer-reuse ABA).
    #[unsafe_ignore_trace]
    pub(crate) mono_shape_id: Cell<u64>,

    /// Monomorphic fast-path slot cached alongside `mono_shape_id`.
    #[unsafe_ignore_trace]
    pub(crate) mono_slot: Cell<Slot>,
}

impl fmt::Display for InlineCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(name:{} entries:", self.name.display_escaped())?;

        if self.megamorphic.get() {
            return write!(f, "(megamorphic))");
        }

        let entries = self.entries.borrow();
        let entries = entries.iter().map(|e| e.shape.to_addr_usize()).format(", ");

        write!(f, "({entries:#x}))")
    }
}

impl InlineCache {
    pub(crate) fn new(name: JsString) -> Self {
        Self {
            name,
            entries: GcRefCell::new(ArrayVec::new()),
            megamorphic: Cell::new(false),
            mono_shape_id: Cell::new(0),
            mono_slot: Cell::new(Slot::new()),
        }
    }

    pub(crate) fn set(&self, shape: &Shape, slot: Slot) {
        if self.megamorphic.get() {
            return;
        }

        let mut entries = self.entries.borrow_mut();
        let was_empty = entries.is_empty();
        let shape_id = shape.id();

        // Add a new entry if there's space.
        if entries
            .try_push(CacheEntry {
                shape: shape.into(),
                shape_id,
                slot,
            })
            .is_err()
        {
            // Polymorphic cache is full, transition to megamorphic.
            self.megamorphic.set(true);
            self.mono_shape_id.set(0);
            entries.clear();
        } else if was_empty {
            self.mono_shape_id.set(shape_id);
            self.mono_slot.set(slot);
        }
    }

    /// Zero-borrow monomorphic fast-path for own-property slot hits.
    ///
    /// When the receiver's monotonic `Shape::id()` matches `mono_shape_id` and the cached slot is
    /// an own property (`!PROTOTYPE`), returns `(None, slot)` without borrowing `GcRefCell` or
    /// upgrading a `WeakGc`. Falls back to the polymorphic `get()` path otherwise.
    #[inline(always)]
    pub(crate) fn get_fast(&self, shape: &Shape) -> Option<(Option<Shape>, Slot)> {
        if self.megamorphic.get() {
            return None;
        }

        if self.mono_shape_id.get() == shape.id() {
            let slot = self.mono_slot.get();
            if !slot.attributes.contains(SlotAttributes::PROTOTYPE) {
                return Some((None, slot));
            }
        }

        self.get(shape).map(|(s, slot)| (Some(s), slot))
    }

    /// Returns the cached `(Shape, Slot)` if a matching shape exists in the inline cache.
    ///
    /// Opportunistically cleans up stale weak shape references during lookup.
    pub(crate) fn get(&self, shape: &Shape) -> Option<(Shape, Slot)> {
        if self.megamorphic.get() {
            return None;
        }

        let mut entries = self.entries.borrow_mut();
        let mut i = 0;
        let mut result = None;
        let shape_id = shape.id();

        while i < entries.len() {
            if let Some(upgraded) = entries[i].shape.upgrade() {
                let upgraded_id = upgraded.id();
                if upgraded_id != entries[i].shape_id {
                    // UniqueShape mutated attributes in-place; evict stale slot entry.
                    entries.swap_remove(i);
                    continue;
                }
                if upgraded_id == shape_id {
                    result = Some((upgraded, entries[i].slot));
                    break;
                }
                i += 1;
            } else {
                // Opportunistically clean up stale weak shapes.
                entries.swap_remove(i);
            }
        }

        if let Some((ref hit_shape, hit_slot)) = result {
            self.mono_shape_id.set(hit_shape.id());
            self.mono_slot.set(hit_slot);
        } else if entries.is_empty() {
            self.mono_shape_id.set(0);
        }

        result
    }
}
