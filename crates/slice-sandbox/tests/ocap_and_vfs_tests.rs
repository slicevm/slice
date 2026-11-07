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

