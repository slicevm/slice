use crate::vfs::{
    CachedOsTmpInfo, build_os_tmp_info, vfs_normalized_candidate_matches_prefix_with_tmp,
};
use crate::*;
use std::net::IpAddr;
use std::path::Path;

#[test]
fn test_wave4_capability_token_attenuate_port_lattice_and_wildcard_ssrf_containment() {
    // 1. Port-scoped parent ("API.Internal.:443") canonicalizes and NEVER escalates to bare all-ports ("api.internal")
    let parent_port_scoped = CapabilityToken::pure_compute()
        .with_net_host("API.Internal.:443")
        .with_net_host("[::1]")
        .with_net_host("[2001:DB8::1]:8443")
        .with_read_prefix("/app/data")
        .with_write_prefix("/tmp/work")
        .with_exec_binary("*")
        .with_env("STAGE", "production")
        .with_env("INTERNAL_TOKEN", "tok_secret_value")
        .with_max_checkpoints(Some(6));

    assert!(
        parent_port_scoped
            .allowed_net_hosts
            .contains("api.internal:443")
    );
    assert!(parent_port_scoped.allowed_net_hosts.contains("::1"));
    assert!(
        parent_port_scoped
            .allowed_net_hosts
            .contains("[2001:db8::1]:8443")
    );

    let req_escalate_port = ["api.internal".to_string()];
    let req_disjoint_read = ["/app".to_string(), "/etc".to_string()];
    let req_env = ["STAGE".to_string()];
    let child_escalate = parent_port_scoped.attenuate(&CapabilityAttenuationRequest {
        allow_read: Some(&req_disjoint_read),
        allow_write: Some(&[]),
        allow_net: Some(&req_escalate_port),
        allow_exec: Some(&["git".to_string()]),
        allow_env: Some(&req_env),
        allow_checkpoints: None,
        max_checkpoints: Some(10),
        consumed_checkpoints: 2,
    });
    assert!(
        child_escalate.allowed_net_hosts.is_empty(),
        "port-scoped api.internal:443 must not attenuate into bare all-ports api.internal"
    );
    assert!(
        child_escalate.allowed_read_prefixes.is_empty(),
        "broader /app or disjoint /etc must yield empty read set against /app/data"
    );
    assert!(child_escalate.allowed_write_prefixes.is_empty());
    assert_eq!(
        child_escalate.allowed_exec_binaries,
        vec!["git".to_string()]
    );
    assert_eq!(child_escalate.allowed_env_keys.len(), 1);
    assert_eq!(
        child_escalate
            .allowed_env_keys
            .get("STAGE")
            .map(String::as_str),
        Some("production")
    );
    assert!(child_escalate.allow_checkpoints);
    assert_eq!(
        child_escalate.max_checkpoints,
        Some(4),
        "remaining parent checkpoints (6 - 2 = 4) must clamp requested 10"
    );

    // Requesting exact ":443" against "api.internal:443" and narrowed "[::1]:9090" against "::1" succeeds
    let req_exact = [
        "api.internal:443".to_string(),
        "[::1]:9090".to_string(),
        "[2001:db8::1]".to_string(),
    ];
    let child_exact = parent_port_scoped.attenuate(&CapabilityAttenuationRequest {
        allow_net: Some(&req_exact),
        ..Default::default()
    });
    assert!(child_exact.allowed_net_hosts.contains("api.internal:443"));
    assert!(child_exact.allowed_net_hosts.contains("[::1]:9090"));
    assert!(
        !child_exact.allowed_net_hosts.contains("2001:db8::1"),
        "port-scoped [2001:db8::1]:8443 must not grant bare 2001:db8::1"
    );
    let comp_exact = TenantCompartment::new("child-exact", 1, child_exact, 10_000, 65536);
    assert!(comp_exact.verify_net_connect("api.internal:443").is_ok());
    assert!(comp_exact.verify_net_connect("api.internal:6379").is_err());
    assert!(comp_exact.verify_net_connect("[::1]:9090").is_ok());
    assert!(comp_exact.verify_net_connect("[::1]:6379").is_err());

    // 2. Wildcard "*" parent NEVER materializes explicit loopback, RFC1918, or metadata grants
    let parent_wildcard = CapabilityToken::pure_compute().with_net_host("*");
    let req_mixed = [
        "api.stripe.com:443".to_string(),
        "127.0.0.1:6379".to_string(),
        "localhost:8080".to_string(),
        "[::1]:8080".to_string(),
        "10.0.0.8:5432".to_string(),
        "192.168.1.1:80".to_string(),
        "169.254.169.254:80".to_string(),
        "168.63.129.16:80".to_string(),
        "127.0.0.08:80".to_string(),
    ];
    let child_from_wildcard = parent_wildcard.attenuate(&CapabilityAttenuationRequest {
        allow_net: Some(&req_mixed),
        ..Default::default()
    });
    assert_eq!(child_from_wildcard.allowed_net_hosts.len(), 1);
    assert!(
        child_from_wildcard
            .allowed_net_hosts
            .contains("api.stripe.com:443")
    );
}

#[test]
fn test_wave4_percent_decoded_delimiters_invalid_ports_leading_zero_ipv4_and_cloud_metadata() {
    let permissive = CapabilityToken::pure_compute()
        .with_net_host("*")
        .with_net_host("evil.com")
        .with_net_host("api.stripe.com")
        .with_net_host("::1");
    let tenant = TenantCompartment::new("wave4-ssrf", 2, permissive, 10_000, 65536);

    // 1. Percent-decoded authority delimiters, NUL, CRLF, and malformed percent escapes fail closed
    for malformed_host in [
        "169.254.169.254%40evil.com:80",
        "http://169.254.169.254%40evil.com/path",
        "evil.com%2f..:443",
        "evil.com%5cadmin:443",
        "evil.com%00.trusted.io:443",
        "evil.com%0d%0aX-Injected:1",
        "evil.com%3fquery:80",
        "evil.com%23frag:80",
        "evil.com%2",
        "evil.com%zz:80",
    ] {
        assert!(
            parse_net_connect_target(malformed_host).is_none(),
            "expected parse_net_connect_target to reject {}",
            malformed_host
        );
        assert_eq!(extract_clean_hostname(malformed_host), "");
        assert!(
            tenant.verify_net_connect(malformed_host).is_err(),
            "expected verify_net_connect to reject {}",
            malformed_host
        );
    }

    // 2. Port 0, out-of-range ports (> 65535), non-numeric ports, and trailing IPv6 junk fail closed
    for bad_port in [
        "api.stripe.com:0",
        "http://api.stripe.com:0/v1",
        "api.stripe.com:65536",
        "api.stripe.com:99999",
        "api.stripe.com:bad",
        "api.stripe.com:",
        "[::1]:0",
        "[::1]:65536",
        "[::1]:bad",
        "[::1]junk",
        "[::1",
    ] {
        assert!(
            parse_net_connect_target(bad_port).is_none(),
            "expected invalid port/bracket target {} to be rejected",
            bad_port
        );
        assert!(
            tenant.verify_net_connect(bad_port).is_err(),
            "expected verify_net_connect to deny {}",
            bad_port
        );
    }

    // 3. Leading-zero non-octal IPv4 ("127.0.0.08", "0169.0254.0169.0254") is blocked
    for non_octal in [
        "127.0.0.08",
        "127.0.0.09:8080",
        "http://127.0.0.08/admin",
        "0169.0254.0169.0254",
        "http://0169.0254.0169.0254/latest/meta-data/",
    ] {
        assert!(
            is_private_or_loopback_ssrf_target(non_octal),
            "expected {} to be classified as private/non-canonical SSRF target",
            non_octal
        );
        assert!(
            tenant.verify_net_connect(non_octal).is_err(),
            "expected verify_net_connect to deny {}",
            non_octal
        );
    }
    assert!(is_cloud_metadata_ssrf_target("0169.0254.0169.0254"));
    assert!(is_cloud_metadata_ssrf_target(
        "http://0169.0254.0169.0254:80/metadata"
    ));

    // 4. Azure WireServer (168.63.129.16) and AWS EKS Pod Identity (169.254.170.23, fd00:ec2::23)
    for meta_ip_str in [
        "168.63.129.16",
        "::ffff:168.63.129.16",
        "169.254.170.23",
        "::ffff:169.254.170.23",
        "fd00:ec2::23",
    ] {
        let ip: IpAddr = meta_ip_str.parse().unwrap();
        assert!(
            is_cloud_metadata_ip(ip),
            "expected cloud metadata IP for {}",
            meta_ip_str
        );
        assert!(is_private_or_loopback_ip(ip));
    }
    let explicit_meta_caps = CapabilityToken::pure_compute()
        .with_net_host("168.63.129.16:80")
        .with_net_host("169.254.170.23:80")
        .with_net_host("[fd00:ec2::23]:80");
    let explicit_meta_tenant =
        TenantCompartment::new("meta-blocked", 3, explicit_meta_caps, 10_000, 65536);
    for target in [
        "168.63.129.16:80",
        "http://168.63.129.16/machine/?comp=goalstate",
        "169.254.170.23:80",
        "http://169.254.170.23/v1/credentials",
        "[fd00:ec2::23]:80",
        "http://[fd00:ec2::23]/v1/credentials",
    ] {
        assert!(is_cloud_metadata_ssrf_target(target));
        assert!(explicit_meta_tenant.verify_net_connect(target).is_err());
    }
}

#[test]
fn test_wave4_vfs_dotdot_mixed_prefix_root_tmp_guard_bidirectional_tmp_and_win32_devices() {
    // 1. Mixed relative/absolute `..` collapse: "../__outside_escape__/passwd" must never match CWD
    let cwd = cached_normalized_cwd();
    assert!(
        !vfs_normalized_candidate_matches_prefix("../__outside_escape__/passwd", cwd),
        "relative parent escape must be lexically collapsed before prefix boundary check"
    );
    assert!(
        !vfs_normalized_candidate_matches_prefix(
            &format!("{}/../__outside_escape__/passwd", cwd),
            "."
        ),
        "uncollapsed /../ segment in candidate must never match relative '.' prefix"
    );

    // 2. TMPDIR=/ and TMP=C:\ root-escalation guards
    let root_posix_tmp = build_os_tmp_info(Path::new("/"));
    assert!(root_posix_tmp.primary.is_none());
    assert!(root_posix_tmp.lexical_alias.is_none());
    assert!(!vfs_normalized_candidate_matches_prefix_with_tmp(
        "/etc/passwd",
        "/tmp",
        &root_posix_tmp
    ));
    assert!(vfs_normalized_candidate_matches_prefix_with_tmp(
        "/tmp/isolated.txt",
        "/tmp",
        &root_posix_tmp
    ));

    let root_win_tmp = build_os_tmp_info(Path::new("C:/"));
    assert!(root_win_tmp.primary.is_none());
    assert!(!vfs_normalized_candidate_matches_prefix_with_tmp(
        "C:/Windows/System32/drivers/etc/hosts",
        "/tmp",
        &root_win_tmp
    ));

    // 3. Bidirectional canonical `/tmp` <-> `/private/var/folders/...` <-> `/var/folders/...`
    let macos_tmp = CachedOsTmpInfo {
        primary: Some("/private/var/folders/ab/cd/T".to_string()),
        lexical_alias: Some("/var/folders/ab/cd/T".to_string()),
    };
    assert!(vfs_normalized_candidate_matches_prefix_with_tmp(
        "/private/var/folders/ab/cd/T/job1/out.json",
        "/tmp/job1",
        &macos_tmp
    ));
    assert!(vfs_normalized_candidate_matches_prefix_with_tmp(
        "/tmp/job1/out.json",
        "/private/var/folders/ab/cd/T/job1",
        &macos_tmp
    ));
    assert!(vfs_normalized_candidate_matches_prefix_with_tmp(
        "/private/var/folders/ab/cd/T/job1/out.json",
        "/var/folders/ab/cd/T/job1",
        &macos_tmp
    ));
    assert!(!vfs_normalized_candidate_matches_prefix_with_tmp(
        "/private/var/folders/ab/cd/T/job2/secret.json",
        "/var/folders/ab/cd/T/job1",
        &macos_tmp
    ));

    // 4. Mixed-slash Win32 UNC and Win32 reserved device basenames blocked; normal filenames allowed
    let broad = CapabilityToken::pure_compute()
        .with_read_prefix("/")
        .with_read_prefix("C:/")
        .with_read_prefix(".");
    let tenant = TenantCompartment::new("vfs-win32", 4, broad, 10_000, 65536);

    for blocked in [
        r"\/evil.example.com\share\payload.dll",
        r"/\evil.example.com/share/payload.dll",
        "CON",
        "con.txt",
        "/tmp/AUX",
        "/app/logs/prn.log",
        "C:/app/NUL",
        "./com1",
        "/tmp/COM9.json",
        "/tmp/lpt1.dat",
        "CONIN$",
        "/tmp/conout$.txt",
        "/tmp/com\u{00b9}.txt",
    ] {
        assert!(
            is_rejected_vfs_candidate(blocked),
            "expected {} to be rejected by VFS guard",
            blocked
        );
        assert!(tenant.verify_fs_read(blocked).is_err());
    }

    for allowed in [
        "/app/console.js",
        "/app/context.ts",
        "/app/null.json",
        "/app/auxiliary.ts",
        "/app/com10.txt",
        "/app/routes/[...slug]/page.tsx",
        "//file.txt",
    ] {
        assert!(
            !is_rejected_vfs_candidate(allowed),
            "expected {} to remain allowed",
            allowed
        );
        assert!(tenant.verify_fs_read(allowed).is_ok());
    }
}

#[test]
fn test_wave4_multiline_manifest_arrays_memory_overflow_directive_quotes_and_sensitive_env() {
    // 1. Multi-line arrays with comments and overflow-clamped memory_kb / memory_mb
    let toml_src = r#"
[package]
name = "wave4-service"

[limits]
memory_kb = 18446744073709551615

[capabilities]
allow_read = [
    # leading comment
    "./public", # inline comment with [brackets]
    "/tmp/cache",
]
allow_net = [
    "API.Example.com.:443",
    "[2001:DB8::1]:8443",
]
audit_egress = [
    "sk_live_multi_line_1",
    "secret_with_[brackets]_2" # trailing comment
]
"#;
    let m = SliceManifest::parse_toml(toml_src);
    assert!(m.memory_limit_kb <= usize::MAX / 1024);
    assert_eq!(
        m.memory_limit_kb.saturating_mul(1024) / 1024,
        m.memory_limit_kb
    );
    assert_eq!(
        m.capabilities.allowed_read_prefixes,
        vec!["public".to_string(), "/tmp/cache".to_string()]
    );
    assert!(
        m.capabilities
            .allowed_net_hosts
            .contains("api.example.com:443")
    );
    assert!(
        m.capabilities
            .allowed_net_hosts
            .contains("[2001:db8::1]:8443")
    );
    assert_eq!(
        m.audit_egress,
        vec![
            "sk_live_multi_line_1".to_string(),
            "secret_with_[brackets]_2".to_string()
        ]
    );

    // 2. Inline directives preserve outer quotes on comma-containing items, strip `#` comments on scalars,
    //    and never read host `std::env::var` during parsing
    let dir_src = r#"// @slice-fuel: 250_000 # deterministic ops ceiling
// @slice-allow-read: "/data/a", "/data/b"
// @slice-allow-env: "FEATURES=alpha,beta", "PATH"
export default 1;
"#;
    let dirs = parse_inline_directives(dir_src);
    assert_eq!(dirs.fuel_limit, Some(250_000));
    assert_eq!(
        dirs.allow_read,
        vec!["/data/a".to_string(), "/data/b".to_string()]
    );
    assert_eq!(
        dirs.allow_env,
        vec![("FEATURES".to_string(), "alpha,beta".to_string())]
    );
    assert_eq!(dirs.allow_env_inherit, vec!["PATH".to_string()]);

    // 3. `is_sensitive_env_key`: `AUTH` and `SECRET`/`PRIVATE` override `PUBLIC_KEY`
    assert!(is_sensitive_env_key("BASIC_AUTH_HEADER"));
    assert!(is_sensitive_env_key("OAUTH_ACCESS"));
    assert!(is_sensitive_env_key("SECRET_PUBLIC_KEY_HMAC"));
    assert!(is_sensitive_env_key("PRIVATE_PUBLISHABLE_KEY_SEED"));
    assert!(!is_sensitive_env_key("NEXT_PUBLIC_KEY"));
    assert!(!is_sensitive_env_key("STRIPE_PUBLISHABLE_KEY"));
}
