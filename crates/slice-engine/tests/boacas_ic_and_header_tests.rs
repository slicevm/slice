use crate::SliceIsolate;
use crate::boacas::{
    BOACAS_FLAG_IS_RC_BUNDLE, BOACAS_HEADER_LEN, BOACAS_MAGIC, BoaCasHeader,
    set_thread_exper_boacas_override,
};
use crate::parser_and_compiler::compile_source_to_bytecode;
use slice_sandbox::CapabilityToken;

#[test]
fn test_boacas_header_roundtrip_and_runtime_toggle() {
    set_thread_exper_boacas_override(Some(true));
    let ts_src = r#"
        const u = new URL("https://edge.slice.local/v1/items?limit=10");
        const enc = new TextEncoder().encode(u.pathname);
        console.log(u.searchParams.get("limit") + ":" + enc.length);
    "#;
    let bc = compile_source_to_bytecode(ts_src);
    assert_eq!(&bc[0..8], BOACAS_MAGIC);

    let (hdr, pre, fb_js) = BoaCasHeader::decode(&bc).expect("Valid SLICEBCv1 header");
    assert_eq!(
        hdr.approach_kind, 3,
        "Expected Approach C (precompiled CodeBlock bytecode)"
    );
    assert!(
        !pre.is_empty(),
        "Expected non-empty precompiled CodeBlock bytecode"
    );
    assert_ne!(hdr.shards_mask & (1 << 1), 0, "Expected URL shard bit set");
    assert_ne!(
        hdr.shards_mask & (1 << 2),
        0,
        "Expected TextEncoder shard bit set"
    );
    assert!(fb_js.contains("URLSearchParams") || fb_js.contains("new URL"));

    // Clear thread-local pool so iso1 takes the cold bare realm precompiled CodeBlock path
    crate::boa_realm::BOA_CONTEXT_POOL.with(|pool| pool.borrow_mut().clear());

    // Execute with --exper-boacas enabled (cold precompiled CodeBlock)
    let mut iso1 = SliceIsolate::spawn(
        "boacas-c-t1",
        1,
        CapabilityToken::pure_compute(),
        100_000,
        4 * 1024 * 1024,
    );
    let m1 = iso1.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m1.stdout, vec!["10:9"]);

    // Execute a second time on the warm pooled realm
    let mut iso_warm = SliceIsolate::spawn(
        "boacas-c-warm",
        1,
        CapabilityToken::pure_compute(),
        100_000,
        4 * 1024 * 1024,
    );
    let m_warm = iso_warm.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m_warm.stdout, vec!["10:9"]);

    // Execute the exact same SLICEBC\x01 buffer with --no-exper-boacas (fallback path)
    set_thread_exper_boacas_override(Some(false));
    let mut iso2 = SliceIsolate::spawn(
        "boacas-c-t2",
        1,
        CapabilityToken::pure_compute(),
        100_000,
        4 * 1024 * 1024,
    );
    let m2 = iso2.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m2.stdout, vec!["10:9"]);

    set_thread_exper_boacas_override(None);
}

#[test]
fn test_boacas_codeblock_edge_cases_bigint_utf16_closures_handlers_and_corruption_fallback() {
    set_thread_exper_boacas_override(Some(true));
    let edge_src = r#"
        const big = 9007199254740993n * 2n;
        const utf16 = "\u03b1\u03b2\u03b3\u03b4-\u03bb-\u03a9";
        function makeAdder(base) {
            let acc = base;
            return function(step) {
                try {
                    if (step < 0) throw new Error("neg:" + step);
                    acc += step;
                    return acc;
                } catch (e) {
                    return -1;
                } finally {
                    acc += 1;
                }
            };
        }
        class Boxed {
            constructor(v) { this._v = v; }
            get val() { return this._v; }
        }
        const add = makeAdder(10);
        const r1 = add(5);
        const r2 = add(-3);
        const r3 = new Boxed(add(0)).val;
        Promise.resolve(utf16.length).then((uLen) => {
            console.log(big.toString() + "|" + utf16 + "|" + uLen + "|" + r1 + "," + r2 + "," + r3);
        });
    "#;
    let bc = compile_source_to_bytecode(edge_src);
    assert_eq!(&bc[0..8], BOACAS_MAGIC);
    let expected_out =
        "18014398509481986|\u{03b1}\u{03b2}\u{03b3}\u{03b4}-\u{03bb}-\u{03a9}|8|15,-1,17";

    // 1. Cold bare-realm execution of precompiled CodeBlock with BigInt, UTF-16, closures, and try/catch/finally
    crate::boa_realm::BOA_CONTEXT_POOL.with(|pool| pool.borrow_mut().clear());
    let mut iso_cold = SliceIsolate::spawn(
        "boacas-edge-cold",
        1,
        CapabilityToken::pure_compute(),
        100_000,
        8 * 1024 * 1024,
    );
    let m_cold = iso_cold.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m_cold.stdout, vec![expected_out]);

    // 2. Corrupt the inner SLCB\x01 magic inside precompiled_slice and verify graceful fallback to fallback_js
    let mut corrupted_bc = bc.clone();
    corrupted_bc[BOACAS_HEADER_LEN] ^= 0xFF;
    crate::boa_realm::BOA_CONTEXT_POOL.with(|pool| pool.borrow_mut().clear());
    let mut iso_corrupt = SliceIsolate::spawn(
        "boacas-edge-corrupt",
        1,
        CapabilityToken::pure_compute(),
        100_000,
        8 * 1024 * 1024,
    );
    let m_corrupt = iso_corrupt.execute_bytecode(&corrupted_bc, None).unwrap();
    assert_eq!(m_corrupt.stdout, vec![expected_out]);

    // 3. Truncated header returns None from BoaCasHeader::decode
    assert!(BoaCasHeader::decode(&bc[..16]).is_none());

    set_thread_exper_boacas_override(None);
}

#[test]
fn test_boacas_prefused_node_builtins_and_deferred_module_spans() {
    set_thread_exper_boacas_override(Some(true));

    let tmp_dir = std::env::temp_dir().join("slice_boacas_edge_node_modules");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let nm_dir = tmp_dir.join("node_modules");
    std::fs::create_dir_all(nm_dir.join("hot-pkg")).unwrap();
    std::fs::create_dir_all(nm_dir.join("cold-pkg")).unwrap();
    std::fs::write(
        nm_dir.join("hot-pkg").join("index.js"),
        "const path = require('node:path');\nexports.tag = function(a, b) { return path.posix.join(a, b); };\n",
    )
    .unwrap();
    std::fs::write(
        nm_dir.join("cold-pkg").join("index.js"),
        "exports.neverCalled = function() { return 'COLD_PKG_LOADED'; };\n",
    )
    .unwrap();
    let entry_path = tmp_dir.join("index.js");
    std::fs::write(
        &entry_path,
        r#"
        const hot = require("hot-pkg");
        const crypto = require("node:crypto");
        const assert = require("node:assert");
        function unusedHelper() {
            return require("cold-pkg").neverCalled();
        }
        const joined = hot.tag("edge", "v1");
        assert.strictEqual(joined, "edge/v1");
        const digest = crypto.createHash("sha256").update(joined).digest("hex").slice(0, 8);
        console.log(joined + ":" + digest);
        "#,
    )
    .unwrap();

    let bundled = slice_cas::bundle_entrypoint_with_deps(&entry_path).unwrap();
    let _ = std::fs::remove_dir_all(&tmp_dir);

    // Verify Branch A's Oxc AST deferred function span optimization turned cold-pkg into a lazy string literal
    assert!(
        bundled.contains("= \"exports.neverCalled"),
        "Expected cold-pkg inside unusedHelper() to be deferred as a lazy string literal"
    );

    let bc = compile_source_to_bytecode(&bundled);
    let (hdr, _, _) = BoaCasHeader::decode(&bc).expect("Valid SLICEBCv1 header");
    assert_ne!(
        hdr.shards_mask & (1 << 19),
        0,
        "Expected node:crypto bit 19"
    );
    assert_ne!(hdr.shards_mask & (1 << 21), 0, "Expected node:path bit 21");
    assert_ne!(
        hdr.shards_mask & (1 << 25),
        0,
        "Expected node:assert bit 25"
    );
    assert_ne!(hdr.feature_flags & BOACAS_FLAG_IS_RC_BUNDLE, 0);

    crate::boa_realm::BOA_CONTEXT_POOL.with(|pool| pool.borrow_mut().clear());
    let mut iso_cold = SliceIsolate::spawn(
        "boacas-node-cold",
        1,
        CapabilityToken::pure_compute(),
        100_000,
        8 * 1024 * 1024,
    );
    let m_cold = iso_cold.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m_cold.stdout.len(), 1);
    assert!(m_cold.stdout[0].starts_with("edge/v1:"));

    // Second execution hits the warm __slice_rc cached function handle
    let mut iso_warm = SliceIsolate::spawn(
        "boacas-node-warm",
        1,
        CapabilityToken::pure_compute(),
        100_000,
        8 * 1024 * 1024,
    );
    let m_warm = iso_warm.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m_warm.stdout, m_cold.stdout);

    set_thread_exper_boacas_override(None);
}

#[test]
fn test_slicebc_adversarial_deserialization_bounds() {
    let mut ctx = boa_engine::Context::default();
    let valid_script =
        boa_engine::Script::parse(boa_engine::Source::from_bytes(b"1 + 2"), None, &mut ctx)
            .expect("valid parse");
    let valid_bytes = valid_script
        .compile_to_slicebc(&mut ctx)
        .expect("valid slicebc");
    assert_eq!(&valid_bytes[..5], b"SLCB\x01");
    assert!(boa_engine::Script::from_slicebc_bytes(&valid_bytes, &mut ctx).is_some());

    // 1. Bad magic header must fail closed immediately
    let mut bad_magic = valid_bytes.clone();
    bad_magic[0] = b'X';
    assert!(boa_engine::Script::from_slicebc_bytes(&bad_magic, &mut ctx).is_none());

    // 2. Trailing garbage bytes must be rejected
    let mut with_trailing = valid_bytes.clone();
    with_trailing.push(0xAA);
    assert!(boa_engine::Script::from_slicebc_bytes(&with_trailing, &mut ctx).is_none());

    // 3. Adversarial global_bindings = u32::MAX (zero-byte loop hazard)
    let t0 = std::time::Instant::now();
    let mut huge_globals = valid_bytes.clone();
    huge_globals[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(boa_engine::Script::from_slicebc_bytes(&huge_globals, &mut ctx).is_none());

    // 4. Adversarial register_count = u32::MAX & mapped_len = u32::MAX (OOM hazard)
    let mut huge_regs = Vec::new();
    huge_regs.extend_from_slice(b"SLCB\x01");
    huge_regs.extend_from_slice(&0u32.to_le_bytes()); // global_bindings
    huge_regs.extend_from_slice(&0u16.to_le_bytes()); // flags
    huge_regs.extend_from_slice(&0u32.to_le_bytes()); // length
    huge_regs.extend_from_slice(&0u32.to_le_bytes()); // parameter_length
    huge_regs.extend_from_slice(&u32::MAX.to_le_bytes()); // register_count
    huge_regs.push(1u8); // strict
    assert!(boa_engine::Script::from_slicebc_bytes(&huge_regs, &mut ctx).is_none());

    let mut huge_mapped = Vec::new();
    huge_mapped.extend_from_slice(b"SLCB\x01");
    huge_mapped.extend_from_slice(&0u32.to_le_bytes()); // global_bindings
    huge_mapped.extend_from_slice(&0u16.to_le_bytes()); // flags
    huge_mapped.extend_from_slice(&0u32.to_le_bytes()); // length
    huge_mapped.extend_from_slice(&0u32.to_le_bytes()); // parameter_length
    huge_mapped.extend_from_slice(&4u32.to_le_bytes()); // register_count
    huge_mapped.push(1u8); // strict
    huge_mapped.extend_from_slice(&u32::MAX.to_le_bytes()); // mapped_len = u32::MAX
    assert!(boa_engine::Script::from_slicebc_bytes(&huge_mapped, &mut ctx).is_none());

    // 5. Adversarial Constant::Scope binding count = u32::MAX - 1 (34 GB scope_cache resize hazard)
    let mut huge_scope = Vec::new();
    huge_scope.extend_from_slice(b"SLCB\x01");
    huge_scope.extend_from_slice(&0u32.to_le_bytes()); // global_bindings
    huge_scope.extend_from_slice(&0u16.to_le_bytes()); // flags
    huge_scope.extend_from_slice(&0u32.to_le_bytes()); // length
    huge_scope.extend_from_slice(&0u32.to_le_bytes()); // parameter_length
    huge_scope.extend_from_slice(&1u32.to_le_bytes()); // register_count
    huge_scope.push(1u8); // strict
    huge_scope.extend_from_slice(&0u32.to_le_bytes()); // mapped_len
    huge_scope.extend_from_slice(&0u32.to_le_bytes()); // bc_len
    huge_scope.extend_from_slice(&1u32.to_le_bytes()); // const_len = 1
    huge_scope.push(3u8); // Constant::Scope
    huge_scope.extend_from_slice(&(u32::MAX - 1).to_le_bytes()); // n = u32::MAX - 1
    assert!(boa_engine::Script::from_slicebc_bytes(&huge_scope, &mut ctx).is_none());

    // 6. 500-deep nested Constant::Function recursion bomb
    let mut deep_fn = Vec::new();
    deep_fn.extend_from_slice(b"SLCB\x01");
    deep_fn.extend_from_slice(&0u32.to_le_bytes()); // global_bindings
    for _ in 0..500 {
        deep_fn.extend_from_slice(&0u16.to_le_bytes()); // flags
        deep_fn.extend_from_slice(&0u32.to_le_bytes()); // length
        deep_fn.extend_from_slice(&0u32.to_le_bytes()); // parameter_length
        deep_fn.extend_from_slice(&1u32.to_le_bytes()); // register_count
        deep_fn.push(1u8); // strict
        deep_fn.extend_from_slice(&0u32.to_le_bytes()); // mapped_len
        deep_fn.extend_from_slice(&0u32.to_le_bytes()); // bc_len
        deep_fn.extend_from_slice(&1u32.to_le_bytes()); // const_len = 1
        deep_fn.push(1u8); // tag = 1 (Constant::Function)
    }
    assert!(boa_engine::Script::from_slicebc_bytes(&deep_fn, &mut ctx).is_none());
    assert!(
        t0.elapsed().as_millis() < 50,
        "adversarial SLICEBC checks must reject in sub-millisecond time"
    );
}

#[test]
fn test_inline_cache_shape_id_gc_churn_and_unique_shape_transitions() {
    let mut ctx = boa_engine::Context::default();

    // Define an IC-hot reader/writer over both SharedShape and UniqueShape objects
    // (Object.create(null) + property deletion transitions into dictionary/UniqueShape mode)
    ctx.eval(boa_engine::Source::from_bytes(
        br#"
        function readTarget(o) { return o.target; }
        function writeTarget(o, v) { o.target = v; }
        function makeUniqueWithSixSlots(val) {
            var u = Object.create(null);
            u.a = 1; u.b = 2; u.c = 3; u.d = 4; u.e = 5; u.target = val;
            delete u.a;
            return u;
        }
        function makeUniqueWithOneSlot(val) {
            var u = Object.create(null);
            u.x = 1; u.only = val;
            delete u.x;
            return u;
        }
        "#,
    ))
    .expect("setup IC helpers");

    // Churn GC between UniqueShape allocations to verify monotonic Shape::id() never hits stale slots
    for round in 0..32 {
        let v = ctx
            .eval(boa_engine::Source::from_bytes(
                format!("readTarget(makeUniqueWithSixSlots({}))", 1000 + round).as_bytes(),
            ))
            .expect("eval six-slot unique shape");
        assert_eq!(v.as_number(), Some((1000 + round) as f64));
        boa_engine::gc::force_collect();

        let v_small = ctx
            .eval(boa_engine::Source::from_bytes(
                b"readTarget(makeUniqueWithOneSlot(7))",
            ))
            .expect("eval one-slot unique shape after GC");
        assert!(v_small.is_undefined());
    }

    // Verify UniqueShape in-place defineProperty(writable: false) invalidates cached writable IC slot
    let frozen_prop_res = ctx
        .eval(boa_engine::Source::from_bytes(
            br#"
            function strictWriteTarget(o, v) { "use strict"; o.target = v; }
            var u = Object.create(null);
            u.a = 1; u.target = 10;
            delete u.a;
            strictWriteTarget(u, 20);
            Object.defineProperty(u, "target", { writable: false });
            var threw = false;
            try { strictWriteTarget(u, 99); } catch (_e) { threw = true; }
            threw + ":" + u.target
            "#,
        ))
        .expect("strict non-writable UniqueShape check");
    assert_eq!(
        frozen_prop_res
            .as_string()
            .map(|s| s.to_std_string_escaped()),
        Some("true:20".to_string())
    );

    // Microbenchmark monomorphic IC loop throughput
    let t0 = std::time::Instant::now();
    let sum_val = ctx
        .eval(boa_engine::Source::from_bytes(
            br#"
            (function() {
                var pt = { x: 3, y: 4, z: 5 };
                var acc = 0;
                for (var i = 0; i < 100000; i++) {
                    acc += pt.x + pt.y + pt.z;
                }
                return acc;
            })()
            "#,
        ))
        .expect("monomorphic IC loop");
    assert_eq!(sum_val.as_number(), Some(1_200_000.0));
    println!(
        "[IC-PERF] 300k monomorphic Shape::id() IC reads completed in {:.2} ms",
        t0.elapsed().as_secs_f64() * 1_000.0
    );
}
