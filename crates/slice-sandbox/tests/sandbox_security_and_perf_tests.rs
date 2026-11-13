use crate::*;
use std::borrow::Cow;
use std::time::Instant;

#[test]
fn test_sbx_sec_01_vfs_unc_device_and_null_byte_injection_blocked() {
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix(".")
        .with_read_prefix("/app/data")
        .with_write_prefix("/tmp");
    let tenant = TenantCompartment::new("tenant-sec01", 1, caps, 10_000, 65536);

    // 1. Null-byte injection must be rejected even under matching prefix
    assert!(tenant
        .verify_fs_read("/app/data/safe.json\0/../../etc/passwd")
        .is_err());
    assert!(!vfs_prefix_matches("/app/data/safe.json\0", "/app/data"));

    // 2. Win32 device namespace (`\\.\` and `//./`) must be rejected
    assert!(tenant.verify_fs_read(r"\\.\PhysicalDrive0").is_err());
    assert!(tenant.verify_fs_read("//./pipe/docker_engine").is_err());

    // 3. Win32 extended UNC share (`\\?\UNC\server\share`) and raw SMB UNC (`\\server\share`)
    //    must be rejected and never treated as relative to "."
    assert!(tenant
        .verify_fs_read(r"\\?\UNC\evil.example.com\share\secret.txt")
        .is_err());
    assert!(tenant
        .verify_fs_read("//?/unc/evil.example.com/share/secret.txt")
        .is_err());
    assert!(tenant
        .verify_fs_read(r"\\evil.example.com\share\secret.txt")
        .is_err());

    // 4. Legitimate Win32 extended local drive (`\\?\C:\...`) normalization still works
    assert_eq!(
        normalize_vfs_path_lexical(r"\\?\c:\app\data\config.json"),
        "C:/app/data/config.json"
    );
}

#[test]
fn test_sbx_sec_02_verify_net_connect_strict_port_isolation() {
    // Compartment allowlisted ONLY for `api.internal:443`
    let caps = CapabilityToken::pure_compute().with_net_host("api.internal:443");
    let tenant = TenantCompartment::new("tenant-sec02", 2, caps, 10_000, 65536);

    // Exact port 443 and `https://` URL (default port 443) must succeed
    assert!(tenant.verify_net_connect("api.internal:443").is_ok());
    assert!(tenant
        .verify_net_connect("https://api.internal/v1/status")
        .is_ok());

    // Different explicit ports (`:6379`, `:22`, `:80`, `:8080`) on the same host MUST be denied
    assert!(
        tenant.verify_net_connect("api.internal:6379").is_err(),
        "Expected port 6379 to be denied when only api.internal:443 is allowlisted"
    );
    assert!(
        tenant.verify_net_connect("api.internal:22").is_err(),
        "Expected port 22 to be denied when only api.internal:443 is allowlisted"
    );
    assert!(
        tenant
            .verify_net_connect("http://api.internal:8080/admin")
            .is_err(),
        "Expected port 8080 to be denied when only api.internal:443 is allowlisted"
    );
    // Plaintext `http://` URL (default port 80) must not match `:443`-only allowlist
    assert!(
        tenant
            .verify_net_connect("http://api.internal/v1/status")
            .is_err(),
        "Expected plaintext http:// (port 80) to be denied when only :443 is allowlisted"
    );

    // Compartment allowlisted with bare hostname (`"service.internal"`) permits any port on that host
    let bare_caps = CapabilityToken::pure_compute().with_net_host("service.internal");
    let bare_tenant = TenantCompartment::new("tenant-bare", 3, bare_caps, 10_000, 65536);
    assert!(bare_tenant
        .verify_net_connect("service.internal:443")
        .is_ok());
    assert!(bare_tenant
        .verify_net_connect("service.internal:8080")
        .is_ok());
}

#[test]
fn test_sbx_sec_03_ssrf_trailing_dot_ipv6_zone_and_transition_tunnels_blocked() {
    // 1. Trailing FQDN dot on metadata and localhost domains
    assert!(is_cloud_metadata_ssrf_target(
        "http://metadata.google.internal./computeMetadata/v1/"
    ));
    assert!(is_cloud_metadata_ssrf_target(
        "http://sub.metadata.google.internal.:80/"
    ));
    assert!(is_private_or_loopback_ssrf_target(
        "http://localhost.:8080/"
    ));
    assert!(is_private_or_loopback_ssrf_target("http://api.localhost./"));

    // 2. IPv6 zone IDs (`[fe80::1%eth0]` and `[::1%lo0]`)
    assert!(is_private_or_loopback_ssrf_target(
        "http://[fe80::1%eth0]:8080/"
    ));
    assert!(is_private_or_loopback_ssrf_target("http://[::1%lo0]:3000/"));

    // 3. IPv4-compatible IPv6 (`::169.254.169.254`), NAT64 (`64:ff9b::169.254.169.254`),
    //    and 6to4 (`2002:a9fe:a9fe::1` = 169.254.169.254, `2002:7f00:0001::1` = 127.0.0.1)
    assert!(is_cloud_metadata_ssrf_target("http://[::169.254.169.254]/"));
    assert!(is_cloud_metadata_ssrf_target(
        "http://[64:ff9b::169.254.169.254]/"
    ));
    assert!(is_cloud_metadata_ssrf_target("http://[2002:a9fe:a9fe::1]/"));
    assert!(is_private_or_loopback_ssrf_target("http://[::127.0.0.1]/"));
    assert!(is_private_or_loopback_ssrf_target(
        "http://[64:ff9b::10.0.0.1]/"
    ));
    assert!(is_private_or_loopback_ssrf_target(
        "http://[2002:7f00:0001::1]/"
    ));
}

#[test]
fn test_sbx_sec_04_and_05_dotenv_quoting_and_exec_binary_metacharacters() {
    // Single-quoted `.env` values preserve literal backslashes while double-quoted values unescape
    let dotenv_src = r#"
LITERAL_SECRET_KEY='raw\nvalue\"kept'
ESCAPED_SECRET_KEY="line1\nline2\"quoted\\"
"#;
    let (pairs, secrets) = parse_dotenv_content(dotenv_src);
    assert_eq!(pairs[0].1, r#"raw\nvalue\"kept"#);
    assert_eq!(pairs[1].1, "line1\nline2\"quoted\\");
    assert_eq!(secrets.len(), 2);

    // `exec_binary_matches` blocks shell metacharacters and control chars even under wildcard `*`
    for injected in [
        "git;id",
        "git|cat /etc/passwd",
        "git&whoami",
        "git`id`",
        "git$IFS",
        "git>out",
        "git<in",
        "git\nid",
        "git\0--version",
    ] {
        assert!(
            !exec_binary_matches(injected, "*"),
            "Expected shell metacharacter binary '{}' to be blocked even under '*'",
            injected
        );
    }
    assert!(exec_binary_matches("git", "/usr/bin/git"));
    assert!(!exec_binary_matches("/tmp/evil/git", "/usr/bin/git"));
}

#[test]
fn test_sbx_perf_zero_alloc_hot_paths_and_throughput() {
    // 1. Verify `normalize_vfs_path_cow` borrows already-canonical paths with zero allocation
    assert!(matches!(
        normalize_vfs_path_cow("/app/data/users/profile.json"),
        Cow::Borrowed("/app/data/users/profile.json")
    ));
    assert!(matches!(
        normalize_vfs_path_cow("C:/Users/slice/config.json"),
        Cow::Borrowed("C:/Users/slice/config.json")
    ));

    // 2. Verify `extract_clean_hostname_and_port` borrows lowercase hostnames with zero allocation
    let (host_cow, port, scheme_port) =
        extract_clean_hostname_and_port("https://api.stripe.com:443/v1/charges");
    assert!(matches!(host_cow, Cow::Borrowed("api.stripe.com")));
    assert_eq!(port, Some(443));
    assert_eq!(scheme_port, Some(443));

    // 3. Benchmark 100,000 iterations of OCap VFS + Net + SSRF verification
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/app/data")
        .with_write_prefix("/tmp/shards")
        .with_net_host("api.stripe.com:443")
        .with_env("NODE_ENV", "production");
    let tenant = TenantCompartment::new("bench-tenant", 1, caps, 50_000_000, 32 * 1024 * 1024);

    let iterations = 100_000usize;
    let start = Instant::now();
    for _ in 0..iterations {
        tenant
            .verify_fs_read(std::hint::black_box("/app/data/users/profile.json"))
            .unwrap();
        tenant
            .verify_net_connect(std::hint::black_box("https://api.stripe.com/v1/charges"))
            .unwrap();
        assert!(!is_private_or_loopback_ssrf_target(std::hint::black_box(
            "https://api.openai.com/v1/chat"
        )));
    }
    let elapsed = start.elapsed();
    println!(
        "[SBX-PERF] 100,000 iterations of (verify_fs_read + verify_net_connect + is_private_or_loopback_ssrf_target): {:?} ({:.1} ns/iter)",
        elapsed,
        elapsed.as_nanos() as f64 / (iterations as f64)
    );
}

#[test]
fn test_empty_capability_fast_reject_and_cached_tmp_prefix_equivalence() {
    let pure_tenant = TenantCompartment::new(
        "pure-tenant",
        2,
        CapabilityToken::pure_compute(),
        1_000_000,
        4 * 1024 * 1024,
    );
    assert!(pure_tenant
        .verify_fs_read("/app/data/../secret.json")
        .is_err());
    assert!(pure_tenant.verify_fs_write("/tmp/out.txt").is_err());
    assert!(pure_tenant
        .verify_net_connect("https://api.example.com:443/v1")
        .is_err());
    assert!(pure_tenant.verify_exec_binary("git").is_err());

    let tmp_sub = std::env::temp_dir()
        .join("slice_sbx_tmp_equiv_probe")
        .join("item.txt");
    let tmp_caps =
        CapabilityToken::pure_compute().with_write_prefix("/tmp/slice_sbx_tmp_equiv_probe");
    let tmp_tenant = TenantCompartment::new("tmp-tenant", 3, tmp_caps, 1_000_000, 4 * 1024 * 1024);
    assert!(tmp_tenant
        .verify_fs_write(&tmp_sub.to_string_lossy())
        .is_ok());
    let outside_tmp = std::env::temp_dir().join("slice_sbx_tmp_equiv_probe_sibling.txt");
    assert!(tmp_tenant
        .verify_fs_write(&outside_tmp.to_string_lossy())
        .is_err());
}
