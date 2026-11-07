use criterion::{criterion_group, criterion_main, Criterion};
use slice_cas::{
    analyze_typescript_semantics, fuse_array_pipelines_ast, strip_typescript_and_normalize,
};
use std::hint::black_box;

fn bench_oxc_compiler(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice_cas::oxc_compiler");

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

criterion_group!(benches, bench_oxc_compiler);
criterion_main!(benches);

