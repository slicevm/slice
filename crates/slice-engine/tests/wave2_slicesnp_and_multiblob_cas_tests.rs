use crate::{
    BinOp, BytecodeModule, BytecodeOp, Expr, JsFunction, JsValue, SLICESNP_V2_VERSION,
    SNAPSHOT_MAGIC, SliceIsolate, Stmt, UnaryOp, compile_source_to_bytecode,
    decode_slicesnp_module, disassemble_bytecode, encode_slicesnp_v2, set_thread_tier_override,
};
use rustc_hash::FxHashMap;
use slice_cas::{CasBuilder, MappedCasArchive};
use slice_sandbox::CapabilityToken;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

fn temp_cas_file(prefix: &str) -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{}_{}_{}.cas", prefix, std::process::id(), id))
}

struct ThreadTierOverrideGuard;
impl Drop for ThreadTierOverrideGuard {
    fn drop(&mut self) {
        set_thread_tier_override(None);
    }
}

#[test]
fn test_wave2_slicesnp_v2_roundtrip_legacy_json_fallback_and_corrupt_fuzz_bounds() {
    let mut obj_map = FxHashMap::default();
    obj_map.insert("nested".to_string(), JsValue::Bool(true));
    obj_map.insert("nil".to_string(), JsValue::Null);

    let func = JsFunction {
        name: "inc".to_string(),
        params: vec!["n".to_string()],
        body: vec![Stmt::Return(Some(Expr::Binary {
            op: BinOp::Add,
            lhs: Box::new(Expr::Ident("n".to_string())),
            rhs: Box::new(Expr::Literal(JsValue::Number(1.0))),
        }))],
        compiled_numeric: None,
    };

    let mut static_exports = FxHashMap::default();
    static_exports.insert("BETA".to_string(), 20.0);
    static_exports.insert("ALPHA".to_string(), 10.0);

    let module = BytecodeModule {
        static_exports,
        ops: vec![
            BytecodeOp::ConstNum {
                dst: "a".to_string(),
                val: 42.5,
            },
            BytecodeOp::ConstStr {
                dst: "s".to_string(),
                val: "hello".to_string(),
            },
            BytecodeOp::ImportCas {
                dst: "imp".to_string(),
                specifier: "./dep.ts".to_string(),
            },
            BytecodeOp::AddNum {
                dst: "sum".to_string(),
                lhs: "a".to_string(),
                rhs: "10".to_string(),
            },
            BytecodeOp::AddAssignVar {
                dst: "sum".to_string(),
                src: "a".to_string(),
            },
            BytecodeOp::ConsoleLogVar {
                var: "sum".to_string(),
            },
            BytecodeOp::MonomorphicForLoop {
                acc_var: "sum".to_string(),
                idx_var: "i".to_string(),
                start: 0,
                limit: 10,
                scale: 2.0,
            },
            BytecodeOp::EvalStmt(Stmt::If {
                cond: Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(Expr::Literal(JsValue::Undefined)),
                },
                then_branch: vec![
                    Stmt::VarDecl {
                        name: "arr".to_string(),
                        init: Some(Expr::ArrayLit(vec![
                            Expr::Literal(JsValue::Object(Box::new(obj_map))),
                            Expr::Literal(JsValue::Function(Box::new(func))),
                        ])),
                        is_export: true,
                    },
                    Stmt::ExprStmt(Expr::Ternary {
                        cond: Box::new(Expr::Binary {
                            op: BinOp::StrictEq,
                            lhs: Box::new(Expr::Literal(JsValue::Number(1.0))),
                            rhs: Box::new(Expr::Literal(JsValue::Number(1.0))),
                        }),
                        then_expr: Box::new(Expr::ArrowFunc {
                            params: vec!["x".to_string()],
                            body: vec![Stmt::Return(Some(Expr::Ident("x".to_string())))],
                        }),
                        else_expr: Box::new(Expr::ObjectLit(vec![(
                            "k".to_string(),
                            Expr::Literal(JsValue::Number(0.0)),
                        )])),
                    }),
                ],
                else_branch: vec![],
            }),
            BytecodeOp::ReturnVar {
                var: "sum".to_string(),
            },
        ],
    };

    let encoded = encode_slicesnp_v2(&module);
    assert_eq!(&encoded[0..8], SNAPSHOT_MAGIC);
    assert_eq!(encoded[8], SLICESNP_V2_VERSION);

    // Deterministic canonical encoding across repeated runs
    let encoded_again = encode_slicesnp_v2(&module);
    assert_eq!(encoded, encoded_again);

    let decoded = decode_slicesnp_module(&encoded).expect("decode valid SLICESNPv2");
    assert_eq!(decoded.static_exports, module.static_exports);
    assert_eq!(decoded.ops.len(), module.ops.len());
    let disasm = disassemble_bytecode(&encoded).expect("disassemble valid SLICESNPv2");
    assert_eq!(disasm.len(), module.ops.len());

    // Legacy JSON `SLICESNP{...}` snapshot compatibility
    let mut legacy_json = SNAPSHOT_MAGIC.to_vec();
    legacy_json.extend_from_slice(
        &serde_json::to_vec(&BytecodeModule {
            static_exports: FxHashMap::default(),
            ops: vec![
                BytecodeOp::ConstNum {
                    dst: "v".to_string(),
                    val: 777.0,
                },
                BytecodeOp::ReturnVar {
                    var: "v".to_string(),
                },
            ],
        })
        .unwrap(),
    );
    let mut iso_legacy = SliceIsolate::spawn(
        "legacy-snp",
        0,
        CapabilityToken::pure_compute(),
        10_000,
        65536,
    );
    let m_legacy = iso_legacy.execute_bytecode(&legacy_json, None).unwrap();
    assert_eq!(m_legacy.return_value, "777");
    assert!(disassemble_bytecode(&legacy_json).is_ok());

    // Corrupt / adversarial SLICESNPv2 inputs must fail closed without panic or OOM
    for cut in 0..encoded.len() {
        let truncated = &encoded[..cut];
        assert!(decode_slicesnp_module(truncated).is_err());
    }
    let mut with_trailing = encoded.clone();
    with_trailing.push(0xFF);
    assert!(decode_slicesnp_module(&with_trailing).is_err());

    // Huge `u32::MAX` length prefix on a tiny 18-byte buffer must NOT trigger capacity OOM
    let mut huge_len = SNAPSHOT_MAGIC.to_vec();
    huge_len.push(SLICESNP_V2_VERSION);
    huge_len.push(0x00);
    huge_len.extend_from_slice(&u32::MAX.to_le_bytes());
    huge_len.extend_from_slice(&0u32.to_le_bytes());
    assert!(decode_slicesnp_module(&huge_len).is_err());

    // Deeply nested unary expression (> MAX_CODEC_DEPTH = 512) must fail closed without stack overflow
    let mut deep = SNAPSHOT_MAGIC.to_vec();
    deep.push(SLICESNP_V2_VERSION);
    deep.push(0x00);
    deep.extend_from_slice(&0u32.to_le_bytes()); // 0 static exports
    deep.extend_from_slice(&1u32.to_le_bytes()); // 1 op
    deep.push(16); // EvalStmt
    deep.push(10); // ExprStmt
    for _ in 0..600 {
        deep.push(3); // Expr::Unary
        deep.push(0); // UnaryOp::Neg
    }
    deep.push(0); // Expr::Literal
    deep.push(3); // JsValue::Number
    deep.extend_from_slice(&1.0f64.to_le_bytes());
    assert!(
        decode_slicesnp_module(&deep).is_err(),
        "600-deep nested SLICESNPv2 expression must fail closed on MAX_CODEC_DEPTH"
    );
    let mut iso_fuzz = SliceIsolate::spawn(
        "fuzz-snp",
        1,
        CapabilityToken::pure_compute(),
        10_000,
        65536,
    );
    let m_fuzz = iso_fuzz.execute_bytecode(&deep, None).unwrap();
    assert_eq!(m_fuzz.return_value, "undefined");
}

#[test]
fn test_wave2_snp_last_hit_equal_length_buffer_reuse_zero_aba() {
    // Construct a single in-place reusable byte buffer (exact same pointer AND exact same length!)
    // for both general AST `SLICESNP\x02` (hitting `SNP_LAST_HIT`) and flat `LLInt` `SLICESNP\x02`.
    let template_src = "function id(n: number): number { return n + 100; }\nid(100);";
    let first_bc = compile_source_to_bytecode(template_src);
    assert_eq!(&first_bc[0..8], SNAPSHOT_MAGIC);
    let mut inplace_buf = first_bc.clone();

    for expected_arg in 100..140 {
        let src = format!(
            "function id(n: number): number {{ return n + 100; }}\nid({});",
            expected_arg
        );
        let next_bc = compile_source_to_bytecode(&src);
        assert_eq!(
            next_bc.len(),
            inplace_buf.len(),
            "test payloads must have identical byte length to exercise exact-address/exact-length ABA"
        );
        // Overwrite `inplace_buf` in-place so `as_ptr()` and `len()` are 100% identical across iterations!
        let ptr_before = inplace_buf.as_ptr();
        inplace_buf.copy_from_slice(&next_bc);
        assert_eq!(inplace_buf.as_ptr(), ptr_before);

        let mut iso = SliceIsolate::spawn(
            "aba-tenant",
            expected_arg as u32,
            CapabilityToken::pure_compute(),
            50_000,
            65536,
        );
        let metrics = iso.execute_bytecode(&inplace_buf, None).unwrap();
        assert_eq!(
            metrics.return_value,
            (expected_arg + 100).to_string(),
            "stale SNP_LAST_HIT ABA hit detected on iteration {}",
            expected_arg
        );
    }
}

#[test]
fn test_wave2_500_module_tier0_and_multimodule_tier1_cas_zero_disk_execution() {
    // 1. 500-module Tier-0 `.cas` archive opened from memory (`0` source files on disk!)
    let cas_path_500 = temp_cas_file("wave2_tier0_500");
    let mut b500 = CasBuilder::new("./index.ts");
    let mut imports = String::with_capacity(28_000);
    let mut sums = String::from("let total: number = 0;\n");
    for idx in 0..500 {
        let spec = format!("./mod_{}.ts", idx);
        let mod_src = format!("export const VAL_{}: number = {};\n", idx, idx + 1);
        b500.add_module(
            &spec,
            PathBuf::from(&spec),
            &mod_src,
            compile_source_to_bytecode,
        );
        imports.push_str(&format!(
            "import {{ VAL_{} }} from \"./mod_{}.ts\";\n",
            idx, idx
        ));
        sums.push_str(&format!("total += VAL_{};\n", idx));
    }
    imports.push_str(&sums);
    imports.push_str("console.log(total);\n");
    b500.add_module(
        "./index.ts",
        PathBuf::from("./index.ts"),
        &imports,
        compile_source_to_bytecode,
    );
    b500.write_to_file(&cas_path_500).unwrap();
    let raw_500 = std::fs::read(&cas_path_500).unwrap();
    let _ = std::fs::remove_file(&cas_path_500);

    let mapped_500 = MappedCasArchive::from_bytes(&raw_500).unwrap();
    let entry_500 = mapped_500.entrypoint_blob().unwrap();
    assert_eq!(
        &mapped_500.get_bytecode_slice(entry_500)[0..8],
        SNAPSHOT_MAGIC
    );
    let mut iso_500 = SliceIsolate::spawn(
        "cas-500",
        0,
        CapabilityToken::pure_compute(),
        1_000_000,
        1024 * 1024,
    );
    let m500 = iso_500
        .execute_bytecode(mapped_500.get_bytecode_slice(entry_500), Some(&mapped_500))
        .unwrap();
    assert_eq!(m500.exit_code, None);
    assert_eq!(m500.stdout, vec!["125250".to_string()]);
    assert_eq!(m500.cas_modules_resolved, 500);
    assert_eq!(m500.os_stat_syscalls, 0);

    // 2. Multi-module Tier-1 `.cas` archive (`0` source files on disk!) exercising:
    //    - nested `./sub/url_service.ts` + `../shared/num.ts` (Tier-0 `SLICESNP` imported by Tier-1 Boa!)
    //    - `.js` -> `.ts` resolution
    //    - child module using `new URL(...)` (Shard 1 auto-loaded on demand!)
    //    - circular CJS/ESM dependency (`cycle_a.ts` <-> `cycle_b.ts`)
    //    - falsy `module.exports = 0` & `module.exports = false` (proving `__slice_cas_miss` Symbol sentinel!)
    //    - nested `Slice.evalSandbox` isolation (child sandbox cannot `require` parent `.cas` blobs!)
    let cas_path_t1 = temp_cas_file("wave2_tier1_multi");
    let mut bt1 = CasBuilder::new("./src/index.ts");

    bt1.add_module(
        "./src/shared/num.ts",
        PathBuf::from("./src/shared/num.ts"),
        "export const BASE_PORT: number = 8443;\n",
        compile_source_to_bytecode,
    );
    bt1.add_module(
        "./src/shared/zero.cjs",
        PathBuf::from("./src/shared/zero.cjs"),
        "module.exports = 0;\n",
        compile_source_to_bytecode,
    );
    bt1.add_module(
        "./src/shared/falsey.cjs",
        PathBuf::from("./src/shared/falsey.cjs"),
        "module.exports = false;\n",
        compile_source_to_bytecode,
    );
    bt1.add_module(
        "./src/sub/cycle_a.ts",
        PathBuf::from("./src/sub/cycle_a.ts"),
        "exports.stage = 'a_init';\nconst b = require('./cycle_b.ts');\nexports.fromB = b.reply;\nexports.stage = 'a_done';\n",
        compile_source_to_bytecode,
    );
    bt1.add_module(
        "./src/sub/cycle_b.ts",
        PathBuf::from("./src/sub/cycle_b.ts"),
        "const a = require('./cycle_a.ts');\nexports.reply = 'seen_' + a.stage;\n",
        compile_source_to_bytecode,
    );
    bt1.add_module(
        "./src/sub/url_service.ts",
        PathBuf::from("./src/sub/url_service.ts"),
        r#"
import { BASE_PORT } from "../shared/num.js";
export default class EndpointBuilder {
    private prefix: string;
    constructor(prefix: string) {
        this.prefix = prefix;
    }
    build(path: string): string {
        const u = new URL("https://edge.slice.local:" + BASE_PORT + this.prefix + path);
        return u.host + u.pathname;
    }
}
export async function buildAsync(path: string): Promise<string> {
    const eb = new EndpointBuilder("/v2");
    return eb.build(path);
}
"#,
        compile_source_to_bytecode,
    );
    bt1.add_module(
        "./src/index.ts",
        PathBuf::from("./src/index.ts"),
        r#"
import EndpointBuilder, { buildAsync } from "./sub/url_service.js";
const zeroVal = require("./shared/zero.cjs");
const falseVal = require("./shared/falsey.cjs");
const cyc = require("./sub/cycle_a.ts");

const childSandbox = Slice.evalSandbox("typeof require('./sub/url_service.js') === 'undefined' ? 'ISOLATED' : 'LEAKED'");
const built = await buildAsync("/health");
const direct = new EndpointBuilder("/v1").build("/ok");
console.log([built, direct, zeroVal, falseVal, cyc.fromB, cyc.stage, childSandbox.returnValue].join("|"));
"#,
        compile_source_to_bytecode,
    );
    bt1.write_to_file(&cas_path_t1).unwrap();
    let raw_t1 = std::fs::read(&cas_path_t1).unwrap();
    let _ = std::fs::remove_file(&cas_path_t1);

    let mapped_t1 = MappedCasArchive::from_bytes(&raw_t1).unwrap();
    let entry_t1 = mapped_t1.entrypoint_blob().unwrap();
    let mut iso_t1 = SliceIsolate::spawn(
        "cas-tier1",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    );
    let mt1 = iso_t1
        .execute_bytecode(mapped_t1.get_bytecode_slice(entry_t1), Some(&mapped_t1))
        .unwrap();
    assert_eq!(
        mt1.exit_code, None,
        "unexpected Tier-1 multi-blob CAS error: return={:?}, stdout={:?}",
        mt1.return_value, mt1.stdout
    );
    assert_eq!(
        mt1.stdout,
        vec![
            "edge.slice.local:8443/v2/health|edge.slice.local:8443/v1/ok|0|false|seen_a_init|a_done|ISOLATED"
                .to_string()
        ]
    );
    assert!(
        mt1.cas_modules_resolved >= 6,
        "expected >= 6 resolved CAS modules, got {}",
        mt1.cas_modules_resolved
    );
}

#[test]
fn test_wave2_ts_decorators_accessor_using_cjs_equals_and_standalone_export_runtime() {
    // 1. Decorators + Stage-3 `accessor` + ES2026 `using` (normal + SuppressedError) executing in Boa
    let ts_modern = r#"
const events: string[] = [];
function tagClass(label: string) {
    return function(ctor: any) {
        ctor.tag = label;
        events.push("cls:" + label);
        return ctor;
    };
}
function doubleResult(_target: any, key: string, desc: PropertyDescriptor) {
    const orig = desc.value;
    desc.value = function(...args: any[]) {
        events.push("call:" + key);
        return orig.apply(this, args) * 2;
    };
    return desc;
}

@tagClass("svc")
class Calculator {
    accessor base: number = 10;
    static accessor version: number = 3;

    @doubleResult
    compute(delta: number): number {
        return this.base + delta + Calculator.version;
    }
}

{
    using guard = {
        [Symbol.dispose]() {
            events.push("disposed:normal");
        }
    };
    events.push("inside");
}

let suppressedCaught = false;
try {
    using badGuard = {
        [Symbol.dispose]() {
            throw new Error("dispose_boom");
        }
    };
    throw new Error("body_boom");
} catch (err: any) {
    suppressedCaught = err.name === "SuppressedError"
        && err.error.message === "dispose_boom"
        && err.suppressed.message === "body_boom";
}

const c = new Calculator();
c.base = 20;
const result = c.compute(5);
console.log(JSON.stringify({
    tag: (Calculator as any).tag,
    result,
    events,
    suppressedCaught,
}));
"#;

    let bc_modern = compile_source_to_bytecode(ts_modern);
    let mut iso_mod = SliceIsolate::spawn(
        "ts-modern",
        0,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    );
    let m_mod = iso_mod.execute_bytecode(&bc_modern, None).unwrap();
    assert_eq!(m_mod.exit_code, None, "return={:?}", m_mod.return_value);
    let parsed: serde_json::Value = serde_json::from_str(&m_mod.stdout[0]).unwrap();
    assert_eq!(parsed["tag"], "svc");
    assert_eq!(parsed["result"], 56); // (20 + 5 + 3) * 2
    assert_eq!(parsed["suppressedCaught"], true);
    assert_eq!(
        parsed["events"],
        serde_json::json!(["cls:svc", "inside", "disposed:normal", "call:compute"])
    );

    // 2. TypeScript `import path = require("node:path")`, bare `import crypto from "crypto"`,
    //    and standalone `export const x = 42; x` under both default (Tier-0) and forced `--tier=1`!
    let ts_import_eq = r#"
import path = require("node:path");
const joined: string = path.posix.join("/edge", "v1", "ok");
console.log(joined);
"#;
    let bc_ieq = compile_source_to_bytecode(ts_import_eq);
    let mut iso_ieq = SliceIsolate::spawn(
        "ts-ieq",
        1,
        CapabilityToken::pure_compute(),
        2_000_000,
        8 * 1024 * 1024,
    );
    let m_ieq = iso_ieq.execute_bytecode(&bc_ieq, None).unwrap();
    assert_eq!(m_ieq.exit_code, None, "return={:?}", m_ieq.return_value);
    assert_eq!(m_ieq.stdout, vec!["/edge/v1/ok".to_string()]);

    let bare_builtin_esm = r#"
import crypto from "crypto";
console.log(crypto.createHash("sha256").update("slice").digest("hex").slice(0, 8));
"#;
    let bc_bare = compile_source_to_bytecode(bare_builtin_esm);
    let mut iso_bare = SliceIsolate::spawn(
        "bare-esm",
        2,
        CapabilityToken::pure_compute(),
        2_000_000,
        8 * 1024 * 1024,
    );
    let m_bare = iso_bare.execute_bytecode(&bc_bare, None).unwrap();
    assert_eq!(m_bare.exit_code, None, "return={:?}", m_bare.return_value);
    assert_eq!(m_bare.stdout[0].len(), 8);

    // Standalone `export const x = 42; x` in Tier-0 AND forced `--tier=1`
    let standalone_exp = "export const x: number = 42;\nx;";
    let bc_t0 = compile_source_to_bytecode(standalone_exp);
    let mut iso_t0 =
        SliceIsolate::spawn("exp-t0", 3, CapabilityToken::pure_compute(), 100_000, 65536);
    let m_t0 = iso_t0.execute_bytecode(&bc_t0, None).unwrap();
    assert_eq!(m_t0.exit_code, None);
    assert_eq!(m_t0.return_value, "42");

    let _tier_guard = ThreadTierOverrideGuard;
    set_thread_tier_override(Some(1));
    let bc_t1 = compile_source_to_bytecode(standalone_exp);
    set_thread_tier_override(None);
    let mut iso_t1 = SliceIsolate::spawn(
        "exp-t1",
        4,
        CapabilityToken::pure_compute(),
        2_000_000,
        8 * 1024 * 1024,
    );
    let m_t1 = iso_t1.execute_bytecode(&bc_t1, None).unwrap();
    assert_eq!(m_t1.exit_code, None, "return={:?}", m_t1.return_value);
    assert_eq!(m_t1.return_value, "42");
}
