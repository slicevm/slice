use criterion::{criterion_group, criterion_main, Criterion};
use rustc_hash::FxHashSet;
use slice_cas::linker::{
    extract_module_require_summary, prune_and_minify_cjs_module_ast, transform_module_to_cjs,
};
use slice_cas::{
    analyze_typescript_semantics, fuse_array_pipelines_ast, strip_typescript_and_normalize,
};
use std::hint::black_box;
use std::path::Path;

fn bench_oxc_compiler(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice_cas::compiler");

    let ts_sample = r#"
export interface OrderPayload {
    readonly orderId: string;
    totalCents: number;
    settled: boolean;
}
export function computeDiscount(order: OrderPayload, rate: number): number {
    return order.settled ? Math.floor(order.totalCents * rate) : 0;
}
"#;
    group.bench_function("strip_typescript_and_normalize", |b| {
        b.iter(|| {
            let out = strip_typescript_and_normalize(black_box(ts_sample), true);
            black_box(out.len())
        });
    });

    group.bench_function("analyze_typescript_semantics", |b| {
        b.iter(|| {
            let hints = analyze_typescript_semantics(black_box(ts_sample));
            black_box(hints.numeric_vars.len())
        });
    });

    let pipeline_sample = r#"
const items = [{ price: 100, qty: 2, active: true }, { price: 50, qty: 3, active: false }];
const total = items.filter((x) => x.active).map((x) => x.price * x.qty).reduce((a, b) => a + b, 0);
"#;
    group.bench_function("fuse_array_pipelines_ast", |b| {
        b.iter(|| {
            let fused = fuse_array_pipelines_ast(black_box(pipeline_sample));
            black_box(fused.len())
        });
    });

    group.finish();
}

fn bench_esm_cjs_linker(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice_cas::linker");

    let esm_sample = r#"
import * as path from "node:path";
import * as crypto from "node:crypto";
export function computeDigest(dir, file) {
    const full = path.join(dir, file);
    return crypto.createHash("sha256").update(full).digest("hex");
}
export function unusedHelper(x) {
    return x * 999;
}
"#;

    let dummy_dir = Path::new(".");
    let mut init_deps = Vec::new();
    let cjs_sample = transform_module_to_cjs(esm_sample, dummy_dir, None, &mut init_deps);
    let mut used_exports = FxHashSet::default();
    used_exports.insert("computeDigest".to_string());

    group.bench_function("transform_module_to_cjs", |b| {
        b.iter(|| {
            let mut deps = Vec::new();
            let out = transform_module_to_cjs(
                black_box(esm_sample),
                black_box(dummy_dir),
                None,
                &mut deps,
            );
            black_box(out.len())
        });
    });

    group.bench_function("extract_module_require_summary", |b| {
        b.iter(|| {
            let summary = extract_module_require_summary(black_box(&cjs_sample));
            black_box(summary.targets.len())
        });
    });

    group.bench_function("prune_and_minify_cjs_module_ast", |b| {
        b.iter(|| {
            let pruned = prune_and_minify_cjs_module_ast(
                black_box(&cjs_sample),
                black_box(Some(&used_exports)),
                false,
            );
            black_box(pruned.len())
        });
    });

    group.finish();
}

criterion_group!(benches, bench_oxc_compiler, bench_esm_cjs_linker);
criterion_main!(benches);
