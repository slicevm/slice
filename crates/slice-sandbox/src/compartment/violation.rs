//! Structured OCap, SES, Resource Cage, Hardware PKEY, and Egress DLP Violation Errors.

use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum SandboxViolation {
    #[error(
        "OCap Violation: Compartment '{tenant_id}' lacks filesystem read authority for path '{path}'"
    )]
    FsReadDenied { tenant_id: String, path: String },

    #[error(
        "OCap Violation: Compartment '{tenant_id}' lacks filesystem write authority for path '{path}'"
    )]
    FsWriteDenied { tenant_id: String, path: String },

    #[error(
        "OCap Violation: Compartment '{tenant_id}' lacks outbound network authority for host '{host}'"
    )]
    NetAccessDenied { tenant_id: String, host: String },

    #[error(
        "OCap Violation: Compartment '{tenant_id}' lacks environment variable authority for key '{key}'"
    )]
    EnvAccessDenied { tenant_id: String, key: String },

    #[error(
        "[Slice Sandbox Violation] Subprocess execution denied: Compartment '{tenant_id}' lacks exec authority for binary '{binary}'"
    )]
    ExecDenied { tenant_id: String, binary: String },

    #[error(
        "SES Lockdown Violation: Compartment '{tenant_id}' attempted to mutate frozen intrinsic '{intrinsic}'"
    )]
    IntrinsicMutationBlocked {
        tenant_id: String,
        intrinsic: String,
    },

    #[error(
        "Resource Cage Exhausted: Compartment '{tenant_id}' exceeded deterministic CPU instruction fuel limit ({limit} ops)"
    )]
    CpuFuelExhausted { tenant_id: String, limit: u64 },

    #[error(
        "Resource Cage Exhausted: Compartment '{tenant_id}' exceeded memory ceiling ({requested} > {limit} bytes)"
    )]
    MemoryCeilingExceeded {
        tenant_id: String,
        requested: usize,
        limit: usize,
    },

    #[error(
        "Hardware PKEY Trap: Compartment '{tenant_id}' attempted unauthorized cross-domain memory access (PKEY 0x{pkey:x})"
    )]
    HardwareMemoryDomainTrap { tenant_id: String, pkey: u32 },

    #[error(
        "OCap Violation (CapabilityDenied: checkpoints): Compartment '{tenant_id}' checkpoint denied: {reason}"
    )]
    CheckpointDenied { tenant_id: String, reason: String },

    #[error(
        "Egress DLP Violation (EGRESS_SECRET_LEAK_BLOCKED): Compartment '{tenant_id}' blocked from exfiltrating a registered audit secret"
    )]
    EgressSecretLeakBlocked { tenant_id: String },
}
