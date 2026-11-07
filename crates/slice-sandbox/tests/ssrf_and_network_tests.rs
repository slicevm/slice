use crate::*;

#[test]
fn test_tag_f3_ssrf_shield_and_wildcard_private_network_blocking() {
    // 1. Wildcard `allow_net = ["*"]` allows public internet hosts, but blocks cloud metadata and RFC1918/loopback
    let wildcard_caps = CapabilityToken::pure_compute().with_net_host("*");
    let wildcard_tenant =
        TenantCompartment::new("tenant-wildcard", 4, wildcard_caps, 10_000, 65536);

    assert!(wildcard_tenant
        .verify_net_connect("api.openai.com:443")
        .is_ok());
    assert!(wildcard_tenant.verify_net_connect("api.stripe.com").is_ok());

    // Cloud instance metadata endpoints must be unconditionally blocked even under `*`
    for metadata_host in [
        "169.254.169.254",
        "169.254.169.254:80",
        "169.254.170.2:80",
        "100.100.100.200:80",
        "[fd00:ec2::254]:80",
        "metadata.google.internal",
        "metadata.google.internal:80",
        "metadata.goog:80",
    ] {
        assert!(
            matches!(
                wildcard_tenant.verify_net_connect(metadata_host),
                Err(SandboxViolation::NetAccessDenied { .. })
            ),
            "Expected SSRF block for metadata endpoint {}",
            metadata_host
        );
    }

    // Loopback and RFC1918 private networks must be blocked under `*`
    for private_host in [
        "localhost:8080",
        "127.0.0.1:6379",
        "0.0.0.0:3000",
        "[::1]:8080",
        "10.0.0.42:5432",
        "172.16.0.10:8080",
        "172.31.255.1:8080",
        "192.168.1.100:9000",
    ] {
        assert!(
            matches!(
                wildcard_tenant.verify_net_connect(private_host),
                Err(SandboxViolation::NetAccessDenied { .. })
            ),
            "Expected SSRF block for private/loopback host {}",
            private_host
        );
    }
    // Public 172.32.x.x is outside 172.16.0.0/12 and should be permitted under `*`
    assert!(wildcard_tenant.verify_net_connect("172.32.0.1:443").is_ok());

    // 2. Explicit local microservice allowlisting permits only that exact host:port
    let local_caps = CapabilityToken::pure_compute().with_net_host("127.0.0.1:18091");
    let local_tenant = TenantCompartment::new("tenant-local", 5, local_caps, 10_000, 65536);
    assert!(local_tenant.verify_net_connect("127.0.0.1:18091").is_ok());
    assert!(local_tenant.verify_net_connect("127.0.0.1:6379").is_err());
    assert!(local_tenant
        .verify_net_connect("169.254.169.254:80")
        .is_err());
}

#[test]
fn test_phase_n1_resolved_ip_ssrf_and_ipv4_mapped_ipv6_blocking() {
    use std::net::IpAddr;

    for meta_ip in [
        "169.254.169.254",
        "169.254.170.2",
        "100.100.100.200",
        "::ffff:169.254.169.254",
        "::ffff:100.100.100.200",
        "fd00:ec2::254",
    ] {
        let parsed: IpAddr = meta_ip.parse().unwrap();
        assert!(
            is_cloud_metadata_ip(parsed),
            "Expected cloud metadata classification for {}",
            meta_ip
        );
    }

    for priv_ip in [
        "127.0.0.1",
        "0.0.0.0",
        "10.0.0.42",
        "172.16.0.1",
        "172.31.255.254",
        "192.168.1.50",
        "100.64.0.1",
        "100.127.255.254",
        "169.254.1.10",
        "::1",
        "::",
        "fe80::1",
        "fc00::1",
        "fd12:3456:789a::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "::ffff:192.168.1.1",
    ] {
        let parsed: IpAddr = priv_ip.parse().unwrap();
        assert!(
            is_private_or_loopback_ip(parsed),
            "Expected private/loopback classification for {}",
            priv_ip
        );
    }

    for pub_ip in [
        "93.184.216.34",
        "1.1.1.1",
        "8.8.8.8",
        "172.32.0.1",
        "100.128.0.1",
        "2606:2800:220:1:248:1893:25c8:1946",
    ] {
        let parsed: IpAddr = pub_ip.parse().unwrap();
        assert!(!is_cloud_metadata_ip(parsed));
        assert!(!is_private_or_loopback_ip(parsed));
    }
}

#[test]
fn test_nsec8_noncanonical_ip_and_special_cidr_ssrf_blocking() {
    use std::net::IpAddr;

    // Octal, hex, dword, and shorthand metadata IPs must be caught as cloud metadata targets
    for meta_bypass in [
        "http://2852039166/latest/meta-data/",
        "http://0251.0376.0251.0376/",
        "http://0xa9.0xfe.0xa9.0xfe/",
        "http://169.254.43518/",
        "http://[::ffff:169.254.169.254]/",
    ] {
        assert!(
            is_cloud_metadata_ssrf_target(meta_bypass),
            "Expected cloud metadata detection for {}",
            meta_bypass
        );
    }

    // Octal, hex, dword, shorthand loopback/private, and special-use CIDRs must be blocked under wildcard allow_net
    for ssrf_bypass in [
        "http://0177.0.0.1:8080/",
        "http://0x7f.0.0.1/",
        "http://2130706433/",
        "http://127.1/",
        "http://10.1/",
        "http://sub.localhost:3000/",
        "http://[fc00::1]/",
        "http://[fe80::1]/",
        "http://100.64.0.1/",
        "http://198.18.0.1/",
        "http://192.0.0.1/",
        "http://224.0.0.1/",
        "http://[ff02::1]/",
    ] {
        assert!(
            is_private_or_loopback_ssrf_target(ssrf_bypass),
            "Expected SSRF target classification for {}",
            ssrf_bypass
        );
    }

    // Verify special-use IPs in `is_private_or_loopback_ip`
    for special_ip in [
        "192.0.0.1",
        "192.0.2.1",
        "198.18.0.1",
        "198.19.255.254",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "240.0.0.1",
        "ff02::1",
        "2001:db8::1",
    ] {
        let parsed: IpAddr = special_ip.parse().unwrap();
        assert!(
            is_private_or_loopback_ip(parsed),
            "Expected special-use IP blocking for {}",
            special_ip
        );
    }
}

