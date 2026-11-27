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
        virtual_mounts: None,
        allow_net: Some(&req_escalate_port),
        allow_private_networks: None,
        allow_exec: Some(&["git".to_string()]),
        allow_ext: None,
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

#[test]
fn test_v0147_ssrf_ranges_loopback_private_net_attenuation_and_violation_display() {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    // 1. is_loopback_ip handles IPv4, IPv6, and IPv4-mapped IPv6 loopback strictly
    assert!(is_loopback_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
    assert!(is_loopback_ip(IpAddr::V4(Ipv4Addr::new(
        127, 255, 255, 254
    ))));
    assert!(is_loopback_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    assert!(is_loopback_ip("::ffff:127.0.0.1".parse().unwrap()));
    assert!(!is_loopback_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
    assert!(!is_loopback_ip(IpAddr::V4(Ipv4Addr::new(
        169, 254, 169, 254
    ))));
    assert!(!is_loopback_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));

    // 2. Extended RFC IPv6/IPv4 special-use ranges blocked by is_private_or_loopback_ip
    for special in [
        "192.88.99.1",
        "fec0::1",
        "100::1",
        "2001:2::1",
        "2001:10::1",
        "2001:20::1",
        "3fff::1",
    ] {
        let ip: IpAddr = special.parse().unwrap();
        assert!(
            is_private_or_loopback_ip(ip),
            "expected special-use IP {special} to be classified as private/non-global"
        );
    }
    assert!(!is_private_or_loopback_ip("1.1.1.1".parse().unwrap()));
    assert!(!is_private_or_loopback_ip(
        "2606:4700:4700::1111".parse().unwrap()
    ));

    // 3. Monotonic attenuation of allow_private_networks & manifest parsing
    let parent_default = CapabilityToken::pure_compute().with_net_host("api.example.com");
    assert!(!parent_default.allow_private_networks);
    let escalated = parent_default.attenuate(&CapabilityAttenuationRequest {
        allow_private_networks: Some(true),
        ..Default::default()
    });
    assert!(
        !escalated.allow_private_networks,
        "child must never escalate allow_private_networks when parent is false"
    );

    let parent_private = parent_default.with_allow_private_networks(true);
    assert!(
        parent_private
            .attenuate(&CapabilityAttenuationRequest::default())
            .allow_private_networks
    );
    assert!(
        !parent_private
            .attenuate(&CapabilityAttenuationRequest {
                allow_private_networks: Some(false),
                ..Default::default()
            })
            .allow_private_networks
    );

    let m = SliceManifest::parse_toml(
        "[capabilities]\nallow_net = [\"intranet.corp\"]\nallow_private_networks = true\n",
    );
    assert!(m.capabilities.allow_private_networks);

    // 4. Issue #220: SandboxViolation::Display never leaks compartment names and uses one OCap prefix
    let violations = [
        SandboxViolation::FsReadDenied {
            tenant_id: "secret-host-compartment".to_string(),
            path: "/etc/hostname".to_string(),
        },
        SandboxViolation::FsWriteDenied {
            tenant_id: "secret-host-compartment".to_string(),
            path: "/tmp/x".to_string(),
        },
        SandboxViolation::NetAccessDenied {
            tenant_id: "secret-host-compartment".to_string(),
            host: "example.com:443".to_string(),
        },
        SandboxViolation::EnvAccessDenied {
            tenant_id: "secret-host-compartment".to_string(),
            key: "SECRET_KEY".to_string(),
        },
        SandboxViolation::ExecDenied {
            tenant_id: "secret-host-compartment".to_string(),
            binary: "/bin/sh".to_string(),
        },
        SandboxViolation::CheckpointDenied {
            tenant_id: "secret-host-compartment".to_string(),
            reason: "disabled".to_string(),
        },
    ];
    for v in &violations {
        let s = v.to_string();
        assert!(
            s.starts_with("OCap Violation: "),
            "expected unified OCap Violation prefix, got: {s}"
        );
        assert!(
            !s.contains("secret-host-compartment") && !s.contains("Compartment '"),
            "violation Display must not leak host compartment name: {s}"
        );
        assert_eq!(v.tenant_id(), "secret-host-compartment");
    }
}

#[test]
fn test_v0150_virtual_mounts_attenuation_and_manifest_parsing() {
    let parent = CapabilityToken::pure_compute()
        .with_virtual_mount("/artifacts")
        .with_virtual_mount("/scratch/work");
    let comp = TenantCompartment::new("vm-parent", 1, parent.clone(), 10_000, 65536);

    assert!(comp.verify_fs_read("/artifacts/report.png").is_ok());
    assert!(comp.verify_fs_write("/artifacts/report.png").is_ok());
    assert!(comp.verify_fs_read("/scratch/work/tmp.txt").is_ok());
    assert!(comp.verify_fs_read("/etc/passwd").is_err());
    assert!(comp.verify_fs_write("/tmp/outside.txt").is_err());

    // Attenuation can narrow a virtual mount, never widen it, and never promote it to host read/write.
    let req_narrow = [
        "/artifacts/sub".to_string(),
        "/artifacts/../../etc".to_string(),
        "/other".to_string(),
    ];
    let req_host_escalate = ["/artifacts".to_string()];
    let child = parent.attenuate(&CapabilityAttenuationRequest {
        allow_read: Some(&req_host_escalate),
        allow_write: Some(&req_host_escalate),
        virtual_mounts: Some(&req_narrow),
        ..Default::default()
    });
    assert!(
        child.allowed_read_prefixes.is_empty(),
        "virtual mount must never attenuate into host read grant"
    );
    assert!(
        child.allowed_write_prefixes.is_empty(),
        "virtual mount must never attenuate into host write grant"
    );
    assert_eq!(child.virtual_mounts, vec!["/artifacts/sub".to_string()]);

    let child_comp = TenantCompartment::new("vm-child", 2, child, 10_000, 65536);
    assert!(child_comp.verify_fs_write("/artifacts/sub/ok.txt").is_ok());
    assert!(child_comp.verify_fs_write("/artifacts/top.txt").is_err());

    // Manifest parsing for virtual_mount / virtual_mounts and [run] artifacts_dir
    let toml_src = r#"
[run]
artifacts_dir = "./out/artifacts"

[capabilities]
virtual_mounts = ["/artifacts", "/work/scratch"]
"#;
    let manifest = SliceManifest::parse_toml(toml_src);
    assert_eq!(manifest.artifacts_dir.as_deref(), Some("./out/artifacts"));
    assert_eq!(
        manifest.capabilities.virtual_mounts,
        vec!["/artifacts".to_string(), "/work/scratch".to_string()]
    );
}

#[test]
fn test_v0150_extension_cap_attenuation_and_manifest_parsing() {
    let parent = CapabilityToken::pure_compute()
        .with_extension("store")
        .with_extension("telemetry.emit")
        .with_extension("*"); // wildcard must be ignored
    assert_eq!(
        parent.allowed_extensions,
        vec!["store".to_string(), "telemetry.emit".to_string()]
    );

    let comp = TenantCompartment::new("ext-parent", 1, parent.clone(), 10_000, 65536);
    assert!(comp.verify_extension("store.get").is_ok());
    assert!(comp.verify_extension("store.put").is_ok());
    assert!(comp.verify_extension("telemetry.emit").is_ok());
    assert!(comp.verify_extension("telemetry.flush").is_err());
    assert!(comp.verify_extension("unknown.call").is_err());

    let req = [
        "store.get".to_string(),
        "telemetry".to_string(), // cannot widen telemetry.emit -> telemetry
        "other".to_string(),
        "*".to_string(),
    ];
    let child = parent.attenuate(&CapabilityAttenuationRequest {
        allow_ext: Some(&req),
        ..Default::default()
    });
    assert_eq!(child.allowed_extensions, vec!["store.get".to_string()]);
    let child_comp = TenantCompartment::new("ext-child", 2, child, 10_000, 65536);
    assert!(child_comp.verify_extension("store.get").is_ok());
    assert!(child_comp.verify_extension("store.put").is_err());

    let m =
        SliceManifest::parse_toml("[capabilities]\nallow_ext = [\"store\", \"fn.call\", \"*\"]\n");
    assert_eq!(
        m.capabilities.allowed_extensions,
        vec!["store".to_string(), "fn.call".to_string()]
    );
}

#[test]
fn test_v0150_rolling_egress_dlp_cross_chunk_secret_detection() {
    let mut expanded = Vec::new();
    expand_egress_secret_variants("sk_live_split_secret_42", &mut expanded);

    // 1. Raw secret split across two adjacent chunks is detected
    let mut scanner = RollingEgressDlpScanner::new("stream-tenant", expanded.clone());
    assert!(scanner.scan_chunk(b"data: hello sk_live_split").is_ok());
    let err = scanner.scan_chunk(b"_secret_42\n\n").unwrap_err();
    assert!(matches!(
        err,
        SandboxViolation::EgressSecretLeakBlocked { .. }
    ));

    // 2. Base64-encoded secret split across two chunks is detected
    let b64 = "c2tfbGl2ZV9zcGxpdF9zZWNyZXRfNDI=";
    let (p1, p2) = b64.split_at(12);
    let mut scanner_b64 = RollingEgressDlpScanner::new("stream-tenant", expanded.clone());
    assert!(scanner_b64.scan_chunk(p1.as_bytes()).is_ok());
    assert!(scanner_b64.scan_chunk(p2.as_bytes()).is_err());

    // 3. Clean multi-chunk stream passes with zero false positives
    let mut scanner_clean = RollingEgressDlpScanner::new("stream-tenant", expanded);
    for chunk in [
        b"data: chunk 1\n\n".as_slice(),
        b"data: chunk 2\n\n",
        b"data: done\n\n",
    ] {
        assert!(scanner_clean.scan_chunk(chunk).is_ok());
    }
}
