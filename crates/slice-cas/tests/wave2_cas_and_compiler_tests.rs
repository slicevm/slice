use crate::{
    CasBuilder, MappedCasArchive, StaticAssetTable, bundle_entrypoint_with_deps,
    embed_cas_into_executable, inspect_sliceexe_trailer, lower_unbundled_esm,
    strip_typescript_and_normalize_with_ext,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

fn temp_test_dir(prefix: &str) -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), id));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

#[test]
fn test_wave2_cas_builder_static_magic_collision_dts_skip_sibling_stems_and_per_blob_sourcemap() {
    let root = temp_test_dir("slice_wave2_cas_integrity");
    let src_dir = root.join("src");
    let sub_dir = src_dir.join("sub");
    let pub_dir = root.join("public");
    std::fs::create_dir_all(&sub_dir).unwrap();
    std::fs::create_dir_all(&pub_dir).unwrap();

    // Declaration files that `ingest_directory` must skip completely
    std::fs::write(
        src_dir.join("ambient.d.ts"),
        "export interface Ghost { x: number; }\n",
    )
    .unwrap();
    std::fs::write(
        src_dir.join("types.d.mts"),
        "export type GhostMts = string;\n",
    )
    .unwrap();
    std::fs::write(
        src_dir.join("legacy.d.cts"),
        "export declare const nope: number;\n",
    )
    .unwrap();

    // Sibling stems: `./src/index.ts` is the configured entrypoint; `./src/index.cjs` shares the stem
    std::fs::write(
        src_dir.join("index.ts"),
        "// #slice-source: index.ts|src/index.ts|\nimport { helperVal } from \"./sub/helper.js\";\nexport const mainVal: number = helperVal + 1;\n",
    )
    .unwrap();
    std::fs::write(
        src_dir.join("index.cjs"),
        "// #slice-source: index.cjs|src/index.cjs|\nmodule.exports = { cjsOnly: true };\n",
    )
    .unwrap();
    std::fs::write(
        sub_dir.join("helper.ts"),
        "// #slice-source: helper.ts|src/sub/helper.ts|\nimport { leaf } from \"./dir\";\nexport const helperVal: number = leaf * 2;\n",
    )
    .unwrap();
    let nested_idx_dir = sub_dir.join("dir");
    std::fs::create_dir_all(&nested_idx_dir).unwrap();
    std::fs::write(
        nested_idx_dir.join("index.ts"),
        "export const leaf: number = 20;\n",
    )
    .unwrap();

    // Static assets whose raw payloads start with `SLICEBC\x01` and `SLICEBOA{\n...\n}` magic bytes!
    let mut magic_bc_asset = b"SLICEBC\x01".to_vec();
    magic_bc_asset.extend_from_slice(&[0u8; 40]);
    std::fs::write(pub_dir.join("magic_bc.bin"), &magic_bc_asset).unwrap();

    let magic_boa_asset = b"SLICEBOA{\nconsole.log('not js');\n}".to_vec();
    std::fs::write(pub_dir.join("magic_boa.txt"), &magic_boa_asset).unwrap();

    let mut builder = CasBuilder::new("./src/index.ts");
    let ingested = builder
        .ingest_directory(&root, &|norm| {
            let mut out = b"SLICEBOA{\n".to_vec();
            out.extend_from_slice(norm.trim_end().as_bytes());
            out.extend_from_slice(b"\n}");
            out
        })
        .unwrap();
    assert_eq!(
        ingested, 4,
        "ingest_directory must ingest 4 code modules and skip all 3 .d.ts/.d.mts/.d.cts files"
    );
    let static_count = builder.ingest_static_directory(&pub_dir).unwrap();
    assert_eq!(static_count, 2);

    let cas_path = root.join("app.cas");
    builder.write_to_file(&cas_path).unwrap();

    let mapped = MappedCasArchive::open(&cas_path).unwrap();
    assert!(mapped.resolve_specifier("./src/ambient.d.ts").is_none());
    assert!(mapped.resolve_specifier("./src/types.d.mts").is_none());
    assert!(mapped.resolve_specifier("./src/legacy.d.cts").is_none());

    // Entrypoint must remain `./src/index.ts`, never clobbered by sibling `./src/index.cjs`
    let entry = mapped.entrypoint_blob().expect("entrypoint blob");
    assert_eq!(entry.original_specifier, "./src/index.ts");
    let cjs_entry = mapped
        .resolve_specifier("./src/index.cjs")
        .expect("exact sibling index.cjs");
    assert_eq!(cjs_entry.original_specifier, "./src/index.cjs");
    assert_ne!(entry.hash_hex, cjs_entry.hash_hex);

    // Per-blob source map reconstruction must scope markers strictly per blob hash
    let entry_annotated = mapped.reconstruct_annotated_source(entry);
    assert!(
        entry_annotated.contains("src/index.ts"),
        "expected index.ts marker in: {entry_annotated}"
    );
    assert!(
        !entry_annotated.contains("src/index.cjs"),
        "index.ts source map must not contain sibling index.cjs markers: {entry_annotated}"
    );
    let cjs_annotated = mapped.reconstruct_annotated_source(cjs_entry);
    assert!(
        cjs_annotated.contains("src/index.cjs"),
        "expected index.cjs marker in: {cjs_annotated}"
    );

    // Relative, `.js` -> `.ts`, directory `/index`, and traversal-guard resolution
    let resolved_helper = mapped
        .resolve_specifier_from(Some("./src/index.ts"), "./sub/helper.js")
        .expect("resolve .js -> .ts relative to ./src/index.ts");
    assert_eq!(resolved_helper.original_specifier, "./src/sub/helper.ts");

    let resolved_dir_idx = mapped
        .resolve_specifier_from(Some("./src/sub/helper.ts"), "./dir")
        .expect("resolve ./dir -> ./src/sub/dir/index.ts");
    assert_eq!(
        resolved_dir_idx.original_specifier,
        "./src/sub/dir/index.ts"
    );

    let resolved_parent = mapped
        .resolve_specifier_from(Some("./src/sub/dir/index.ts"), "../../index.ts")
        .expect("resolve ../../index.ts within archive");
    assert_eq!(resolved_parent.original_specifier, "./src/index.ts");

    assert!(
        mapped
            .resolve_specifier_from(Some("./src/index.ts"), "../../escape.ts")
            .is_none(),
        ".. escaping above archive root must fail closed"
    );
    assert!(
        mapped
            .resolve_specifier_from(Some("./src/index.ts"), "static:/magic_bc.bin")
            .is_none(),
        "module resolver must never resolve static:/ blobs"
    );

    // Static assets starting with SLICEBC\x01 / SLICEBOA must keep intact MIME & payload bytes!
    let static_table = StaticAssetTable::from_cas_archive(&mapped);
    let bc_asset = static_table
        .by_path
        .get("/magic_bc.bin")
        .expect("magic_bc.bin static asset");
    assert_eq!(bc_asset.content_type, "application/octet-stream");
    assert_eq!(&*bc_asset.payload, magic_bc_asset.as_slice());

    let boa_asset = static_table
        .by_path
        .get("/magic_boa.txt")
        .expect("magic_boa.txt static asset");
    assert!(boa_asset.content_type.starts_with("text/plain"));
    assert_eq!(&*boa_asset.payload, magic_boa_asset.as_slice());

    drop(static_table);
    drop(mapped);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_wave2_sliceexe_trailer_bounds_and_idempotent_3x_repack() {
    let dir = temp_test_dir("slice_wave2_sliceexe");

    // 1. Corrupt SLICEEXE trailer bounds checks
    let mut bad_overlap = vec![0xAAu8; 64];
    let file_len_after = (bad_overlap.len() + 24) as u64;
    // Claim cas_offset = 40, cas_len = 40 -> cas_end = 80 > file_len - 24 (64)!
    bad_overlap.extend_from_slice(&40u64.to_le_bytes());
    bad_overlap.extend_from_slice(&40u64.to_le_bytes());
    bad_overlap.extend_from_slice(b"SLICEEXE");
    assert_eq!(bad_overlap.len() as u64, file_len_after);
    let bad_overlap_path = dir.join("bad_overlap.exe");
    std::fs::write(&bad_overlap_path, &bad_overlap).unwrap();
    let mut bad_overlap_file = std::fs::File::open(&bad_overlap_path).unwrap();
    assert!(
        inspect_sliceexe_trailer(&mut bad_overlap_file).is_err(),
        "cas_end overlapping the 24-byte SLICEEXE trailer must be rejected"
    );

    let mut bad_overflow = vec![0xBBu8; 64];
    bad_overflow.extend_from_slice(&u64::MAX.to_le_bytes());
    bad_overflow.extend_from_slice(&64u64.to_le_bytes());
    bad_overflow.extend_from_slice(b"SLICEEXE");
    let bad_overflow_path = dir.join("bad_overflow.exe");
    std::fs::write(&bad_overflow_path, &bad_overflow).unwrap();
    let mut bad_overflow_file = std::fs::File::open(&bad_overflow_path).unwrap();
    assert!(inspect_sliceexe_trailer(&mut bad_overflow_file).is_err());

    // 2. Idempotent 3x `embed_cas_into_executable` without duplicate payload growth
    let base_bin = dir.join("fake_slice_host.bin");
    std::fs::write(&base_bin, vec![0x7Fu8; 512]).unwrap();

    let cas_v1 = dir.join("v1.cas");
    let mut b1 = CasBuilder::new("./index.ts");
    b1.add_module(
        "./index.ts",
        PathBuf::from("./index.ts"),
        "export const V = 1;\n",
        |s| {
            let mut v = b"SLICEBOA".to_vec();
            v.extend_from_slice(s.as_bytes());
            v
        },
    );
    b1.write_to_file(&cas_v1).unwrap();

    let cas_v2 = dir.join("v2.cas");
    let mut b2 = CasBuilder::new("./index.ts");
    b2.add_module(
        "./index.ts",
        PathBuf::from("./index.ts"),
        "export const V = 222;\n",
        |s| {
            let mut v = b"SLICEBOA".to_vec();
            v.extend_from_slice(s.as_bytes());
            v
        },
    );
    b2.write_to_file(&cas_v2).unwrap();

    let out_exe = dir.join("standalone.exe");
    let len1 = embed_cas_into_executable(&base_bin, &cas_v1, &out_exe).unwrap();
    let len2 = embed_cas_into_executable(&out_exe, &cas_v1, &out_exe).unwrap();
    assert_eq!(
        len2, len1,
        "re-embedding identical CAS must keep exact binary size"
    );

    let len3 = embed_cas_into_executable(&out_exe, &cas_v2, &out_exe).unwrap();
    let expected_v2_len = 512 + std::fs::metadata(&cas_v2).unwrap().len() + 24;
    assert_eq!(
        len3, expected_v2_len,
        "re-embedding updated CAS onto existing SLICEEXE must strip prior payload cleanly"
    );

    let mut out_exe_file = std::fs::File::open(&out_exe).unwrap();
    let (cas_off, cas_len) = inspect_sliceexe_trailer(&mut out_exe_file)
        .expect("inspect trailer")
        .expect("valid trailer after 3x repack");
    assert_eq!(cas_off, 512);
    let exe_bytes = std::fs::read(&out_exe).unwrap();
    let embedded =
        MappedCasArchive::from_bytes(&exe_bytes[cas_off as usize..(cas_off + cas_len) as usize])
            .expect("open v2 cas");
    let entry = embedded.entrypoint_blob().unwrap();
    assert!(embedded.get_source_slice(entry).contains("222"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_wave2_oxc_ts_decorators_accessor_using_cjs_equals_and_whitespaceless_esm() {
    // 1. Parameter property `readonly`, BOF `abstract class`, and `\ntype` without space padding
    let tricky_ts = "abstract class Base {\n  abstract id(): number;\n}\ntype Num = number;\nclass Impl extends Base {\n  constructor(readonly val: Num = 7) { super(); }\n  id(): Num { return this.val; }\n}\n";
    let lowered_tricky = strip_typescript_and_normalize_with_ext(tricky_ts, false, false);
    assert!(!lowered_tricky.contains("abstract "));
    assert!(!lowered_tricky.contains("readonly "));
    assert!(lowered_tricky.contains("this.val = val"));

    // 2. Decorators, Stage-3 `accessor`, and ES2026 `using`
    let dec_acc_using = r#"
function doubleDec(target: any) { return target; }
@doubleDec
class Counter {
    accessor count: number = 5;
    static accessor label: string = "ok";
}
{
    using res = { [Symbol.dispose]() {} };
}
"#;
    let lowered_modern = strip_typescript_and_normalize_with_ext(dec_acc_using, true, false);
    assert!(
        lowered_modern.contains("babelHelpers.decorate"),
        "expected decorate helper in:\n{lowered_modern}"
    );
    assert!(
        lowered_modern.contains("babelHelpers.usingCtx"),
        "expected usingCtx helper in:\n{lowered_modern}"
    );
    assert!(
        lowered_modern.contains("get count()") && lowered_modern.contains("set count("),
        "expected lowered getter/setter in:\n{lowered_modern}"
    );
    assert!(
        !lowered_modern.contains("accessor "),
        "raw accessor keyword must be stripped:\n{lowered_modern}"
    );

    let js_acc = "class JsAcc { accessor count = 5; }\n";
    let lowered_js_acc = strip_typescript_and_normalize_with_ext(js_acc, false, false);
    assert!(
        lowered_js_acc.contains("get count()") && !lowered_js_acc.contains("accessor "),
        "JS is_ts=false accessor must also lower cleanly:\n{lowered_js_acc}"
    );

    // 3. TypeScript `import x = require(...)` and `export = ...`
    let ts_cjs = "import path = require(\"node:path\");\nconst out: string = path.posix.join(\"a\", \"b\");\nexport = out;\n";
    let lowered_cjs = strip_typescript_and_normalize_with_ext(ts_cjs, true, false);
    assert!(
        lowered_cjs.contains("__slice_cjs_require(\"node:path\")"),
        "expected rewritten require in:\n{lowered_cjs}"
    );
    assert!(
        lowered_cjs.contains("module.exports = out"),
        "expected module.exports = out in:\n{lowered_cjs}"
    );

    // 4. Standalone `export const` lowering via `lower_unbundled_esm`
    let standalone_exp = lower_unbundled_esm("export const answer = 42;\nanswer;");
    assert!(standalone_exp.contains("exports.answer = answer"));

    // 5. Bundler with whitespace-less ESM & bare Node builtin imports
    let dir = temp_test_dir("slice_wave2_bundler_edges");
    std::fs::write(dir.join("dep.ts"), "export const X: number = 19;\n").unwrap();
    std::fs::write(dir.join("re.ts"), "export*from\"./dep.js\";\n").unwrap();
    std::fs::write(
        dir.join("entry.ts"),
        "import path from \"path\";\nimport{X}from\"./re.js\";\nconsole.log(path.posix.basename(\"/a/b\") + \":\" + X);\n",
    )
    .unwrap();
    let bundled = bundle_entrypoint_with_deps(&dir.join("entry.ts")).unwrap();
    assert!(bundled.contains("var __slice_modules ="));
    assert!(bundled.contains("node:path"));

    let _ = std::fs::remove_dir_all(&dir);
}
