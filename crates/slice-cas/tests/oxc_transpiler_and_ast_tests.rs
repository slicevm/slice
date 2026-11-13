use crate::compiler::strip_js_comments;
use crate::linker::{
    inline_cold_method_requires_ast, prune_and_minify_cjs_module_ast, transform_module_to_cjs,
};
use crate::*;

#[test]
fn test_phase_n0_oxc_ast_array_pipeline_fusion_and_unmodified_node_modules() {
    let raw_code = r#"
const orders = [{ price: 120, qty: 2, active: true }, { price: 50, qty: 5, active: false }, { price: 300, qty: 1, active: true }];
const totalGross = orders.map((item) => item.price * item.qty).reduce((acc, v) => acc + v, 0);
const activeRevenue = orders.filter((o) => o.active).map((o) => o.price * o.qty).reduce((sum, val) => sum + val, 0);
const hasHighValue = orders.some((entry) => entry.price >= 250);
const allPositive = orders.every((entry) => entry.qty > 0);
"#;
    let fused = fuse_array_pipelines_ast(raw_code);
    assert!(
        !fused.contains(".map(")
            && !fused.contains(".reduce(")
            && !fused.contains(".some(")
            && !fused.contains(".every("),
        "Expected all higher-order array pipelines to be fused into single-pass for loops, got:\n{}",
        fused
    );
    assert!(fused.contains("for (let __sl_i1 = 0;"));
    assert!(fused.contains("for (let __sl_i2 = 0;"));
    assert!(fused.contains("for (let __sl_i3 = 0;"));
    assert!(fused.contains("for (let __sl_i4 = 0;"));

    let test_dir = std::env::temp_dir().join("slice_phase_n0_real_node_modules_test");
    let _ = std::fs::remove_dir_all(&test_dir);
    let nm_ms_dir = test_dir.join("node_modules").join("ms");
    std::fs::create_dir_all(&nm_ms_dir).unwrap();
    std::fs::write(
        nm_ms_dir.join("package.json"),
        r#"{"name":"ms","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm_ms_dir.join("index.js"),
        r#"module.exports = function customMsFromDisk(val) { return "disk-ms:" + val; };"#,
    )
    .unwrap();
    let entry_path = test_dir.join("index.ts");
    std::fs::write(
        &entry_path,
        r#"import ms from "ms"; console.log(ms("5m"));"#,
    )
    .unwrap();

    let bundled = bundle_entrypoint_with_deps(&entry_path).unwrap();
    assert!(
        bundled.contains("customMsFromDisk") && bundled.contains("disk-ms:"),
        "Expected bundler to preserve actual node_modules/ms/index.js source from disk, got:\n{}",
        bundled
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_oxc_native_jsx_tsx_compilation_and_bundling() {
    let tsx_src = r#"
import { renderToString } from "slice:jsx";

interface CardProps {
    title: string;
    items: string[];
}

export function Card({ title, items }: CardProps) {
    return (
        <section className="card" data-count={items.length}>
            <h2>{title}</h2>
            <ul>
                {items.map((it) => <li>{it}</li>)}
            </ul>
        </section>
    );
}
"#;
    let compiled = strip_typescript_and_normalize(tsx_src, true);
    assert!(
        !compiled.contains("interface CardProps") && !compiled.contains("<section"),
        "Expected JSX and TypeScript types to be compiled away by Oxc, got:\n{}",
        compiled
    );
    assert!(
        compiled.contains("slice:jsx"),
        "Expected automatic JSX runtime import to resolve to slice:jsx builtin:\n{}",
        compiled
    );
}

#[test]
fn test_wave_g_oxc_ast_typescript_semantics_and_string_literal_require_safety() {
    let ts_src = r#"
export interface OrderPayload {
    readonly orderId: string;
    totalCents: number;
    settled: boolean;
    status: "pending" | "paid";
}

const msg = "do not touch require('fs') inside string literal or fake: number";
const obj = { fakeObjField: number_ref };
export function processOrder(order: OrderPayload, discount: number, active: boolean): string {
    const label: string = `template require("crypto") stays untouched`;
    const fsMod = require("fs");
    return label + typeof fsMod + order.orderId + String(discount) + String(active);
}
"#;
    let hints = analyze_typescript_semantics(ts_src);
    assert!(hints.numeric_vars.contains("discount"));
    assert!(hints.boolean_vars.contains("active"));
    assert!(hints.string_vars.contains("label"));
    assert!(
        !hints.numeric_vars.contains("fake") && !hints.numeric_vars.contains("fakeObjField"),
        "Object literal and string literal colons must not be misclassified as TypeScript type annotations"
    );
    assert!(
        !hints.interface_schemas.contains_key("JournalRecord"),
        "No hardcoded JournalRecord schema should be injected"
    );
    let schema = hints
        .interface_schemas
        .get("OrderPayload")
        .expect("OrderPayload schema");
    assert_eq!(
        schema,
        &vec![
            ("orderId".to_string(), "string".to_string()),
            ("totalCents".to_string(), "number".to_string()),
            ("settled".to_string(), "boolean".to_string()),
            ("status".to_string(), "string".to_string()),
        ]
    );
    assert_eq!(
        hints.interface_vars.get("order").map(|s| s.as_str()),
        Some("OrderPayload")
    );

    let normalized = strip_typescript_and_normalize(ts_src, true);
    assert!(
        normalized.contains("do not touch require('fs') inside string literal"),
        "String literal containing require('fs') must not be rewritten:\n{}",
        normalized
    );
    assert!(
        normalized.contains("template require(\"crypto\") stays untouched"),
        "Template literal containing require(\"crypto\") must not be rewritten:\n{}",
        normalized
    );
    assert!(
        normalized.contains("__slice_cjs_require(\"fs\")")
            || normalized.contains("__slice_cjs_require(\"node:fs\")"),
        "Actual CallExpression require(\"fs\") must be rewritten:\n{}",
        normalized
    );
}

#[test]
fn test_ts_generics_with_html_strings_and_template_comment_preservation() {
    let ts_src = r#"
export const identity = <T>(val: T): T => val;
export const htmlTemplate = `<div class="panel">
  // line inside template literal starting with double slash
  /* block comment syntax inside template literal */
  <span>Hello</span>
</div>`;
console.log(identity(htmlTemplate));
"#;
    let out = strip_typescript_raw_with_ext(ts_src, true, false);
    assert!(
        out.contains("// line inside template literal starting with double slash"),
        "Template literal lines starting with // must be preserved verbatim:\n{}",
        out
    );
    assert!(
        out.contains("/* block comment syntax inside template literal */"),
        "Template literal lines starting with /* must be preserved verbatim:\n{}",
        out
    );
    assert!(
        out.contains("identity = (val) => val"),
        "TypeScript <T> generic arrow function must not be misparsed as JSX when HTML strings exist:\n{}",
        out
    );
}

#[test]
fn test_pipeline_fusion_reused_parameter_names_and_scope_hazards() {
    let code = r#"
const total = items
  .filter((x) => x > 1)
  .map((x) => ({ x, doubled: x * 2 }))
  .reduce((x, item) => x + item.doubled, 0);
"#;
    let fused = fuse_array_pipelines_ast(code);
    assert!(
        fused.contains("for (let __sl_i"),
        "Expected filter+map+reduce chain with reused param names to fuse:\n{}",
        fused
    );
    check_syntax_source(&fused, None)
        .expect("Fused pipeline with reused parameter names and shorthand object must be valid JS");
    assert!(
        fused.contains("x: __sl_m"),
        "Object shorthand {{ x }} must expand cleanly when renaming callback parameter:\n{}",
        fused
    );

    // Async/await or `this`/`arguments` inside callback or receiver must bail out cleanly
    let async_hazard =
        "async function f(arr) { return arr.map((x) => await g(x)).reduce((a, b) => a + b, 0); }";
    let unfused_async = fuse_array_pipelines_ast(async_hazard);
    assert_eq!(unfused_async, async_hazard);

    let this_hazard = "function m(arr) { return arr.some((x) => x === this.target); }";
    let unfused_this = fuse_array_pipelines_ast(this_hazard);
    assert_eq!(unfused_this, this_hazard);
}

#[test]
fn test_crypto_import_aliases_and_subexports_preserved() {
    let aliased_src = r#"
import { createHash as hash, pbkdf2Sync } from "node:crypto";
export function derive(input) {
  return hash("sha256").update(pbkdf2Sync(input, "salt", 1, 16, "sha256")).digest("hex");
}
"#;
    let mut deps = Vec::new();
    let cjs_aliased =
        transform_module_to_cjs(aliased_src, std::path::Path::new("."), None, &mut deps);
    assert!(
        cjs_aliased.contains("require(\"node:crypto\")"),
        "Aliased and non-default node:crypto named imports must retain require(\"node:crypto\"):\n{}",
        cjs_aliased
    );
    assert!(cjs_aliased.contains("createHash: hash") && cjs_aliased.contains("pbkdf2Sync"));

    let ns_alias_src = r#"
import * as crypto from "node:crypto";
const c = crypto;
export const id = () => c.randomUUID();
"#;
    let cjs_ns = transform_module_to_cjs(ns_alias_src, std::path::Path::new("."), None, &mut deps);
    assert!(
        cjs_ns.contains("const crypto = require(\"node:crypto\");"),
        "Namespace node:crypto import assigned to another variable must retain require:\n{}",
        cjs_ns
    );

    let unused_crypto_src = r#"
import { createHash } from "node:crypto";
export const answer = 42;
"#;
    let cjs_unused = transform_module_to_cjs(
        unused_crypto_src,
        std::path::Path::new("."),
        None,
        &mut deps,
    );
    assert!(
        !cjs_unused.contains("require(\"node:crypto\")"),
        "Completely unreferenced node:crypto specifier import should still be omitted:\n{}",
        cjs_unused
    );
}

#[test]
fn test_comment_stripper_nested_templates_and_regex_after_parens() {
    let js_src = r#"
const msg = `outer ${`inner // keep-inner`} // keep-outer`;
function check(ok, u) {
  /* inline comment between tokens */
  if (ok) return/*gap*/ /https?:\/\/example\.com\/path/.test(u);
  return 0;
}
"#;
    let stripped = strip_js_comments(js_src);
    check_syntax_source(&stripped, None).expect("Comment-stripped JS must parse cleanly");
    assert!(stripped.contains("inner // keep-inner"));
    assert!(stripped.contains("} // keep-outer`"));
    assert!(stripped.contains("return /https?:\\/\\/example\\.com\\/path/.test(u)"));
}

#[test]
fn test_cjs_lazy_barrel_nested_arrays_and_tree_shaker_side_effect_init() {
    let cjs_with_array = r#"const foo = require("./foo");
const bar = require("./bar");
module.exports = {
  bar,
  list: [
    foo,
  ],
};"#;
    let mut deps = Vec::new();
    let transformed =
        transform_module_to_cjs(cjs_with_array, std::path::Path::new("."), None, &mut deps);
    check_syntax_source(&transformed, None)
        .expect("CJS lazy barrel transform must not emit getter syntax inside nested array");
    assert!(
        transformed.contains("get bar()"),
        "Direct top-level shorthand property on module.exports should become a lazy getter:\n{}",
        transformed
    );
    assert!(
        !transformed.contains("get foo()"),
        "Array element `foo,` inside module.exports must never become `get foo()`:\n{}",
        transformed
    );

    // Cold method require inliner must preserve string literals and object property keys
    let cold_cjs = r#"const { parseTag } = require("/pkg/node_modules/tag/index.js");
class Widget {
  renderCold() {
    return { parseTag: "parseTag", out: parseTag("x") };
  }
}"#;
    let mut single_deps = rustc_hash::FxHashSet::default();
    single_deps.insert("/pkg/node_modules/tag/index.js".to_string());
    let inlined = inline_cold_method_requires_ast(cold_cjs, &single_deps);
    check_syntax_source(&inlined, None).expect("Inlined cold method CJS must parse cleanly");
    assert!(
        inlined.contains(r#"parseTag: "parseTag""#),
        "Object key and string literal matching binding name must remain untouched:\n{}",
        inlined
    );
    assert!(
        inlined.contains(r#"require("/pkg/node_modules/tag/index.js").parseTag("x")"#),
        "Identifier call in cold method should be inlined:\n{}",
        inlined
    );

    // Tree shaker must preserve `const local = exports.used = ...` when `used` is live
    let side_effect_cjs = r#"
exports.__esModule = true;
const internalHelper = exports.liveApi = function(v) { return v + 1; };
const deadFn = function() { return 0; };
exports.deadApi = deadFn;
"#;
    let mut used = rustc_hash::FxHashSet::default();
    used.insert("liveApi".to_string());
    let pruned = prune_and_minify_cjs_module_ast(side_effect_cjs, Some(&used), false);
    assert!(
        pruned.contains("exports.liveApi"),
        "Declarator with `exports.liveApi = ...` assignment must not be pruned when `internalHelper` has 0 local refs:\n{}",
        pruned
    );
    assert!(!pruned.contains("deadApi"));
}
