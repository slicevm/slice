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
