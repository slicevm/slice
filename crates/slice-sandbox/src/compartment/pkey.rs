//! Linux `x86_64` Protection Key (`SYS_pkey_alloc` / `WRPKRU`) Domain Switch & Scoped Restoration.
//!
//! Allocates up to 15 kernel protection keys (`1..=15`) per OS process via `SYS_pkey_alloc` on
//! PKU-capable Linux `x86_64` hosts and switches the per-thread `PKRU` register via `WRPKRU`
//! through an RAII [`PkruDomainGuard`] that saves (`RDPKRU`) and restores the caller's prior
//! `PKRU` mask on normal exit or panic unwind (including nested `Slice.evalSandbox` calls).
//! On non-Linux-`x86_64` platforms (or CPUs/containers without PKU), a deterministic thread-local
//! virtual `PKRU` shadow cell mirrors the exact same scoped save/restore semantics with zero
//! syscall overhead.

use std::cell::Cell;

thread_local! {
    static VIRTUAL_THREAD_PKRU: Cell<u32> = const { Cell::new(0) };
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[inline(always)]
fn read_hw_pkru() -> u32 {
    let pkru: u32;
    // SAFETY: Caller only invokes `read_hw_pkru` when `hardware_enforced == true`, which proves
    // `SYS_pkey_alloc` succeeded on an OSPKE-enabled `x86_64` Linux kernel/CPU (`RDPKRU` legal).
    // `ECX = 0` is required by `RDPKRU` (non-zero `ECX` faults `#GP`).
    unsafe {
        core::arch::asm!(
            "xor ecx, ecx",
            "rdpkru",
            out("eax") pkru,
            out("ecx") _,
            out("edx") _,
            options(nomem, nostack)
        );
    }
    pkru
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[inline(always)]
fn write_hw_pkru(pkru: u32) {
    // SAFETY: Caller only invokes `write_hw_pkru` when `hardware_enforced == true` (`OSPKE` active).
    // `ECX = 0` and `EDX = 0` are required by `WRPKRU`. Omit `nomem` so LLVM treats the permission
    // switch as a compiler memory barrier across guarded memory accesses.
    unsafe {
        core::arch::asm!(
            "xor ecx, ecx",
            "xor edx, edx",
            "wrpkru",
            in("eax") pkru,
            out("ecx") _,
            out("edx") _,
            options(nostack)
        );
    }
}

/// Returns the active `PKRU` mask for the current OS thread (hardware `RDPKRU` when `hw` is true
/// on PKU-capable Linux `x86_64`, otherwise the deterministic thread-local virtual `PKRU` shadow).
#[inline(always)]
pub fn current_thread_pkru(hardware_enforced: bool) -> u32 {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    if hardware_enforced {
        return read_hw_pkru();
    }
    let _ = hardware_enforced;
    VIRTUAL_THREAD_PKRU.with(Cell::get)
}

/// Scoped RAII guard returned by [`HardwareProtectionDomain::enter_domain`].
///
/// Restores the thread's previous `PKRU` mask on normal scope exit or panic unwind so nested
/// compartments (`Slice.evalSandbox`) never clobber or unlock (`PKRU = 0`) an outer tenant's domain.
#[must_use = "bind PkruDomainGuard (`let _guard = domain.enter_domain();`) so the domain remains active across the execution scope"]
pub struct PkruDomainGuard {
    saved_pkru: u32,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    hardware_enforced: bool,
    _not_send: core::marker::PhantomData<*mut ()>,
}

impl Drop for PkruDomainGuard {
    #[inline(always)]
    fn drop(&mut self) {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        if self.hardware_enforced {
            write_hw_pkru(self.saved_pkru);
        }
        VIRTUAL_THREAD_PKRU.with(|slot| slot.set(self.saved_pkru));
    }
}

/// Linux `x86_64` Memory Protection Key (`MPK` / `PKU`) domain descriptor.
///
/// Acts as a 12-byte RAII handle: when `hardware_enforced` is active on a PKU-capable Linux
/// `x86_64` host, dropping a `HardwareProtectionDomain` resets `PKRU` to `0` **only if** this
/// domain's mask is still active on the current thread (never clobbering an outer compartment)
/// and frees the owned kernel protection key once, while cloning allocates an independent key.
#[repr(C)]
#[derive(Debug)]
pub struct HardwareProtectionDomain {
    pub pkey: i32,
    pub pkru_mask: u32,
    pub hardware_enforced: bool,
}

impl HardwareProtectionDomain {
    /// Allocates a hardware protection key via Linux `pkey_alloc` when supported by CPU/kernel,
    /// or assigns a virtualized protection domain tag (`1..=15`) for deterministic domain tracking.
    #[inline]
    pub fn allocate(tenant_slot: u32) -> Self {
        let domain_slot = ((tenant_slot % 15) + 1) as i32;

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            // SAFETY: `SYS_pkey_alloc` (330 on `x86_64` Linux) takes `(flags = 0, access_rights = 0)`
            // and has no memory-aliasing side effects; returns `-1` if unsupported or exhausted.
            let pkey = unsafe { libc::syscall(330, 0usize, 0usize) as i32 };
            if pkey > 0 && pkey < 16 {
                // Keep PKEY 0 (bits 0..1, default kernel stack/heap key) and the active tenant's
                // PKEY (bits 2*pkey..2*pkey+1) set to 0b00 (read/write allowed), while setting
                // all other tenant keys (1..15) to 0b11 (PKEY_DISABLE_ACCESS | PKEY_DISABLE_WRITE).
                let mask = !(0b11u32 | (0b11u32 << (pkey * 2)));
                return Self {
                    pkey,
                    pkru_mask: mask,
                    hardware_enforced: true,
                };
            }
        }

        let mask = !(0b11u32 | (0b11u32 << (domain_slot * 2)));
        Self {
            pkey: domain_slot,
            pkru_mask: mask,
            hardware_enforced: false,
        }
    }

    /// Rebinds a reused compartment to `tenant_slot` without freeing or leaking an already-owned
    /// hardware protection key on hot request resets.
    #[inline]
    pub fn rebind_slot(&mut self, tenant_slot: u32) {
        if !self.hardware_enforced {
            let domain_slot = ((tenant_slot % 15) + 1) as i32;
            self.pkey = domain_slot;
            self.pkru_mask = !(0b11u32 | (0b11u32 << (domain_slot * 2)));
        }
    }

    /// Switches the current thread into this tenant's protection key domain and returns a
    /// [`PkruDomainGuard`] that restores the previous `PKRU` mask on `Drop`.
    #[inline(always)]
    pub fn enter_domain(&self) -> PkruDomainGuard {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        if self.hardware_enforced {
            let saved_hw = read_hw_pkru();
            write_hw_pkru(self.pkru_mask);
            VIRTUAL_THREAD_PKRU.with(|slot| slot.set(self.pkru_mask));
            return PkruDomainGuard {
                saved_pkru: saved_hw,
                hardware_enforced: true,
                _not_send: core::marker::PhantomData,
            };
        }

        let saved_virt = VIRTUAL_THREAD_PKRU.with(|slot| slot.replace(self.pkru_mask));
        PkruDomainGuard {
            saved_pkru: saved_virt,
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            hardware_enforced: false,
            _not_send: core::marker::PhantomData,
        }
    }

    /// Explicitly consumes and releases this hardware protection domain.
    #[inline]
    pub fn release(self) {
        drop(self);
    }
}

impl Clone for HardwareProtectionDomain {
    #[inline]
    fn clone(&self) -> Self {
        if self.hardware_enforced {
            Self::allocate(self.pkey.max(0) as u32)
        } else {
            Self {
                pkey: self.pkey,
                pkru_mask: self.pkru_mask,
                hardware_enforced: false,
            }
        }
    }
}

impl Drop for HardwareProtectionDomain {
    #[inline]
    fn drop(&mut self) {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            if self.hardware_enforced && self.pkey > 0 {
                self.hardware_enforced = false;
                // Only clear `PKRU` to `0` if this domain's mask is currently installed on this
                // thread, ensuring dropping a child/cloned domain never unlocks an active parent!
                if read_hw_pkru() == self.pkru_mask {
                    write_hw_pkru(0);
                    VIRTUAL_THREAD_PKRU.with(|slot| slot.set(0));
                }
                // SAFETY: `self.pkey` (`1..=15`) is exclusively owned by this handle (`hardware_enforced`
                // cleared above) and freed via `SYS_pkey_free` (331 on `x86_64` Linux) after clearing `PKRU`.
                unsafe {
                    let _ = libc::syscall(331, self.pkey as usize);
                }
            }
        }
    }
}
