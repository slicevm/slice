use crate::bundler::resolver::resolve_module_specifier;
use crate::linker::{
    analyze_module_used_exports_from_summaries, extract_module_require_summary, format_exports_lhs,
    is_valid_js_identifier, prune_and_minify_cjs_module_ast, transform_module_to_cjs, UsedExports,
};
use crate::*;
use std::time::Instant;

#[test]
fn test_cas_sec_01_json_module_code_injection_blocked() {
    let test_dir = std::env::temp_dir().join("slice_cas_sec_01_json_injection_test");
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(&test_dir).unwrap();

    // Malicious .json payload attempting to break out of `module.exports = ...;`
    std::fs::write(
        test_dir.join("evil.json"),
        r#"{"safe": 1}; globalThis.__pwned = true; //"#,
    )
    .unwrap();
    let entry_path = test_dir.join("index.ts");
    std::fs::write(
        &entry_path,
        r#"import cfg from "./evil.json"; export const v = cfg.safe;"#,
    )
    .unwrap();

    let res = bundle_entrypoint_with_deps(&entry_path);
    assert!(
        res.is_err(),
        "Expected invalid .json with trailing JS injection to be rejected by bundle_entrypoint_with_deps"
    );
    let err_msg = format!("{:#}", res.unwrap_err());
    assert!(
        err_msg.contains("Invalid JSON module"),
        "Expected Invalid JSON module error, got: {}",
        err_msg
    );

    // Valid JSON with comments (JSONC) should be parsed and canonicalized cleanly
    std::fs::write(
        test_dir.join("evil.json"),
        "{\n  // comment allowed in JSONC\n  \"safe\": 42\n}",
    )
    .unwrap();
    let bundled = bundle_entrypoint_with_deps(&entry_path).expect("Valid JSONC should bundle");
    assert!(
        bundled.contains(r#"module.exports = {"safe":42};"#),
        "Expected canonicalized JSON in bundle, got:\n{}",
        bundled
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cas_sec_02_package_json_exports_and_imports_path_traversal_blocked() {
    let test_dir = std::env::temp_dir().join("slice_cas_sec_02_exports_traversal_test");
    let _ = std::fs::remove_dir_all(&test_dir);
    let app_dir = test_dir.join("app");
    let nm_pkg_dir = app_dir.join("node_modules").join("evil-pkg");
    std::fs::create_dir_all(&nm_pkg_dir).unwrap();

    // Secret file outside `node_modules/evil-pkg`
    let secret_outside = test_dir.join("secret.js");
    std::fs::write(&secret_outside, "module.exports = 'TOP_SECRET';").unwrap();

    // 1. Direct `..` in package.json "exports" target must be rejected
    std::fs::write(
        nm_pkg_dir.join("package.json"),
        r#"{
            "name": "evil-pkg",
            "exports": {
                ".": "../../../secret.js",
                "./sub/*": "./lib/*.js"
            }
        }"#,
    )
    .unwrap();
    std::fs::create_dir_all(nm_pkg_dir.join("lib")).unwrap();
    std::fs::write(nm_pkg_dir.join("lib").join("ok.js"), "module.exports = 1;").unwrap();

    let resolved_root = resolve_module_specifier(&app_dir, "evil-pkg");
    assert!(
        resolved_root.is_none(),
        "Expected package.json exports target with '..' traversal to be blocked, got: {:?}",
        resolved_root
    );

    // 2. Wildcard `*` subpath traversal (`evil-pkg/sub/../../../../secret`) must be rejected
    let resolved_wildcard_escape =
        resolve_module_specifier(&app_dir, "evil-pkg/sub/../../../../secret");
    assert!(
        resolved_wildcard_escape.is_none(),
        "Expected wildcard subpath with '..' traversal to be blocked, got: {:?}",
        resolved_wildcard_escape
    );

    // 3. Valid wildcard subpath within the package must still resolve normally
    let resolved_valid = resolve_module_specifier(&app_dir, "evil-pkg/sub/ok");
    assert!(
        resolved_valid.is_some(),
        "Expected valid wildcard subpath 'evil-pkg/sub/ok' to resolve"
    );

    // 4. `#imports` subpath map with `..` escape must be rejected
    std::fs::write(
        app_dir.join("package.json"),
        r##"{
            "name": "app",
            "imports": {
                "#secret": "../secret.js"
            }
        }"##,
    )
    .unwrap();
    let resolved_import_escape = resolve_module_specifier(&app_dir, "#secret");
    assert!(
        resolved_import_escape.is_none(),
        "Expected package.json #imports target with '..' traversal to be blocked, got: {:?}",
        resolved_import_escape
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cas_sec_03_circular_tsconfig_extends_does_not_stack_overflow() {
    let test_dir = std::env::temp_dir().join("slice_cas_sec_03_tsconfig_cycle_test");
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(&test_dir).unwrap();

    // Create a 2-node cycle: tsconfig.json -> tsconfig.base.json -> tsconfig.json
    std::fs::write(
        test_dir.join("tsconfig.json"),
        r#"{
            "extends": "./tsconfig.base.json",
            "compilerOptions": {
                "baseUrl": ".",
                "paths": {
                    "@app/*": ["src/*"]
                }
            }
        }"#,
    )
    .unwrap();
    std::fs::write(
        test_dir.join("tsconfig.base.json"),
        r#"{
            "extends": "./tsconfig.json",
            "compilerOptions": {
                "strict": true
            }
        }"#,
    )
    .unwrap();

    let src_dir = test_dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("util.ts"), "export const x = 7;").unwrap();

    // Resolving `@app/util` walks tsconfig.json -> extends cycle without overflowing the stack
    let resolved = resolve_module_specifier(&src_dir, "@app/util");
    assert!(
        resolved.is_some(),
        "Expected @app/util to resolve cleanly despite circular tsconfig.json extends"
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cas_sec_04_ingest_directory_skips_excluded_dirs_and_sorts_deterministically() {
    let test_dir = std::env::temp_dir().join("slice_cas_sec_04_ingest_deterministic_test");
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(test_dir.join("node_modules").join("ignored")).unwrap();
    std::fs::create_dir_all(test_dir.join(".git").join("hooks")).unwrap();
    std::fs::create_dir_all(test_dir.join("src")).unwrap();

    std::fs::write(
        test_dir
            .join("node_modules")
            .join("ignored")
            .join("pollute.js"),
        "module.exports = 'should_not_be_ingested';",
    )
    .unwrap();
    std::fs::write(
        test_dir.join("src").join("b.ts"),
        "export const b: number = 2;",
    )
    .unwrap();
    std::fs::write(
        test_dir.join("src").join("a.ts"),
        "export const a: number = 1;",
    )
    .unwrap();

    let mut builder = CasBuilder::new("./src/a.ts");
    let count = builder
        .ingest_directory(&test_dir, &|_| Vec::new())
        .unwrap();
    assert_eq!(
        count, 2,
        "Expected only src/a.ts and src/b.ts to be ingested (excluding node_modules and .git)"
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cas_sec_05_non_identifier_export_names_use_safe_bracket_syntax() {
    assert!(is_valid_js_identifier("validName_$1"));
    assert!(!is_valid_js_identifier("invalid-name"));
    assert!(!is_valid_js_identifier("123start"));
    assert!(!is_valid_js_identifier("a; globalThis.pwn=1"));

    assert_eq!(format_exports_lhs("validName"), "exports.validName");
    assert_eq!(
        format_exports_lhs("kebab-export"),
        r#"exports["kebab-export"]"#
    );

    let test_dir = std::env::temp_dir().join("slice_cas_sec_05_export_lhs_test");
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(&test_dir).unwrap();
    let dep_path = test_dir.join("dep.js");
    std::fs::write(&dep_path, "exports.x = 1;").unwrap();

    let mut deps = Vec::new();
    let src = r#"export * as "kebab-ns" from "./dep.js";"#;
    let out = transform_module_to_cjs(src, &test_dir, None, &mut deps);
    assert!(
        out.contains(r#"exports["kebab-ns"] = require("#),
        "Expected string-literal namespace export to use bracket syntax, got:\n{}",
        out
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cas_perf_single_pass_require_summary_and_linker_throughput() {
    let test_dir = std::env::temp_dir().join("slice_cas_perf_linker_test");
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(&test_dir).unwrap();

    let math_path = test_dir.join("math.js");
    let fmt_path = test_dir.join("fmt.js");
    std::fs::write(
        &math_path,
        "exports.add = (a, b) => a + b; exports.multiply = (a, b) => a * b;",
    )
    .unwrap();
    std::fs::write(&fmt_path, "exports.formatCurrency = (n) => '$' + n;").unwrap();

    let esm_source = r#"
import { add, multiply } from "./math.js";
import { formatCurrency } from "./fmt.js";
export function computeInvoice(items) {
    let sum = 0;
    for (let i = 0; i < items.length; i++) {
        sum = add(sum, multiply(items[i].price, items[i].qty));
    }
    return formatCurrency(sum);
}
export function unusedHelper(x) {
    return x * 999;
}
"#;

    let mut deps = Vec::new();
    let cjs = transform_module_to_cjs(esm_source, &test_dir, None, &mut deps);
    assert_eq!(deps.len(), 2);

    let math_id = crate::linker::canonical_mod_id(&deps[0]);
    let fmt_id = crate::linker::canonical_mod_id(&deps[1]);

    let summary = extract_module_require_summary(&cjs);
    assert!(!summary.parse_failed);
    assert!(summary.targets.contains_key(&math_id));
    assert!(summary.targets.contains_key(&fmt_id));

    let summaries = vec![summary];
    let used_on_math = analyze_module_used_exports_from_summaries(&math_id, &summaries, usize::MAX);
    match used_on_math {
        UsedExports::Subset(set) => {
            assert!(set.contains("add"));
            assert!(set.contains("multiply"));
        }
        UsedExports::All | UsedExports::Unreferenced => {
            panic!("Expected selective UsedExports::Subset for math module")
        }
    }

    // Benchmark 1,000 iterations of transform + summary extraction + tree-shaking
    let iterations = 1_000;
    let used_set = ["computeInvoice".to_string()].into_iter().collect();
    let start = Instant::now();
    let mut total_bytes = 0usize;
    for _ in 0..iterations {
        let mut iter_deps = Vec::with_capacity(2);
        let transformed = transform_module_to_cjs(esm_source, &test_dir, None, &mut iter_deps);
        let req_summary = extract_module_require_summary(&transformed);
        let pruned = prune_and_minify_cjs_module_ast(&transformed, Some(&used_set), false);
        total_bytes += pruned.len() + req_summary.targets.len();
    }
    let elapsed = start.elapsed();
    assert!(total_bytes > 0);
    println!(
        "[CAS-PERF] 1,000 iterations of (transform_module_to_cjs + extract_module_require_summary + prune_and_minify_cjs_module_ast): {:?} ({:.2} us/iter)",
        elapsed,
        elapsed.as_secs_f64() * 1e6 / (iterations as f64)
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_single_pass_ts_transpile_live_mutable_exports_and_streaming_exe_pack() {
    let test_dir = std::env::temp_dir().join(format!(
        "slice_cas_single_pass_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(&test_dir).unwrap();

    // 1. Single-pass `.ts` compilation with `<T>` generic arrow, HTML strings, and `return` inside functions
    let ts_mod = r#"
        export interface Boxed<T> { value: T; tag: string; }
        export const wrapBox = <T>(v: T): Boxed<T> => {
            const tpl = "<section>ok</section>";
            return { value: v, tag: tpl };
        };
    "#;
    let stripped_fn_ret = crate::strip_typescript_raw_with_ext(ts_mod, true, false);
    assert!(!stripped_fn_ret.contains("interface Boxed"));
    assert!(stripped_fn_ret.contains("return {"));

    // Top-level `return` script masking still roundtrips cleanly
    let top_ret_ts = "const x: number = 41;\nif (x > 0) { return x + 1; }\nreturn 0;\n";
    let stripped_top_ret = crate::strip_typescript_raw_with_ext(top_ret_ts, true, false);
    assert!(!stripped_top_ret.contains(": number"));
    assert!(stripped_top_ret.contains("return x + 1;"));
    assert!(stripped_top_ret.contains("return 0;"));

    // Extension-aware `.tsx` normalization compiles JSX directly without diagnostic fallback
    let tsx_mod = "export const Badge = ({ label }: { label: string }) => <span>{label}</span>;\n";
    let normalized_tsx = crate::strip_typescript_and_normalize_with_ext(tsx_mod, true, true);
    assert!(!normalized_tsx.contains("label: string"));

    // 2. Single-pass TypeScript live mutable ESM export detection (`export let` + `export declare let`)
    let counter_ts = test_dir.join("counter.ts");
    let entry_ts = test_dir.join("entry.ts");
    std::fs::write(
        &counter_ts,
        r#"
        export declare let ambientOnly: number;
        export let count: number = 10;
        export const fixed: number = 5;
        export function increment(step: number): number {
            count += step;
            return count;
        }
        "#,
    )
    .unwrap();
    std::fs::write(
        &entry_ts,
        r#"
        import { count, fixed, increment } from "./counter.ts";
        const before: number = count + fixed;
        increment(7);
        const after: number = count + fixed;
        console.log(before, after);
        "#,
    )
    .unwrap();

    let bundled = crate::bundle_entrypoint_with_deps(&entry_ts).unwrap();
    // `count` must be rewritten as a live property access on the CJS module namespace, while `fixed` stays direct
    assert!(
        bundled.contains(".count"),
        "Expected live binding rewrite for mutated TypeScript `export let count`, got:\n{}",
        bundled
    );

    // 3. Streaming `embed_cas_into_executable` roundtrip (including in-place overwrite of existing output)
    let cas_path = test_dir.join("app.cas");
    let mut builder = crate::CasBuilder::new("./entry.ts");
    builder.add_module("./entry.ts", entry_ts.clone(), &bundled, |js| {
        js.as_bytes().to_vec()
    });
    builder.write_to_file(&cas_path).unwrap();

    let fake_host_exe = test_dir.join("fake_slice_host.bin");
    // Deliberately non-64-byte-aligned size (70,013 bytes) to verify 64-byte padding + streaming copy
    let host_bytes: Vec<u8> = (0..70_013usize).map(|i| (i % 251) as u8).collect();
    std::fs::write(&fake_host_exe, &host_bytes).unwrap();

    let out_exe = test_dir.join("standalone_app.exe");
    std::fs::write(&out_exe, b"preexisting-stale-output").unwrap();

    let total_size = crate::embed_cas_into_executable(&fake_host_exe, &cas_path, &out_exe).unwrap();
    let exe_bytes = std::fs::read(&out_exe).unwrap();
    assert_eq!(exe_bytes.len() as u64, total_size);
    let tail = &exe_bytes[exe_bytes.len() - 24..];
    assert_eq!(&tail[16..24], b"SLICEEXE");
    let cas_offset = u64::from_le_bytes(tail[0..8].try_into().unwrap()) as usize;
    let cas_len = u64::from_le_bytes(tail[8..16].try_into().unwrap()) as usize;
    assert_eq!(cas_offset % crate::CACHE_LINE_ALIGN, 0);

    {
        let embedded =
            crate::MappedCasArchive::open_from_offset(&out_exe, cas_offset, cas_len).unwrap();
        assert!(embedded.entrypoint_blob().is_some());
        assert!(embedded.resolve_specifier("./entry.ts").is_some());
        assert_eq!(embedded.header.blobs.len(), 1);
    }

    // Also verify `base_exe_path == output_exe_path` works safely via sibling staging file
    let inplace_size = crate::embed_cas_into_executable(&out_exe, &cas_path, &out_exe).unwrap();
    assert!(inplace_size > total_size);
    let inplace_bytes = std::fs::read(&out_exe).unwrap();
    let tail2 = &inplace_bytes[inplace_bytes.len() - 24..];
    assert_eq!(&tail2[16..24], b"SLICEEXE");
    let cas_offset2 = u64::from_le_bytes(tail2[0..8].try_into().unwrap()) as usize;
    let cas_len2 = u64::from_le_bytes(tail2[8..16].try_into().unwrap()) as usize;
    {
        let embedded_inplace =
            crate::MappedCasArchive::open_from_offset(&out_exe, cas_offset2, cas_len2).unwrap();
        assert!(embedded_inplace.entrypoint_blob().is_some());
        assert!(embedded_inplace.resolve_specifier("./entry.ts").is_some());
    }

    let _ = std::fs::remove_dir_all(&test_dir);
}
