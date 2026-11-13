use crate::*;

#[test]
fn test_folder_entrypoint_and_esm_cjs_json_bundler() {
    let test_dir = std::env::temp_dir().join("slice_cas_bundler_unit_test");
    let _ = std::fs::remove_dir_all(&test_dir);
    let src_dir = test_dir.join("src");
    let nm_dir = test_dir.join("node_modules").join("mini-pkg");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&nm_dir).unwrap();

    std::fs::write(
        test_dir.join("package.json"),
        r#"{"name":"test-app","main":"./src/index.ts"}"#,
    )
    .unwrap();
    std::fs::write(src_dir.join("config.json"), r#"{"multiplier": 3}"#).unwrap();
    std::fs::write(
        src_dir.join("helper.ts"),
        r#"export function scale(v: number, m: number): number { return v * m; }"#,
    )
    .unwrap();
    std::fs::write(
        nm_dir.join("package.json"),
        r#"{"name":"mini-pkg","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm_dir.join("index.js"),
        r#"module.exports = function addBase(n) { return n + 10; };"#,
    )
    .unwrap();
    std::fs::write(
        src_dir.join("index.ts"),
        r#"
import addBase from "mini-pkg";
import { scale } from "./helper.ts";
import cfg from "./config.json";
console.log(scale(addBase(4), cfg.multiplier));
"#,
    )
    .unwrap();

    let resolved = resolve_project_entrypoint(&test_dir, None).unwrap();
    assert!(resolved.ends_with("index.ts"));

    let bundled = bundle_entrypoint_with_deps(&resolved).unwrap();
    assert!(bundled.contains("var __slice_modules = {};"));
    assert!(bundled.contains("multiplier"));

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_tag_f4_source_line_error_mapping_across_multi_file_bundle() {
    let test_dir = std::env::temp_dir().join("slice_tag_f4_sourcemap_test");
    let _ = std::fs::remove_dir_all(&test_dir);
    let src_dir = test_dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    let validator_ts = r#"export interface OrderSpec {
    id: string;
    amountCents: number;
}
export function assertValidOrder(order: OrderSpec): number {
    if (order.amountCents < 0) throw new Error("Negative order amount forbidden");
    return order.amountCents;
}
"#;
    let index_ts = r#"import { assertValidOrder } from "./pricing_validator.ts";
console.log(assertValidOrder({ id: "ord-1", amountCents: -50 }));
"#;
    std::fs::write(src_dir.join("pricing_validator.ts"), validator_ts).unwrap();
    std::fs::write(src_dir.join("index.ts"), index_ts).unwrap();

    let bundled = bundle_entrypoint_with_deps(&src_dir.join("index.ts")).unwrap();
    assert!(bundled.contains("// #slice-source: src/pricing_validator.ts|"));
    assert!(bundled.contains("// #slice-source: src/index.ts|"));

    let mapped = map_bundled_error_location(
        &bundled,
        r#"Uncaught Error: "Negative order amount forbidden""#,
    );
    assert!(
        mapped.contains("(src/pricing_validator.ts:6)"),
        "Expected mapped error location to point to (src/pricing_validator.ts:6), got: {}",
        mapped
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_phase_n4_oxc_ast_cjs_transform_and_package_exports() {
    let test_dir = std::env::temp_dir().join("slice_phase_n4_exports_and_ast_test");
    let _ = std::fs::remove_dir_all(&test_dir);
    let src_dir = test_dir.join("src");
    let scoped_pkg_dir = test_dir.join("node_modules").join("@edge").join("kit");
    let wildcard_pkg_dir = test_dir.join("node_modules").join("mini-hono");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(scoped_pkg_dir.join("dist")).unwrap();
    std::fs::create_dir_all(wildcard_pkg_dir.join("dist").join("middleware")).unwrap();

    std::fs::write(
        scoped_pkg_dir.join("package.json"),
        r#"{
  "name": "@edge/kit",
  "exports": {
    ".": {
      "types": "./dist/index.d.ts",
      "workerd": {
        "import": "./dist/edge-workerd.js",
        "default": "./dist/edge-workerd.js"
      },
      "default": "./dist/fallback.js"
    }
  }
}"#,
    )
    .unwrap();
    std::fs::write(
        scoped_pkg_dir.join("dist").join("edge-workerd.js"),
        r#"export const RUNTIME_TARGET = "workerd-edge"; export default function kitVersion() { return "kit@4.0"; }"#,
    )
    .unwrap();

    std::fs::write(
        wildcard_pkg_dir.join("package.json"),
        r#"{
  "name": "mini-hono",
  "exports": {
    ".": "./dist/index.js",
    "./*": {
      "import": "./dist/middleware/*.js",
      "default": "./dist/middleware/*.js"
    }
  }
}"#,
    )
    .unwrap();
    std::fs::write(
        wildcard_pkg_dir
            .join("dist")
            .join("middleware")
            .join("jwt.js"),
        r#"export const { ALG_HS256, ALG_HS512 } = { ALG_HS256: "HS256", ALG_HS512: "HS512" };"#,
    )
    .unwrap();

    std::fs::write(
        src_dir.join("helper.ts"),
        r#"export function normalizeRoute(p: string): string { return p.trim().toLowerCase(); }"#,
    )
    .unwrap();

    let entry_ts = src_dir.join("index.ts");
    std::fs::write(
        &entry_ts,
        r#"import kitVersion, {
    RUNTIME_TARGET as targetName
} from "@edge/kit";
import { ALG_HS256 } from "mini-hono/jwt";
import { normalizeRoute } from "./helper.js";

export const summary = `${kitVersion()}:${targetName}:${ALG_HS256}:${normalizeRoute(" /API/V1 ")}`;
"#,
    )
    .unwrap();

    let bundled = bundle_entrypoint_with_deps(&entry_ts).unwrap();
    assert!(
        bundled.contains("workerd-edge")
            && bundled.contains("ALG_HS256")
            && bundled.contains("normalizeRoute"),
        "Expected bundled output to resolve nested workerd exports, wildcard subpaths, and .js->.ts imports:\n{}",
        bundled
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_general_oxc_ast_dead_export_tree_shaker_and_cold_method_inliner() {
    let test_dir = std::env::temp_dir().join("slice_cas_tree_shaker_and_cold_inliner_test");
    let _ = std::fs::remove_dir_all(&test_dir);
    let src_dir = test_dir.join("src");
    let pkg_dir = test_dir.join("node_modules").join("synth-kit");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&pkg_dir).unwrap();

    std::fs::write(
        pkg_dir.join("package.json"),
        r#"{"name":"synth-kit","module":"./index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        pkg_dir.join("cold_body.js"),
        r#"export function parseBody(raw) { return "parsed:" + raw; }"#,
    )
    .unwrap();
    std::fs::write(
        pkg_dir.join("unused_sub.js"),
        r#"export function neverCalledSub() { return "dead-sub-module"; }"#,
    )
    .unwrap();
    std::fs::write(
        pkg_dir.join("index.js"),
        r#"
import { parseBody } from "./cold_body.js";
import { neverCalledSub } from "./unused_sub.js";

function deadRecursiveHelper(n) {
    return n <= 0 ? neverCalledSub() : deadRecursiveHelper(n - 1);
}

export function unusedExportFn(x) {
    return deadRecursiveHelper(x);
}

export class SynthRequest {
    constructor(url) {
        this.url = url;
    }
    async parseBody() {
        return parseBody(this.url);
    }
}
"#,
    )
    .unwrap();

    let entry_ts = src_dir.join("index.ts");
    std::fs::write(
        &entry_ts,
        r#"
import { SynthRequest } from "synth-kit";
const req = new SynthRequest("/v1/ping");
console.log(req.url);
"#,
    )
    .unwrap();

    let bundled = bundle_entrypoint_with_deps(&entry_ts).unwrap();
    assert!(
        !bundled.contains("unusedExportFn")
            && !bundled.contains("deadRecursiveHelper")
            && !bundled.contains("neverCalledSub"),
        "Expected Oxc AST dead-export tree-shaker to prune unused exports, self-recursive helpers, and transitive dead modules:\n{}",
        bundled
    );
    assert!(
        bundled.contains(r#"__slice_modules[0] = ""#),
        "Expected cold-method-only dependency cold_body.js to be emitted as a lazy JSON string module:\n{}",
        bundled
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_wave_h_single_pass_ast_bundler_and_array_module_table() {
    let test_dir = std::env::temp_dir().join("slice_wave_h_single_pass_bundler_test");
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(test_dir.join("src")).unwrap();
    std::fs::create_dir_all(test_dir.join("node_modules/pkg-cjs")).unwrap();

    std::fs::write(
        test_dir.join("node_modules/pkg-cjs/package.json"),
        r#"{"name":"pkg-cjs","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        test_dir.join("node_modules/pkg-cjs/sub.js"),
        r#"module.exports = { compute: function(x) { return x * 3; } };"#,
    )
    .unwrap();
    std::fs::write(
        test_dir.join("node_modules/pkg-cjs/index.js"),
        r#"const sub = require("./sub");
module.exports = {
  sub,
};"#,
    )
    .unwrap();

    let entry_path = test_dir.join("src/index.ts");
    std::fs::write(
        &entry_path,
        r#"import pkg from "pkg-cjs";
const literalNote = "require('./sub') inside string literal stays untouched";
export const answer = pkg.sub.compute(14) + ":" + literalNote;
console.log(answer);
"#,
    )
    .unwrap();

    let bundled = bundle_entrypoint_with_deps(&entry_path).expect("bundle_entrypoint_with_deps");
    assert!(
        bundled.contains("__slice_modules = [];"),
        "Wave H bundle must initialize dense JS Array __slice_modules = []:\n{}",
        bundled
    );
    assert!(
        bundled.contains("get sub() { return require(0); }")
            || bundled.contains("get sub(){return require(0)}"),
        "Single-pass CJS transform must preserve lazy barrel getter optimization:\n{}",
        bundled
    );
    assert!(
        bundled.contains("require('./sub') inside string literal stays untouched"),
        "Single-pass span rewriter must not touch string literals:\n{}",
        bundled
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_wave_i_tsconfig_paths_pkg_imports_meta_dynamic_import_and_embedded_sourcemap() {
    let test_dir = std::env::temp_dir().join("slice_wave_i_dx_resolution_test");
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(test_dir.join("src/utils")).unwrap();
    std::fs::create_dir_all(test_dir.join("src/internal")).unwrap();
    std::fs::create_dir_all(test_dir.join("src/lazy")).unwrap();
    std::fs::create_dir_all(test_dir.join("config")).unwrap();

    // 1. Base tsconfig and child tsconfig with JSONC comments + trailing commas
    std::fs::write(
        test_dir.join("tsconfig.base.json"),
        r#"{
  // Base TypeScript configuration with JSONC comments
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@/*": ["./src/*"],
      "~config": ["./config/app.ts"],
    },
  },
}"#,
    )
    .unwrap();
    std::fs::write(
        test_dir.join("tsconfig.json"),
        r#"{
  /* Child config extending base */
  "extends": "./tsconfig.base.json",
}"#,
    )
    .unwrap();

    // 2. package.json with Node.js subpath `"imports"` (`#...`)
    std::fs::write(
        test_dir.join("package.json"),
        r##"{
  "name": "wave-i-app",
  "imports": {
    "#internal/*": "./src/internal/*.ts"
  }
}"##,
    )
    .unwrap();

    std::fs::write(
        test_dir.join("src/utils/math.ts"),
        r#"export function doubleVal(n: number): number {
  return n * 2;
}
"#,
    )
    .unwrap();

    std::fs::write(
        test_dir.join("config/app.ts"),
        r#"export const APP_TITLE = "SliceWaveI";
"#,
    )
    .unwrap();

    std::fs::write(
        test_dir.join("src/internal/secret.ts"),
        r#"export const INTERNAL_TAG = "subpath-ok";
"#,
    )
    .unwrap();

    std::fs::write(
        test_dir.join("src/lazy/feature.ts"),
        r#"export function lazyBanner(): string {
  return "dynamic-import-ok";
}
"#,
    )
    .unwrap();

    let entry_path = test_dir.join("src/index.ts");
    std::fs::write(
        &entry_path,
        r##"import { doubleVal } from "@/utils/math";
import { APP_TITLE } from "~config";
import { INTERNAL_TAG } from "#internal/secret";

export interface UnusedTypeBlock {
  alpha: string;
  beta: number;
  gamma: boolean;
}

const metaUrl = import.meta.url;
const metaDir = import.meta.dirname;
const cjsDir = __dirname;
const dynMod = await import("@/lazy/feature");
if (doubleVal(21) === 42) {
  throw new Error("wave-i-exact-line-check");
}
console.log(APP_TITLE, INTERNAL_TAG, metaUrl, metaDir, cjsDir, dynMod.lazyBanner());
"##,
    )
    .unwrap();

    let bundled =
        bundle_entrypoint_with_deps(&entry_path).expect("Wave I bundle_entrypoint_with_deps");

    assert!(
        bundled.contains("async function()"),
        "Top-level await must wrap entrypoint runner in async function():\n{}",
        bundled
    );
    assert!(
        bundled.contains("var __filename =") && bundled.contains("__dirname ="),
        "__dirname reference must inject __filename and __dirname bindings:\n{}",
        bundled
    );
    assert!(
        bundled.contains("file:///"),
        "import.meta.url must lower to file:// URL:\n{}",
        bundled
    );

    // Delete the entire source directory from disk to prove embedded line-map works offline
    let _ = std::fs::remove_dir_all(&test_dir);

    let mapped =
        map_bundled_error_location(&bundled, "Uncaught Error: \"wave-i-exact-line-check\"");
    assert!(
        mapped.contains("(src/index.ts:16)"),
        "Embedded #slice-source line map must resolve exact TypeScript line 16 without disk access, got: {}",
        mapped
    );
}

#[test]
fn test_issues_33_and_34_class_method_source_mapping_and_cas_lockfile_provenance() {
    use crate::{map_bundled_stack_trace, CasBuilder, MappedCasArchive};

    let test_dir =
        std::env::temp_dir().join(format!("slice_issues_33_34_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(test_dir.join("src")).unwrap();
    std::fs::create_dir_all(test_dir.join("node_modules/ms")).unwrap();

    std::fs::write(
        test_dir.join("package.json"),
        r#"{"name":"issue-34-app","dependencies":{"ms":"^2.1.0"}}"#,
    )
    .unwrap();
    std::fs::write(
        test_dir.join("package-lock.json"),
        r#"{"name":"issue-34-app","lockfileVersion":3}"#,
    )
    .unwrap();
    std::fs::write(
        test_dir.join("node_modules/ms/package.json"),
        r#"{"name":"ms","version":"2.1.3"}"#,
    )
    .unwrap();
    std::fs::write(
        test_dir.join("node_modules/ms/index.js"),
        "module.exports = function(v) { return v; };\n",
    )
    .unwrap();

    std::fs::write(
        test_dir.join("src/pricing_engine.ts"),
        r#"export interface CheckoutReq {
  requestId: string;
  items: number[];
}

export class PricingEngine {
  processCheckout(req: CheckoutReq): number {
    if (!req.requestId || req.items.length === 0) {
      throw new Error("Invalid checkout request: " + req.requestId);
    }
    return req.items.reduce((a, b) => a + b, 0);
  }
}
"#,
    )
    .unwrap();

    let entry_path = test_dir.join("src/index.ts");
    std::fs::write(
        &entry_path,
        r#"import { PricingEngine } from "./pricing_engine";
const engine = new PricingEngine();
engine.processCheckout({ requestId: "ORD-99", items: [] });
"#,
    )
    .unwrap();

    let bundled = bundle_entrypoint_with_deps(&entry_path).expect("bundle issue 33 app");
    let mapped_err =
        map_bundled_error_location(&bundled, "Uncaught Error: Invalid checkout request: ORD-99");
    assert!(
        mapped_err.contains("(src/pricing_engine.ts:9)"),
        "Dynamic error message in class method must map to src/pricing_engine.ts:9, got: {}",
        mapped_err
    );

    let cas_path = test_dir.join("app.cas");
    let mut builder = CasBuilder::new("./index.ts");
    builder.scan_project_dependencies(&test_dir);
    builder.add_module("./index.ts", entry_path.clone(), &bundled, |s| {
        s.as_bytes().to_vec()
    });
    let header = builder.write_to_file(&cas_path).expect("write cas");
    assert!(
        header.lockfile_blake3.is_some(),
        "CasIndexHeader must record lockfile_blake3"
    );
    assert_eq!(
        header.resolved_dependencies,
        vec![("ms".to_string(), "2.1.3".to_string())]
    );

    let archive = MappedCasArchive::open(&cas_path).expect("open cas");
    let annotated = archive.reconstruct_annotated_source(
        archive
            .resolve_specifier("./index.ts")
            .expect("resolve ./index.ts"),
    );
    let mapped_compacted_err = map_bundled_error_location(
        &annotated,
        "Uncaught Error: Invalid checkout request: ORD-99",
    );
    assert!(
        mapped_compacted_err.contains("(src/pricing_engine.ts:9)"),
        "Compacted CAS //@sm: source map must also resolve src/pricing_engine.ts:9, got: {}",
        mapped_compacted_err
    );
    let fake_raw_stack =
        "Error: Invalid checkout request: ORD-99\n    at processCheckout (<anonymous>:10:13)";
    let mapped_stack = map_bundled_stack_trace(&annotated, fake_raw_stack);
    assert!(
        mapped_stack.contains("src/pricing_engine.ts:"),
        "ES6 class method stack frame must map to src/pricing_engine.ts, got: {}",
        mapped_stack
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_single_file_ts_circular_entry_live_reexports_jsx_override_and_tsconfig_extends() {
    let test_dir = std::env::temp_dir().join(format!(
        "slice_bundler_edge_cases_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(&test_dir).expect("mkdir test_dir");

    // 1. Single-file TypeScript fast path (.ts and .mts)
    let single_ts = test_dir.join("single.ts");
    std::fs::write(
        &single_ts,
        "interface Cfg { scale: number }\nconst cfg: Cfg = { scale: 7 };\nexport default cfg.scale * 6;\n",
    )
    .unwrap();
    let bundled_single = bundle_entrypoint_with_deps(&single_ts).unwrap();
    assert!(
        !bundled_single.contains("interface Cfg"),
        "Single-file .ts fast path must strip TypeScript declarations: {}",
        bundled_single
    );
    assert!(bundled_single.contains("const cfg = { scale: 7 }"));

    // 2. Circular entrypoint imports + hoisted function exports + live export * mutable bindings
    let circ_dir = test_dir.join("circ");
    std::fs::create_dir_all(&circ_dir).unwrap();
    std::fs::write(
        circ_dir.join("state.ts"),
        "export let counter = 10;\nexport function bump(delta: number) { counter += delta; }\n",
    )
    .unwrap();
    std::fs::write(circ_dir.join("barrel.ts"), "export * from './state';\n").unwrap();
    std::fs::write(
        circ_dir.join("helper.ts"),
        "import { entryTag } from './entry';\nexport function describeEntry(): string { return 'seen:' + entryTag(); }\n",
    )
    .unwrap();
    let circ_entry = circ_dir.join("entry.ts");
    std::fs::write(
        &circ_entry,
        "import { describeEntry } from './helper';\nimport { counter, bump } from './barrel';\nexport function entryTag(): string { return 'root'; }\nbump(5);\nconsole.log(describeEntry() + ':' + counter);\n",
    )
    .unwrap();
    let bundled_circ = bundle_entrypoint_with_deps(&circ_entry).unwrap();
    assert!(
        bundled_circ.contains("Object.defineProperty(exports,"),
        "Live export * mutable binding must emit getter property descriptor:\n{}",
        bundled_circ
    );
    assert!(
        bundled_circ.contains("__slice_require("),
        "Circular entrypoint bundle must register and invoke entrypoint via __slice_require:\n{}",
        bundled_circ
    );

    // 3. Installed JSX runtime in node_modules overrides slice:jsx fallback, while slice:jsx/jsx-runtime resolves cleanly
    let jsx_dir = test_dir.join("jsx_app");
    let custom_react_dir = jsx_dir.join("node_modules/react");
    std::fs::create_dir_all(&custom_react_dir).unwrap();
    std::fs::write(
        custom_react_dir.join("package.json"),
        r#"{"name":"react","version":"19.0.0","exports":{"./jsx-runtime":"./jsx-runtime.js"}}"#,
    )
    .unwrap();
    std::fs::write(
        custom_react_dir.join("jsx-runtime.js"),
        "exports.jsx = function(tag, props) { return 'custom-react:' + tag + ':' + (props && props.id || ''); };\n",
    )
    .unwrap();
    let jsx_entry = jsx_dir.join("app.js");
    std::fs::write(
        &jsx_entry,
        "const customJsx = require('react/jsx-runtime');\nconst builtinJsx = require('slice:jsx/jsx-runtime');\nconsole.log(customJsx.jsx('div', { id: 'ok' }) + '|' + typeof builtinJsx.jsx);\n",
    )
    .unwrap();
    let bundled_jsx = bundle_entrypoint_with_deps(&jsx_entry).unwrap();
    assert!(
        bundled_jsx.contains("custom-react:"),
        "Installed node_modules/react/jsx-runtime must be bundled instead of falling back to slice:jsx:\n{}",
        bundled_jsx
    );
    assert!(bundled_jsx.contains("require(\"slice:jsx\")"));

    // 4. tsconfig.json extends "./tsconfig.base" (dot in basename, implicit .json) and array extends
    let ts_dir = test_dir.join("tsconfig_proj");
    std::fs::create_dir_all(ts_dir.join("src/utils")).unwrap();
    std::fs::write(
        ts_dir.join("tsconfig.base.json"),
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@utils/*":["src/utils/*"]}}}"#,
    )
    .unwrap();
    std::fs::write(
        ts_dir.join("tsconfig.extra.json"),
        r#"{"compilerOptions":{"strict":true}}"#,
    )
    .unwrap();
    std::fs::write(
        ts_dir.join("tsconfig.json"),
        r#"{"extends":["./tsconfig.base","./tsconfig.extra"]}"#,
    )
    .unwrap();
    std::fs::write(
        ts_dir.join("src/utils/math.mts"),
        "export const add = (a: number, b: number): number => a + b;\n",
    )
    .unwrap();
    let ts_entry = ts_dir.join("src/main.mts");
    std::fs::write(
        &ts_entry,
        "import { add } from '@utils/math.mjs';\nconsole.log(add(19, 23));\n",
    )
    .unwrap();
    let bundled_ts = bundle_entrypoint_with_deps(&ts_entry).unwrap();
    assert!(bundled_ts.contains("const add = (a, b) => a + b"));

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_esm_default_import_cjs_barrel_tree_shaking_and_eager_modules() {
    use crate::bundle_entrypoint_with_deps;

    let test_dir = std::env::temp_dir().join(format!(
        "slice_cjs_barrel_default_imp_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&test_dir);
    let nm_pkg = test_dir.join("node_modules/toolkit");
    std::fs::create_dir_all(&nm_pkg).unwrap();
    std::fs::write(
        nm_pkg.join("package.json"),
        r#"{"name":"toolkit","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm_pkg.join("used.js"),
        "module.exports = function used(x) { return x * 2; };\n",
    )
    .unwrap();
    std::fs::write(
        nm_pkg.join("unused.js"),
        "module.exports = function unused() { return 'UNUSED_BARREL_MARKER'; };\n",
    )
    .unwrap();
    std::fs::write(
        nm_pkg.join("index.js"),
        "const used = require(\"./used.js\");\nconst unused = require(\"./unused.js\");\nmodule.exports = {\n  used,\n  unused,\n};\n",
    )
    .unwrap();
    let entry = test_dir.join("index.ts");
    std::fs::write(
        &entry,
        "import toolkit from 'toolkit';\nconsole.log(toolkit.used(21));\n",
    )
    .unwrap();

    let bundled = bundle_entrypoint_with_deps(&entry).unwrap();
    assert!(
        !bundled.contains("UNUSED_BARREL_MARKER"),
        "unreferenced CJS barrel sub-module should be pruned: {bundled}"
    );
    assert!(
        !bundled.contains("= \"module.exports="),
        "statically used CJS barrel sub-module should be eager for AOT precompilation: {bundled}"
    );

    let _ = std::fs::remove_dir_all(&test_dir);
}
