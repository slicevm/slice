use criterion::{criterion_group, criterion_main, Criterion};
use slice_sandbox::{
    is_cloud_metadata_ssrf_target, is_private_or_loopback_ssrf_target, parse_dotenv_content,
    parse_inline_directives, CapabilityToken, TenantCompartment,
};
use std::hint::black_box;

fn bench_ocap_verification(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice_sandbox::ocap_verification");
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/app/data")
        .with_write_prefix("/tmp/shards")
        .with_net_host("api.stripe.com:443")
        .with_env("NODE_ENV", "production");
    let tenant = TenantCompartment::new("bench-tenant", 1, caps, 50_000_000, 32 * 1024 * 1024);

    group.bench_function("verify_fs_read_allowed", |b| {
        b.iter(|| {
            let res = tenant.verify_fs_read(black_box("/app/data/users/profile.json"));
            let _ = black_box(res);
        });
    });

    group.bench_function("verify_fs_read_traversal_blocked", |b| {
        b.iter(|| {
            let res = tenant.verify_fs_read(black_box("/app/data/../../etc/passwd"));
            let _ = black_box(res);
        });
    });

    group.bench_function("verify_net_connect_allowed", |b| {
        b.iter(|| {
            let res = tenant.verify_net_connect(black_box("api.stripe.com:443"));
            let _ = black_box(res);
        });
    });

    group.finish();
}

fn bench_ssrf_and_directives(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice_sandbox::ssrf_and_directives");

    group.bench_function("ssrf_shield_check_url", |b| {
        b.iter(|| {
            let meta = is_cloud_metadata_ssrf_target(black_box("https://api.openai.com/v1/chat"));
            let priv_net =
                is_private_or_loopback_ssrf_target(black_box("https://api.openai.com/v1/chat"));
            black_box((meta, priv_net))
        });
    });

    let sample_src = r#"#!/usr/bin/env slice
// @slice-tier: tier1
// @slice-boacas: true
// @slice-fuel: 25_000_000
// @slice-allow-net: ["api.stripe.com:443"]
export default { fetch(req) { return new Response("ok"); } };
"#;
    group.bench_function("parse_inline_directives", |b| {
        b.iter(|| {
            let d = parse_inline_directives(black_box(sample_src));
            black_box(d.fuel_limit)
        });
    });

    let sample_dotenv = "STRIPE_SECRET_KEY=\"sk_live_998877665544332211\"\nPUBLIC_KEY=\"pk_live_123\"\nREGION=us-east-1\n";
    group.bench_function("parse_dotenv_auto_dlp", |b| {
        b.iter(|| {
            let (pairs, secrets) = parse_dotenv_content(black_box(sample_dotenv));
            black_box((pairs.len(), secrets.len()))
        });
    });

    group.finish();
}

criterion_group!(benches, bench_ocap_verification, bench_ssrf_and_directives);
criterion_main!(benches);
