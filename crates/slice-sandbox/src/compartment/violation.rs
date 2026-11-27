//! Structured OCap, SES, Resource Cage, Hardware PKEY, and Egress DLP Violation Errors.

use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum SandboxViolation {
    #[error("OCap Violation: lacks filesystem read authority for path '{path}'")]
    FsReadDenied { tenant_id: String, path: String },

    #[error("OCap Violation: lacks filesystem write authority for path '{path}'")]
    FsWriteDenied { tenant_id: String, path: String },

    #[error("OCap Violation: lacks outbound network authority for host '{host}'")]
    NetAccessDenied { tenant_id: String, host: String },

    #[error("OCap Violation: lacks environment variable authority for key '{key}'")]
    EnvAccessDenied { tenant_id: String, key: String },

    #[error("OCap Violation: lacks exec authority for binary '{binary}'")]
    ExecDenied { tenant_id: String, binary: String },

    #[error("OCap Violation: lacks extension authority for '{target}'")]
    ExtensionDenied { tenant_id: String, target: String },

    #[error("SES Lockdown Violation: attempted to mutate frozen intrinsic '{intrinsic}'")]
    IntrinsicMutationBlocked {
        tenant_id: String,
        intrinsic: String,
    },

    #[error(
        "Resource Cage Exhausted: exceeded deterministic CPU instruction fuel limit ({limit} ops)"
    )]
    CpuFuelExhausted { tenant_id: String, limit: u64 },

    #[error("Resource Cage Exhausted: exceeded memory ceiling ({requested} > {limit} bytes)")]
    MemoryCeilingExceeded {
        tenant_id: String,
        requested: usize,
        limit: usize,
    },

    #[error(
        "Hardware PKEY Trap: attempted unauthorized cross-domain memory access (PKEY 0x{pkey:x})"
    )]
    HardwareMemoryDomainTrap { tenant_id: String, pkey: u32 },

    #[error("OCap Violation: checkpoint denied: {reason}")]
    CheckpointDenied { tenant_id: String, reason: String },

    #[error(
        "Egress DLP Violation (EGRESS_SECRET_LEAK_BLOCKED): blocked from exfiltrating a registered audit secret"
    )]
    EgressSecretLeakBlocked { tenant_id: String },

    #[error(
        "Resource Cage Exhausted (WallClockExceeded): exceeded wall-clock deadline ({limit_ms} ms)"
    )]
    WallClockExceeded { tenant_id: String, limit_ms: u64 },

    #[error("Execution Interrupted: interrupted by host interrupt handle")]
    ExecutionInterrupted { tenant_id: String },
}

impl SandboxViolation {
    /// Returns the host-side compartment identifier associated with this violation for structured
    /// server/telemetry logs (intentionally excluded from guest-facing `Display` strings).
    #[must_use]
    pub fn tenant_id(&self) -> &str {
        match self {
            Self::FsReadDenied { tenant_id, .. }
            | Self::FsWriteDenied { tenant_id, .. }
            | Self::NetAccessDenied { tenant_id, .. }
            | Self::EnvAccessDenied { tenant_id, .. }
            | Self::ExecDenied { tenant_id, .. }
            | Self::ExtensionDenied { tenant_id, .. }
            | Self::IntrinsicMutationBlocked { tenant_id, .. }
            | Self::CpuFuelExhausted { tenant_id, .. }
            | Self::MemoryCeilingExceeded { tenant_id, .. }
            | Self::HardwareMemoryDomainTrap { tenant_id, .. }
            | Self::CheckpointDenied { tenant_id, .. }
            | Self::EgressSecretLeakBlocked { tenant_id }
            | Self::WallClockExceeded { tenant_id, .. }
            | Self::ExecutionInterrupted { tenant_id } => tenant_id,
        }
    }

    /// Returns a concise, compartment-name-free summary of an OCap capability denial for
    /// `deniedCapabilities` audit logs across `SliceIsolate`, `Slice.evalSandbox`, and `slice mcp`.
    #[must_use]
    pub fn capability_denial_summary(&self) -> String {
        match self {
            Self::FsReadDenied { path, .. } => {
                format!("lacks filesystem read authority for path '{path}'")
            }
            Self::FsWriteDenied { path, .. } => {
                format!("lacks filesystem write authority for path '{path}'")
            }
            Self::NetAccessDenied { host, .. } => {
                format!("lacks outbound network authority for host '{host}'")
            }
            Self::EnvAccessDenied { key, .. } => {
                format!("lacks environment variable authority for key '{key}'")
            }
            Self::ExecDenied { binary, .. } => {
                format!("lacks exec authority for binary '{binary}'")
            }
            Self::ExtensionDenied { target, .. } => {
                format!("lacks extension authority for '{target}'")
            }
            Self::CheckpointDenied { reason, .. } => {
                format!("checkpoint denied: {reason}")
            }
            _ => self.to_string(),
        }
    }
}
