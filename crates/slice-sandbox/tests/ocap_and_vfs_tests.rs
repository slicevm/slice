use crate::*;

#[test]
fn test_ses_lockdown_and_ocap_enforcement() {
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/app/data")
        .with_net_host("db.internal:5432")
        .with_env("APP_MODE", "edge");

    let mut tenant = TenantCompartment::new("tenant-alpha", 1, caps, 10_000, 65536);

    // 1. SES Prototype Pollution must be blocked
    assert_eq!(
        tenant.verify_property_mutation("Object.prototype"),
        Err(SandboxViolation::IntrinsicMutationBlocked {
            tenant_id: "tenant-alpha".into(),
            intrinsic: "Object.prototype".into()
        })
    );

    // 2. Scoped VFS read succeeds on /app/data, fails on /etc/passwd or traversal
    assert!(tenant.verify_fs_read("/app/data/config.json").is_ok());
    assert!(tenant.verify_fs_read("/etc/passwd").is_err());
    assert!(tenant.verify_fs_read("/app/data/../../etc/shadow").is_err());

    // 3. Scoped network authority
    assert!(tenant.verify_net_connect("db.internal:5432").is_ok());
    assert!(tenant.verify_net_connect("attacker.evil.com:443").is_err());

    // 4. Scoped env authority
    assert_eq!(tenant.verify_env_read("APP_MODE"), Ok("edge"));
    assert!(tenant.verify_env_read("AWS_SECRET_ACCESS_KEY").is_err());

    // 5. Fuel limit preemption
    assert!(tenant.consume_fuel(5_000).is_ok());
    assert!(tenant.consume_fuel(6_000).is_err());

    // 6. Memory ceiling enforcement
    assert!(tenant.alloc_memory(16_384).is_ok());
    assert!(matches!(
        tenant.alloc_memory(128_000),
        Err(SandboxViolation::MemoryCeilingExceeded { .. })
    ));
}

#[test]
fn test_phase_9a_compartment_layout_and_zero_alloc_intrinsics() {
    // Verify all 9 frozen SES intrinsics match via zero-allocation compile-time trie
    for intrinsic in [
        "Object.prototype",
        "Array.prototype",
        "Function.prototype",
        "Promise.prototype",
        "String.prototype",
        "Number.prototype",
        "Boolean.prototype",
        "RegExp.prototype",
        "Error.prototype",
    ] {
        assert!(is_frozen_intrinsic(intrinsic));
    }
    assert!(!is_frozen_intrinsic("CustomClass.prototype"));

    // Verify Windows backslash normalization still works via Cow::Owned fallback
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/app/data")
        .with_write_prefix("/tmp/out");
    let tenant = TenantCompartment::new("tenant-9a", 3, caps, 10_000, 65536);
    assert!(tenant.verify_fs_read(r"\app\data\file.json").is_ok());
    assert!(tenant.verify_fs_write(r"\tmp/out\result.txt").is_ok());
}

#[test]
fn test_canonical_vfs_resolver_lexical_normalization_and_boundary_guards() {
    assert_eq!(
        normalize_vfs_path_lexical(r"\\?\c:\Users\slice\.\data\..\config.json"),
        "C:/Users/slice/config.json"
    );
    assert_eq!(
        normalize_vfs_path_lexical("/tmp/./sub/../etc/../../etc/passwd"),
        "/etc/passwd"
    );
    assert_eq!(normalize_vfs_path_lexical("./a/b/../c/"), "a/c");

    // 1. Directory boundary enforcement: `/tmp` allows `/tmp` and `/tmp/file.txt`,
    //    but strictly blocks `/tmp_evil` and `/tmp/../etc/passwd`.
    assert!(vfs_prefix_matches("/tmp", "/tmp"));
    assert!(vfs_prefix_matches("/tmp/sub/file.txt", "/tmp"));
    assert!(!vfs_prefix_matches("/tmp_evil/file.txt", "/tmp"));
    assert!(!vfs_prefix_matches("/tmp/../etc/passwd", "/tmp"));

    // 2. Cross-platform `/tmp` <-> `std::env::temp_dir()` equivalence
    let os_tmp_child = std::env::temp_dir().join("slice_vfs_unit_check.txt");
    assert!(vfs_prefix_matches(&os_tmp_child.to_string_lossy(), "/tmp"));

    // 3. Relative `"."` prefix matching against workspace current_dir
    if let Ok(cwd) = std::env::current_dir() {
        let abs_in_cwd = cwd.join("Cargo.toml");
        assert!(vfs_prefix_matches(&abs_in_cwd.to_string_lossy(), "."));
        assert!(vfs_prefix_matches("Cargo.toml", &cwd.to_string_lossy()));
    }
}

#[test]
fn test_compartment_clone_and_rebind_lifecycle_invariants() {
    assert_eq!(std::mem::size_of::<HardwareProtectionDomain>(), 12);

    let mut primary = TenantCompartment::new(
        "tenant-primary",
        2,
        CapabilityToken::pure_compute(),
        100_000,
        65536,
    );
    let primary_pkey = primary.pkey_domain.pkey;
    assert!((1..=15).contains(&primary_pkey));
    assert_eq!(primary.pkey_domain.pkru_mask & 0b11, 0);
    assert_eq!(
        primary.pkey_domain.pkru_mask & (0b11u32 << (primary_pkey * 2)),
        0
    );

    // Cloning and dropping the clone must keep the primary compartment's domain intact
    {
        let cloned = primary.clone();
        cloned.pkey_domain.enter_domain();
        if primary.pkey_domain.hardware_enforced && cloned.pkey_domain.hardware_enforced {
            assert_ne!(
                cloned.pkey_domain.pkey, primary_pkey,
                "Hardware-enforced clone must own a distinct kernel PKEY"
            );
        }
    }

    primary.pkey_domain.enter_domain();
    for slot in 0..32u32 {
        primary.pkey_domain.rebind_slot(slot);
        let k = primary.pkey_domain.pkey;
        assert!((1..=15).contains(&k));
        assert_eq!(primary.pkey_domain.pkru_mask & 0b11, 0);
        assert_eq!(primary.pkey_domain.pkru_mask & (0b11u32 << (k * 2)), 0);
        primary.pkey_domain.enter_domain();
    }
}

#[test]
fn test_checkpoint_capability_toggling_and_max_limits() {
    // Disabling then re-enabling checkpoints must clear the implicit zero cap
    let reenabled = CapabilityToken::pure_compute()
        .with_checkpoints(false)
        .with_checkpoints(true);
    let tenant_reenabled = TenantCompartment::new("cp-toggle", 1, reenabled, 10_000, 65536);
    assert!(tenant_reenabled.verify_checkpoint(0).is_ok());
    assert!(tenant_reenabled.verify_checkpoint(10).is_ok());

    // Explicit non-zero cap must remain intact when checkpoints are enabled
    let bounded = CapabilityToken::pure_compute()
        .with_max_checkpoints(Some(2))
        .with_checkpoints(true);
    let tenant_bounded = TenantCompartment::new("cp-bounded", 2, bounded, 10_000, 65536);
    assert!(tenant_bounded.verify_checkpoint(0).is_ok());
    assert!(tenant_bounded.verify_checkpoint(1).is_ok());
    assert!(matches!(
        tenant_bounded.verify_checkpoint(2),
        Err(SandboxViolation::CheckpointDenied { .. })
    ));

    // Zero cap followed by with_checkpoints(true) must allow checkpoints again
    let from_zero_cap = CapabilityToken::pure_compute()
        .with_max_checkpoints(Some(0))
        .with_checkpoints(true);
    let tenant_from_zero = TenantCompartment::new("cp-zero", 3, from_zero_cap, 10_000, 65536);
    assert!(tenant_from_zero.verify_checkpoint(0).is_ok());
}

#[test]
fn test_vfs_drive_relative_device_namespaces_catchall_routes_and_dot_prefix() {
    let broad = CapabilityToken::pure_compute()
        .with_read_prefix("/")
        .with_read_prefix("C:/");
    let tenant_broad = TenantCompartment::new("vfs-edge", 1, broad, 10_000, 65536);

    // Reject Windows drive-relative paths, NT native prefixes, and non-drive device namespaces
    for rejected in [
        "C:secret.txt",
        "c:../secret.txt",
        "D:",
        r"\??\C:\Windows\win.ini",
        "/??/C:/Windows/win.ini",
        "//?/GLOBALROOT/Device/HarddiskVolume1/secret",
        r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\secret",
        "//?/UNC/fileserver/share/secret.txt",
        r"\\?\C:relative_escape.txt",
    ] {
        assert!(
            matches!(
                tenant_broad.verify_fs_read(rejected),
                Err(SandboxViolation::FsReadDenied { .. })
            ),
            "expected {} to be rejected by VFS canonicalizer",
            rejected
        );
    }

    // Extended-length absolute drive root (`\\?\C:\...`) remains valid when `C:/` is granted
    assert!(tenant_broad
        .verify_fs_read(r"\\?\C:\allowed\data.json")
        .is_ok());

    // Catch-all framework route segments (`[...slug]`) and `foo..bar` filenames are allowed,
    // while actual `..` parent traversal segments remain blocked.
    let app_caps = CapabilityToken::pure_compute().with_read_prefix("/app");
    let tenant_app = TenantCompartment::new("vfs-routes", 2, app_caps, 10_000, 65536);
    assert!(tenant_app
        .verify_fs_read("/app/blog/[...slug]/page.tsx")
        .is_ok());
    assert!(tenant_app
        .verify_fs_read("/app/docs/[[...catchall]]/route.ts")
        .is_ok());
    assert!(tenant_app.verify_fs_read("/app/archive..v2.tar.gz").is_ok());
    assert!(tenant_app.verify_fs_read("/app/../etc/passwd").is_err());
    assert!(tenant_app.verify_fs_read("/app/blog/..").is_err());

    // POSIX double-slash root path (`//file.txt`) normalizes to `/file.txt`
    let root_caps = CapabilityToken::pure_compute().with_read_prefix("/");
    let tenant_root = TenantCompartment::new("vfs-posix-root", 3, root_caps, 10_000, 65536);
    assert!(tenant_root.verify_fs_read("//file.txt").is_ok());

    // Relative `.` capability prefix authorizes relative project paths without granting `/` or `..`
    let dot_caps = CapabilityToken::pure_compute()
        .with_read_prefix(".")
        .with_read_prefix("");
    let tenant_dot = TenantCompartment::new("vfs-dot", 4, dot_caps, 10_000, 65536);
    assert!(tenant_dot.verify_fs_read(".").is_ok());
    assert!(tenant_dot.verify_fs_read("./src/index.ts").is_ok());
    assert!(tenant_dot.verify_fs_read("Cargo.toml").is_ok());
    assert!(tenant_dot.verify_fs_read("app/[...slug]/page.tsx").is_ok());
    assert!(tenant_dot.verify_fs_read("/etc/passwd").is_err());
    assert!(tenant_dot.verify_fs_read("../outside.txt").is_err());

    // Empty capability prefix alone must never match relative paths
    let empty_caps = CapabilityToken::pure_compute().with_read_prefix("");
    let tenant_empty = TenantCompartment::new("vfs-empty", 5, empty_caps, 10_000, 65536);
    assert!(tenant_empty.verify_fs_read("Cargo.toml").is_err());
    assert!(tenant_empty.verify_fs_read(".").is_err());
}
