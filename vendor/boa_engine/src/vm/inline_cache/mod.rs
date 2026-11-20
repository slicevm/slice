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
pub(crate) const MAX_ADD_IC_PROTO_DEPTH: usize = 4;

/// Cached monomorphic add-property shape transition (`S_from -> S_to`) for `SetPropertyByName`
/// on ordinary extensible objects (such as class constructors and `Object.create(proto)` factories).
#[derive(Clone, Debug, Trace, Finalize)]
pub(crate) struct AddTransitionEntry {
    pub(crate) to_shape: WeakShape,
    #[unsafe_ignore_trace]
    pub(crate) to_shape_id: u64,
    #[unsafe_ignore_trace]
    pub(crate) expected_storage_len: u32,
    #[unsafe_ignore_trace]
    pub(crate) proto_shape_ids: [u64; MAX_ADD_IC_PROTO_DEPTH],
    #[unsafe_ignore_trace]
    pub(crate) proto_depth: u8,
}

/// A cached shape-to-slot mapping for a polymorphic inline cache.
#[derive(Clone, Debug, Trace, Finalize)]
pub(crate) struct CacheEntry {
    /// A weak reference is kept to the shape to avoid the shape preventing deallocation.
    pub(crate) shape: WeakShape,
    #[unsafe_ignore_trace]
    pub(crate) shape_id: u64,
    #[unsafe_ignore_trace]
    pub(crate) proto_shape_id: u64,
    #[unsafe_ignore_trace]
    pub(crate) slot: Slot,
}

#[derive(Clone, Debug, Default, Trace, Finalize)]
pub(crate) struct IcSlowState {
    pub(crate) entries: ArrayVec<CacheEntry, PIC_CAPACITY>,
    pub(crate) add_transition: Option<AddTransitionEntry>,
}

/// An inline cache entry for a property access.
#[derive(Clone, Debug, Trace, Finalize)]
pub(crate) struct InlineCache {
    /// The property that is accessed.
    pub(crate) name: JsString,

    /// Lazily boxed polymorphic + add-transition cache state (`None` on cold `CodeBlock` decode).
    pub(crate) slow: GcRefCell<Option<Box<IcSlowState>>>,

    /// Whether this access site has seen too many shapes and should no longer be cached.
    #[unsafe_ignore_trace]
    pub(crate) megamorphic: Cell<bool>,

    /// Monomorphic fast-path monotonic shape ID (`0` when empty; immune to pointer-reuse ABA).
    #[unsafe_ignore_trace]
    pub(crate) mono_shape_id: Cell<u64>,

    /// Monomorphic 1-hop prototype `Shape::id()` (`0` for own properties).
    #[unsafe_ignore_trace]
    pub(crate) mono_proto_shape_id: Cell<u64>,

    /// Monomorphic fast-path slot cached alongside `mono_shape_id`.
    #[unsafe_ignore_trace]
    pub(crate) mono_slot: Cell<Slot>,

    /// Monomorphic add-property pre-transition `Shape::id()` (`0` when empty).
    #[unsafe_ignore_trace]
    pub(crate) add_from_shape_id: Cell<u64>,
}

impl fmt::Display for InlineCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(name:{} entries:", self.name.display_escaped())?;

        if self.megamorphic.get() {
            return write!(f, "(megamorphic))");
        }

        let slow = self.slow.borrow();
        if let Some(state) = slow.as_deref() {
            let entries = state
                .entries
                .iter()
                .map(|e| e.shape.to_addr_usize())
                .format(", ");
            write!(f, "({entries:#x}))")
        } else {
            write!(f, "())")
        }
    }
}

impl InlineCache {
    pub(crate) fn new(name: JsString) -> Self {
        Self {
            name,
            slow: GcRefCell::new(None),
            megamorphic: Cell::new(false),
            mono_shape_id: Cell::new(0),
            mono_proto_shape_id: Cell::new(0),
            mono_slot: Cell::new(Slot::new()),
            add_from_shape_id: Cell::new(0),
        }
    }

    pub(crate) fn set(&self, shape: &Shape, slot: Slot) {
        if self.megamorphic.get() {
            return;
        }

        let proto_shape_id = if slot.attributes.contains(SlotAttributes::PROTOTYPE) {
            let Some(proto) = shape.prototype() else {
                return;
            };
            proto.borrow().shape().id()
        } else {
            0
        };

        let mut slow = self.slow.borrow_mut();
        let state = slow.get_or_insert_with(Box::default);
        let shape_id = shape.id();
        if let Some(existing) = state.entries.iter_mut().find(|e| e.shape_id == shape_id) {
            existing.proto_shape_id = proto_shape_id;
            existing.slot = slot;
            if self.mono_shape_id.get() == shape_id {
                self.mono_proto_shape_id.set(proto_shape_id);
                self.mono_slot.set(slot);
            }
            return;
        }

        let was_empty = state.entries.is_empty();

        // Add a new entry if there's space.
        if state
            .entries
            .try_push(CacheEntry {
                shape: shape.into(),
                shape_id,
                proto_shape_id,
                slot,
            })
            .is_err()
        {
            // Polymorphic cache is full, transition to megamorphic.
            self.megamorphic.set(true);
            self.mono_shape_id.set(0);
            self.mono_proto_shape_id.set(0);
            self.add_from_shape_id.set(0);
            *slow = None;
        } else if was_empty {
            self.mono_shape_id.set(shape_id);
            self.mono_proto_shape_id.set(proto_shape_id);
            self.mono_slot.set(slot);
        }
    }

    #[inline]
    pub(crate) fn set_add_transition(
        &self,
        from_shape_id: u64,
        to_shape: &Shape,
        expected_storage_len: u32,
        proto_shape_ids: [u64; MAX_ADD_IC_PROTO_DEPTH],
        proto_depth: u8,
    ) {
        if self.megamorphic.get() {
            return;
        }
        let mut slow = self.slow.borrow_mut();
        let state = slow.get_or_insert_with(Box::default);
        state.add_transition = Some(AddTransitionEntry {
            to_shape: to_shape.into(),
            to_shape_id: to_shape.id(),
            expected_storage_len,
            proto_shape_ids,
            proto_depth,
        });
        self.add_from_shape_id.set(from_shape_id);
    }

    #[inline]
    pub(crate) fn try_match_add_transition(
        &self,
        shape: &Shape,
        storage_len: usize,
    ) -> Option<Shape> {
        let borrow = self.slow.borrow();
        let entry = borrow.as_deref()?.add_transition.as_ref()?;
        if storage_len != entry.expected_storage_len as usize {
            return None;
        }
        let mut cur_proto = shape.prototype();
        for hop in 0..(entry.proto_depth as usize) {
            let p = cur_proto?;
            let pb = p.borrow();
            if pb.shape().id() != entry.proto_shape_ids[hop] {
                return None;
            }
            cur_proto = pb.shape().prototype();
        }
        if cur_proto.is_some() {
            return None;
        }
        let next_shape = entry.to_shape.upgrade()?;
        if next_shape.id() != entry.to_shape_id {
            return None;
        }
        Some(next_shape)
    }

    /// Zero-borrow monomorphic fast-path for own-property and 1-hop prototype slot hits.
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
            if let Some(proto) = shape.prototype()
                && proto.borrow().shape().id() == self.mono_proto_shape_id.get()
            {
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

        let mut slow = self.slow.borrow_mut();
        let state = slow.as_deref_mut()?;
        let entries = &mut state.entries;
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
                    let expected_proto_id = entries[i].proto_shape_id;
                    if expected_proto_id != 0 {
                        let proto_ok = shape
                            .prototype()
                            .is_some_and(|p| p.borrow().shape().id() == expected_proto_id);
                        if !proto_ok {
                            entries.swap_remove(i);
                            continue;
                        }
                    }
                    result = Some((upgraded, entries[i].slot, expected_proto_id));
                    break;
                }
                i += 1;
            } else {
                // Opportunistically clean up stale weak shapes.
                entries.swap_remove(i);
            }
        }

        if let Some((hit_shape, hit_slot, hit_proto_id)) = result {
            self.mono_shape_id.set(hit_shape.id());
            self.mono_proto_shape_id.set(hit_proto_id);
            self.mono_slot.set(hit_slot);
            Some((hit_shape, hit_slot))
        } else {
            if entries.is_empty() {
                self.mono_shape_id.set(0);
                self.mono_proto_shape_id.set(0);
            }
            None
        }
    }
}
