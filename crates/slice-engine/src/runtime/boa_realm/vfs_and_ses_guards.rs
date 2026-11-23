//! OCap VFS Read/Write Guards, Tenant VFS Affinity, and SES Prototype Shape Fingerprints.

use super::{
    BOA_ACTIVE_CAPS, BOA_ACTIVE_TENANT, BOA_VFS_OWNER_TENANT, BOA_VFS_STATE, BOA_VIOLATION,
    DIRTY_VFS, SES_INTRINSIC_COUNT, SesProtoSig, WarmBoaRealm, mark_subsystem_dirty,
};
use slice_sandbox::SandboxViolation;

#[inline]
pub(crate) fn norm_vfs_path(p: &str) -> String {
    slice_sandbox::normalize_vfs_path_lexical(p)
}

#[cold]
#[inline(never)]
pub(crate) fn record_boa_fs_violation(tenant_id: &str, path: &str) -> bool {
    BOA_VIOLATION.with(|v| {
        let mut slot = v.borrow_mut();
        if slot.is_none() {
            *slot = Some(SandboxViolation::FsReadDenied {
                tenant_id: tenant_id.to_string(),
                path: path.to_string(),
            });
        }
    });
    false
}

#[cold]
#[inline(never)]
pub(crate) fn record_boa_fs_write_violation(tenant_id: &str, path: &str) -> bool {
    BOA_VIOLATION.with(|v| {
        let mut slot = v.borrow_mut();
        if slot.is_none() {
            *slot = Some(SandboxViolation::FsWriteDenied {
                tenant_id: tenant_id.to_string(),
                path: path.to_string(),
            });
        }
    });
    false
}

#[cold]
#[inline(never)]
fn current_boa_tenant_or_default() -> String {
    BOA_ACTIVE_TENANT.with(|t| {
        let b = t.borrow();
        if b.is_empty() {
            "boa-tenant".to_string()
        } else {
            b.clone()
        }
    })
}

/// Returns `true` when the active compartment has at least one `--allow-read` prefix configured.
#[inline]
pub(crate) fn has_any_boa_fs_read_ocap() -> bool {
    BOA_ACTIVE_CAPS.with(|c| !c.borrow().read_prefixes.is_empty())
}

/// Returns `true` when the active compartment has at least one `--allow-write` prefix configured.
#[inline]
pub(crate) fn has_any_boa_fs_write_ocap() -> bool {
    BOA_ACTIVE_CAPS.with(|c| !c.borrow().write_prefixes.is_empty())
}

/// Side-effect-free predicate checking whether the active compartment currently holds OCap read
/// authority for `path` without recording a `SandboxViolation` when denied.
#[inline]
pub(crate) fn has_boa_fs_read_ocap(path: &str) -> bool {
    BOA_ACTIVE_CAPS
        .with(|c| slice_sandbox::vfs_path_matches_any_prefix(path, &c.borrow().read_prefixes))
}

/// Side-effect-free predicate checking whether the active compartment currently holds OCap write
/// authority for `path` without recording a `SandboxViolation` when denied.
#[inline]
pub(crate) fn has_boa_fs_write_ocap(path: &str) -> bool {
    BOA_ACTIVE_CAPS
        .with(|c| slice_sandbox::vfs_path_matches_any_prefix(path, &c.borrow().write_prefixes))
}

#[inline]
pub(crate) fn check_boa_fs_read_ocap(path: &str) -> bool {
    if !has_boa_fs_read_ocap(path) {
        let tenant = current_boa_tenant_or_default();
        return record_boa_fs_violation(&tenant, path);
    }
    true
}

#[inline]
pub(crate) fn check_boa_fs_write_ocap(path: &str) -> bool {
    if !has_boa_fs_write_ocap(path) {
        let tenant = current_boa_tenant_or_default();
        return record_boa_fs_write_violation(&tenant, path);
    }
    mark_subsystem_dirty(DIRTY_VFS);
    true
}

#[inline]
pub(crate) fn vfs_owner_namespace(tenant_id: &str) -> &str {
    tenant_id.strip_suffix(":sub").unwrap_or(tenant_id)
}

#[inline]
pub(crate) fn ensure_vfs_tenant_affinity(vfs_ns: &str) {
    BOA_VFS_OWNER_TENANT.with(|owner| {
        let mut cur = owner.borrow_mut();
        if cur.as_str() != vfs_ns {
            BOA_VFS_STATE.with(|v| v.borrow_mut().clear());
            cur.clear();
            cur.push_str(vfs_ns);
        }
    });
}

pub(crate) const SES_PROTO_NAMES: [&str; SES_INTRINSIC_COUNT] = [
    "Object.prototype",
    "Array.prototype",
    "Function.prototype",
    "String.prototype",
    "Promise.prototype",
    "Uint8Array.prototype",
    "Number.prototype",
    "Boolean.prototype",
    "RegExp.prototype",
    "Error.prototype",
    "Map.prototype",
    "Set.prototype",
    "Date.prototype",
    "Object",
    "Array",
    "Function",
    "String",
    "Number",
    "Boolean",
    "RegExp",
    "Error",
    "Promise",
    "Map",
    "Set",
    "Uint8Array",
    "Reflect",
    "JSON",
    "Math",
    "Date",
];

#[inline]
pub(crate) fn proto_sig(obj: &boa_engine::JsObject) -> SesProtoSig {
    let b = obj.borrow();
    let proto_addr = b
        .prototype()
        .as_ref()
        .map_or(0, |p| std::ptr::from_ref(&*p.borrow()) as usize);
    (
        b.shape().id(),
        proto_addr,
        b.properties().storage_fingerprint(),
    )
}

#[inline]
pub(crate) fn capture_ses_proto_sigs(
    ctx: &boa_engine::Context,
) -> [SesProtoSig; SES_INTRINSIC_COUNT] {
    let c = ctx.intrinsics().constructors();
    let o = ctx.intrinsics().objects();
    [
        proto_sig(&c.object().prototype()),
        proto_sig(&c.array().prototype()),
        proto_sig(&c.function().prototype()),
        proto_sig(&c.string().prototype()),
        proto_sig(&c.promise().prototype()),
        proto_sig(&c.typed_uint8_array().prototype()),
        proto_sig(&c.number().prototype()),
        proto_sig(&c.boolean().prototype()),
        proto_sig(&c.regexp().prototype()),
        proto_sig(&c.error().prototype()),
        proto_sig(&c.map().prototype()),
        proto_sig(&c.set().prototype()),
        proto_sig(&c.date().prototype()),
        proto_sig(&c.object().constructor()),
        proto_sig(&c.array().constructor()),
        proto_sig(&c.function().constructor()),
        proto_sig(&c.string().constructor()),
        proto_sig(&c.number().constructor()),
        proto_sig(&c.boolean().constructor()),
        proto_sig(&c.regexp().constructor()),
        proto_sig(&c.error().constructor()),
        proto_sig(&c.promise().constructor()),
        proto_sig(&c.map().constructor()),
        proto_sig(&c.set().constructor()),
        proto_sig(&c.typed_uint8_array().constructor()),
        proto_sig(&o.reflect()),
        proto_sig(&o.json()),
        proto_sig(&o.math()),
        proto_sig(&c.date().constructor()),
    ]
}

#[inline]
pub(crate) fn first_ses_proto_sig_mismatch_idx(realm: &WarmBoaRealm) -> Option<usize> {
    let cur = capture_ses_proto_sigs(&realm.ctx);
    cur.iter()
        .zip(realm.ses_proto_sigs.iter())
        .position(|(a, b)| a != b)
}
