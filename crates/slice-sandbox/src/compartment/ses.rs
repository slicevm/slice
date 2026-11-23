//! Secure ECMAScript (SES) Frozen Intrinsic Registry.

/// Zero-allocation static check for SES frozen intrinsics (covering intrinsic prototypes,
/// intrinsic constructors, singleton namespaces, and optional `globalThis.` prefixes).
#[inline(always)]
pub fn is_frozen_intrinsic(target: &str) -> bool {
    let clean = target.strip_prefix("globalThis.").unwrap_or(target);
    matches!(
        clean,
        "Object"
            | "Object.prototype"
            | "Array"
            | "Array.prototype"
            | "Function"
            | "Function.prototype"
            | "String"
            | "String.prototype"
            | "Number"
            | "Number.prototype"
            | "Boolean"
            | "Boolean.prototype"
            | "RegExp"
            | "RegExp.prototype"
            | "Error"
            | "Error.prototype"
            | "Promise"
            | "Promise.prototype"
            | "Map"
            | "Map.prototype"
            | "Set"
            | "Set.prototype"
            | "Date"
            | "Date.prototype"
            | "Uint8Array"
            | "Uint8Array.prototype"
            | "Reflect"
            | "JSON"
            | "Math"
    )
}
