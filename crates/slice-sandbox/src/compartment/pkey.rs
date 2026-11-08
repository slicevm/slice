//! Hardware Memory Protection Key (`pkey_alloc` / `WRPKRU` on Linux x86_64) Domain Isolation.

/// Hardware Memory Protection Key (Intel MPK / ARM POE) domain descriptor.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HardwareProtectionDomain {
    pub pkey: i32,
    pub pkru_mask: u32,
    pub hardware_enforced: bool,
}

impl HardwareProtectionDomain {
    /// Allocates a hardware protection key via Linux `pkey_alloc` when supported by CPU/kernel,
    /// or assigns a virtualized protection domain tag (1..15) for deterministic domain tracking.
    #[inline]
    pub fn allocate(tenant_slot: u32) -> Self {
        let domain_slot = ((tenant_slot % 15) + 1) as i32;

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            // SYS_pkey_alloc on x86_64 Linux is 330
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

    /// Executes a sub-20-cycle domain switch into this tenant's protection key domain.
    #[inline(always)]
    pub fn enter_domain(&self) {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            if self.hardware_enforced {
                unsafe {
                    // Unprivileged WRPKRU instruction: EAX = pkru value, ECX = 0, EDX = 0
                    core::arch::asm!(
                        "xor ecx, ecx",
                        "xor edx, edx",
                        "wrpkru",
                        in("eax") self.pkru_mask,
                        out("ecx") _,
                        out("edx") _,
                        options(nomem, nostack)
                    );
                }
            }
        }
    }

    /// Releases the hardware protection key if allocated via kernel `pkey_alloc`.
    #[inline]
    pub fn release(&self) {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            if self.hardware_enforced && self.pkey > 0 {
                unsafe {
                    let default_pkru: u32 = 0;
                    core::arch::asm!(
                        "xor ecx, ecx",
                        "xor edx, edx",
                        "wrpkru",
                        in("eax") default_pkru,
                        out("ecx") _,
                        out("edx") _,
                        options(nomem, nostack)
                    );
                    // SYS_pkey_free on x86_64 Linux is 331
                    let _ = libc::syscall(331, self.pkey as usize);
                }
            }
        }
    }
}
