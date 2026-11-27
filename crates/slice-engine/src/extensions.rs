//! Capability-Gated Host Extensions (`Slice.ext.<name>.<method>`) for Rust Embedders (`#229`).
//!
//! Host applications register [`SliceExtension`] implementations once at startup inside an
//! [`ExtensionRegistry`] (shared cheaply via `Arc` across isolates with zero per-boot JS overhead).
//! Guest code can reach an extension only when explicitly granted via
//! `CapabilityToken::with_extension("<name>")` or method-scoped `with_extension("<name>.<method>")`.
//!
//! Note on checkpoints: host extension calls are external side effects outside the in-memory VFS
//! and `Slice.kv`; `Slice.restore()` does not undo them. Extensions that must not run inside
//! checkpointed regions can set `.with_allow_in_checkpoint(false)` on their [`ExtMethod`] specs.

use rustc_hash::FxHashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Value crossing the host-extension boundary (always copied; no host objects or handles enter the realm).
#[derive(Debug, Clone, PartialEq)]
pub enum ExtValue {
    Json(serde_json::Value),
    Bytes(Vec<u8>),
}

impl ExtValue {
    /// Approximate serialized byte size used for argument/result limit and fuel/memory accounting.
    #[inline]
    pub fn approx_byte_len(&self) -> usize {
        match self {
            Self::Bytes(b) => b.len(),
            Self::Json(v) => serde_json::to_vec(v).map(|buf| buf.len()).unwrap_or(0),
        }
    }
}

/// Error returned by a host extension method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtError {
    Message(String),
    Denied(String),
    Cancelled,
    InvalidArgument(String),
    ResultTooLarge { actual: usize, limit: usize },
}

impl std::fmt::Display for ExtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(msg) => f.write_str(msg),
            Self::Denied(target) => {
                write!(
                    f,
                    "OCap Violation: lacks extension authority for '{target}'"
                )
            }
            Self::Cancelled => f.write_str("Extension call cancelled"),
            Self::InvalidArgument(msg) => write!(f, "Invalid extension argument: {msg}"),
            Self::ResultTooLarge { actual, limit } => {
                write!(
                    f,
                    "Extension result exceeds maximum size ({actual} > {limit} bytes)"
                )
            }
        }
    }
}

impl std::error::Error for ExtError {}

/// Result returned by synchronous and asynchronous host extension calls.
pub type ExtResult = Result<ExtValue, ExtError>;

/// Boxed future returned by asynchronous host extension methods.
pub type ExtFuture = Pin<Box<dyn Future<Output = ExtResult> + Send + 'static>>;

/// Static descriptor for a single method exposed by a [`SliceExtension`].
#[derive(Debug, Clone, Copy)]
pub struct ExtMethod {
    pub name: &'static str,
    pub is_async: bool,
    pub max_arg_bytes: usize,
    pub max_result_bytes: usize,
    pub base_fuel_cost: u64,
    pub fuel_per_byte: u64,
    pub deterministic: bool,
    pub allow_in_checkpoint: bool,
}

impl ExtMethod {
    /// Creates a synchronous method descriptor with safe default bounds (`256 KiB` arg, `1 MiB` result,
    /// `500` base fuel + `1` fuel/byte, non-deterministic, allowed in checkpointed regions).
    pub const fn sync(name: &'static str) -> Self {
        Self {
            name,
            is_async: false,
            max_arg_bytes: 256 * 1024,
            max_result_bytes: 1024 * 1024,
            base_fuel_cost: 500,
            fuel_per_byte: 1,
            deterministic: false,
            allow_in_checkpoint: true,
        }
    }

    /// Creates an asynchronous method descriptor (returns a JS `Promise` in the guest).
    pub const fn r#async(name: &'static str) -> Self {
        Self {
            name,
            is_async: true,
            max_arg_bytes: 256 * 1024,
            max_result_bytes: 1024 * 1024,
            base_fuel_cost: 500,
            fuel_per_byte: 1,
            deterministic: false,
            allow_in_checkpoint: true,
        }
    }

    pub const fn with_max_arg_bytes(mut self, bytes: usize) -> Self {
        self.max_arg_bytes = bytes;
        self
    }

    pub const fn with_max_result_bytes(mut self, bytes: usize) -> Self {
        self.max_result_bytes = bytes;
        self
    }

    pub const fn with_fuel(mut self, base_fuel_cost: u64, fuel_per_byte: u64) -> Self {
        self.base_fuel_cost = base_fuel_cost;
        self.fuel_per_byte = fuel_per_byte;
        self
    }

    pub const fn with_deterministic(mut self, deterministic: bool) -> Self {
        self.deterministic = deterministic;
        self
    }

    pub const fn with_allow_in_checkpoint(mut self, allow: bool) -> Self {
        self.allow_in_checkpoint = allow;
        self
    }
}

/// Execution context passed to every [`SliceExtension`] invocation.
#[derive(Debug, Clone)]
pub struct ExtCallCtx {
    pub tenant_id: String,
    pub remaining_ms: Option<u64>,
    pub cancel: Arc<AtomicBool>,
}

impl ExtCallCtx {
    /// Returns `true` if the host interrupt handle or wall-clock deadline has cancelled this call.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
}

/// Host-implemented extension reachable in guest JS as `Slice.ext.<name>.<method>(arg)`.
pub trait SliceExtension: Send + Sync + 'static {
    /// Namespace name under `Slice.ext` (for example `"store"`, `"fn"`, `"telemetry"`).
    fn name(&self) -> &'static str;

    /// Static list of methods exposed by this extension.
    fn methods(&self) -> &'static [ExtMethod];

    /// Synchronous method handler (invoked when `ExtMethod::is_async == false`).
    fn call(&self, ctx: &ExtCallCtx, method: &str, arg: ExtValue) -> ExtResult {
        let _ = (ctx, method, arg);
        Err(ExtError::Message(
            "synchronous extension call not implemented".to_string(),
        ))
    }

    /// Asynchronous method handler (invoked when `ExtMethod::is_async == true`).
    /// Defaults to wrapping `self.call(...)` in a ready future.
    fn call_async(&self, ctx: &ExtCallCtx, method: &str, arg: ExtValue) -> ExtFuture {
        let res = self.call(ctx, method, arg);
        Box::pin(async move { res })
    }
}

/// Immutable, cheaply-cloneable (`Arc`) registry of host extensions shared across isolates.
#[derive(Clone, Default)]
pub struct ExtensionRegistry {
    extensions: Arc<FxHashMap<String, Arc<dyn SliceExtension>>>,
}

impl std::fmt::Debug for ExtensionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.extensions.keys().map(String::as_str).collect();
        f.debug_struct("ExtensionRegistry")
            .field("extensions", &names)
            .finish()
    }
}

impl ExtensionRegistry {
    /// Creates an empty extension registry.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a new registry with `ext` registered under `ext.name()`.
    pub fn with(mut self, ext: Arc<dyn SliceExtension>) -> Self {
        self.register(ext);
        self
    }

    /// Inserts or replaces an extension in this registry (copy-on-write if already shared).
    pub fn register(&mut self, ext: Arc<dyn SliceExtension>) {
        let name = ext.name().trim();
        if name.is_empty() || name.contains('.') || name == "*" {
            return;
        }
        Arc::make_mut(&mut self.extensions).insert(name.to_string(), ext);
    }

    /// Looks up a registered extension by namespace name.
    #[inline]
    pub fn get(&self, name: &str) -> Option<&Arc<dyn SliceExtension>> {
        self.extensions.get(name)
    }

    /// Returns the number of registered extensions.
    #[inline]
    pub fn len(&self) -> usize {
        self.extensions.len()
    }

    /// Returns `true` if no extensions are registered.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty()
    }
}

/// Thin synchronous byte-slice host extension bridge for 32-bit MCU and embedded wrappers (`#144`).
///
/// Keeps upstream `slice` completely agnostic of hardware peripherals (GPIO, SPI/I2C, Wi-Fi, BLE)
/// while letting embedded hosts expose OCap-gated peripheral ops via [`HostExtensionAdapter`].
pub trait SliceHostExtension: Send + Sync + 'static {
    fn invoke(
        &self,
        target: &str,
        op: u32,
        input: &[u8],
        output: &mut Vec<u8>,
    ) -> Result<u32, String>;
}

const HOST_EXT_ADAPTER_METHODS: &[ExtMethod] = &[ExtMethod::sync("invoke")];

/// Adapts any [`SliceHostExtension`] into a capability-gated [`SliceExtension`] registered under
/// `Slice.ext.<namespace>.invoke({ target, op, data })` or binary `Slice.ext.<namespace>.invoke(u8)`.
pub struct HostExtensionAdapter<H: SliceHostExtension> {
    namespace: &'static str,
    inner: H,
}

impl<H: SliceHostExtension> HostExtensionAdapter<H> {
    pub const fn new(namespace: &'static str, inner: H) -> Self {
        Self { namespace, inner }
    }
}

impl<H: SliceHostExtension> SliceExtension for HostExtensionAdapter<H> {
    fn name(&self) -> &'static str {
        self.namespace
    }

    fn methods(&self) -> &'static [ExtMethod] {
        HOST_EXT_ADAPTER_METHODS
    }

    fn call(&self, _ctx: &ExtCallCtx, method: &str, arg: ExtValue) -> ExtResult {
        if method != "invoke" {
            return Err(ExtError::Denied(format!("{}.{method}", self.namespace)));
        }
        let mut output = Vec::new();
        match arg {
            ExtValue::Bytes(raw) => {
                // Binary framing: first 4 bytes LE op code (or 0 if shorter), rest is payload.
                let (op, payload) = if raw.len() >= 4 {
                    (
                        u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
                        &raw[4..],
                    )
                } else {
                    (0u32, raw.as_slice())
                };
                self.inner
                    .invoke(self.namespace, op, payload, &mut output)
                    .map_err(ExtError::Message)?;
                Ok(ExtValue::Bytes(output))
            }
            ExtValue::Json(v) => {
                let target = v
                    .get("target")
                    .and_then(|t| t.as_str())
                    .unwrap_or(self.namespace);
                let op = v.get("op").and_then(|o| o.as_u64()).unwrap_or(0) as u32;
                let input_bytes: Vec<u8> = match v.get("data") {
                    Some(serde_json::Value::String(s)) => s.as_bytes().to_vec(),
                    Some(serde_json::Value::Array(arr)) => arr
                        .iter()
                        .filter_map(|x| x.as_u64().map(|n| n as u8))
                        .collect(),
                    _ => Vec::new(),
                };
                let status = self
                    .inner
                    .invoke(target, op, &input_bytes, &mut output)
                    .map_err(ExtError::Message)?;
                Ok(ExtValue::Json(serde_json::json!({
                    "status": status,
                    "bytes": output,
                })))
            }
        }
    }
}
