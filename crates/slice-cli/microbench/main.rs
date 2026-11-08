use criterion::{criterion_group, criterion_main, Criterion};
use slice_sandbox::{parse_inline_directives, SliceManifest};
use std::hint::black_box;

fn bench_cli_config_layers(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice_cli::config_layers");

    let manifest_toml = r#"
[package]
name = "edge-api"
version = "1.0.0"
entrypoint = "./src/index.ts"

[engine]
tier = "tier1"
boacas = true
warm = true

[limits]
fuel = 25_000_000
memory_mb = 32

[capabilities]
allow_read = ["./config", "/tmp"]
allow_net = ["api.stripe.com:443"]

[server]
port = 8080
workers = 4
"#;

    let inline_src = r#"#!/usr/bin/env slice
// @slice-tier: tier1
// @slice-boacas: true
// @slice-fuel: 50_000_000
// @slice-allow-net: ["127.0.0.1:8080"]
export default { fetch(req) { return new Response("ok"); } };
"#;

    group.bench_function("manifest_plus_inline_directive_resolution", |b| {
        b.iter(|| {
            let m = SliceManifest::parse_toml(black_box(manifest_toml));
            let d = parse_inline_directives(black_box(inline_src));
            black_box((m.fuel_limit, d.fuel_limit))
        });
    });

    group.finish();
}

criterion_group!(benches, bench_cli_config_layers);
criterion_main!(benches);
