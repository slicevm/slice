use crate::*;

#[test]
fn test_ts_stripping_and_cas_roundtrip() {
    let ts_code = r#"
interface User {
    id: number;
    name: string;
}
type ID = string;
const x: number = 42;
function add(a: number, b: number): number {
    return a + b;
}
"#;
    let stripped = strip_typescript_and_normalize(ts_code, true);
    assert!(!stripped.contains("interface User"));
    assert!(!stripped.contains("type ID"));
    assert!(stripped.contains("const x = 42;"));

    let temp_path = std::env::temp_dir().join("slice_test_roundtrip.cas");
    let mut builder = CasBuilder::new("./index.ts");
    builder.add_module("./index.ts", PathBuf::from("index.ts"), ts_code, |src| {
        src.as_bytes().to_vec()
    });
    builder.write_to_file(&temp_path).unwrap();

    let archive = MappedCasArchive::open(&temp_path).unwrap();
    let entry = archive.resolve_specifier("./index.ts").unwrap();
    let loaded_src = archive.get_source_slice(entry);
    assert!(loaded_src.contains("const x = 42;"));
    let _ = std::fs::remove_file(&temp_path);
}

#[test]
fn test_embed_cas_into_executable_roundtrip() {
    let tmp = std::env::temp_dir();
    let fake_exe = tmp.join("slice_fake_base.bin");
    let cas_path = tmp.join("slice_embed_test.cas");
    let out_exe = tmp.join("slice_standalone_out.bin");

    std::fs::write(&fake_exe, vec![0x90u8; 100]).unwrap();
    let mut builder = CasBuilder::new("./index.ts");
    builder.add_module(
        "./index.ts",
        PathBuf::from("index.ts"),
        "const a = 7;",
        |s| s.as_bytes().to_vec(),
    );
    builder.write_to_file(&cas_path).unwrap();

    let total_size = embed_cas_into_executable(&fake_exe, &cas_path, &out_exe).unwrap();
    assert!(total_size > 128);

    let raw_out = std::fs::read(&out_exe).unwrap();
    let tail = &raw_out[raw_out.len() - 24..];
    assert_eq!(&tail[16..24], b"SLICEEXE");
    let cas_offset = u64::from_le_bytes(tail[0..8].try_into().unwrap()) as usize;
    let cas_len = u64::from_le_bytes(tail[8..16].try_into().unwrap()) as usize;
    assert_eq!(cas_offset % CACHE_LINE_ALIGN, 0);

    let opened = MappedCasArchive::open_from_offset(&out_exe, cas_offset, cas_len).unwrap();
    let entry = opened.entrypoint_blob().unwrap();
    assert!(opened.get_source_slice(entry).contains("const a = 7;"));

    let _ = std::fs::remove_file(&fake_exe);
    let _ = std::fs::remove_file(&cas_path);
    let _ = std::fs::remove_file(&out_exe);
}

#[test]
fn test_corrupted_cas_bounds_and_utf8_rejection() {
    let temp_dir = std::env::temp_dir().join("slice_cas_safety_test");
    let _ = std::fs::create_dir_all(&temp_dir);

    // 1. Valid archive baseline
    let valid_path = temp_dir.join("valid.cas");
    let mut builder = CasBuilder::new("./index.js");
    builder.add_module(
        "./index.js",
        PathBuf::from("index.js"),
        "console.log('ok');",
        |src| src.as_bytes().to_vec(),
    );
    builder.write_to_file(&valid_path).unwrap();
    let valid_bytes = std::fs::read(&valid_path).unwrap();
    let opened = MappedCasArchive::open(&valid_path).unwrap();
    let entry = opened.entrypoint_blob().unwrap();
    let src_byte_pos = opened.data_start_offset + (entry.offset as usize);
    drop(opened);

    // 2. Corrupted header_len exceeding file size must return Err, not panic
    let mut bad_hdr = valid_bytes.clone();
    bad_hdr[16..24].copy_from_slice(&(999_999u64).to_le_bytes());
    let bad_hdr_path = temp_dir.join("bad_hdr.cas");
    std::fs::write(&bad_hdr_path, &bad_hdr).unwrap();
    assert!(MappedCasArchive::open(&bad_hdr_path).is_err());

    // 3. Invalid UTF-8 inside source payload must return Err on open, preventing UB
    let mut bad_utf8 = valid_bytes.clone();
    bad_utf8[src_byte_pos] = 0xFF;
    let bad_utf8_path = temp_dir.join("bad_utf8.cas");
    std::fs::write(&bad_utf8_path, &bad_utf8).unwrap();
    assert!(MappedCasArchive::open(&bad_utf8_path).is_err());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_audit_neo_phase4_zero_copy_static_mmap_and_resolver_cache() {
    let temp_dir = std::env::temp_dir().join("slice_cas_phase4_test");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let static_dir = temp_dir.join("public");
    std::fs::create_dir_all(&static_dir).unwrap();
    std::fs::write(
        static_dir.join("index.html"),
        "<h1>Zero-Copy CAS Asset</h1>",
    )
    .unwrap();

    let cas_path = temp_dir.join("static_test.cas");
    let mut builder = CasBuilder::new("./index.js");
    builder.add_module(
        "./index.js",
        PathBuf::from("index.js"),
        "console.log('ready');",
        |src| src.as_bytes().to_vec(),
    );
    builder.ingest_static_directory(&static_dir).unwrap();
    builder.write_to_file(&cas_path).unwrap();

    let archive = MappedCasArchive::open(&cas_path).unwrap();
    let table = StaticAssetTable::from_cas_archive(&archive);
    let root_asset = table.resolve("/").expect("Expected /index.html at /");
    assert_eq!(&root_asset.payload[..], b"<h1>Zero-Copy CAS Asset</h1>");
    assert!(
        matches!(root_asset.payload, StaticAssetPayload::Mapped { .. }),
        "Expected StaticAssetTable::from_cas_archive to use zero-copy StaticAssetPayload::Mapped"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_mapped_cas_archive_arc_clone_and_rc_helpers() {
    let temp_dir = std::env::temp_dir().join("slice_cas_arc_clone_and_toml_comment_test");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let src_dir = temp_dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let entry_file = src_dir.join("main#entry.ts");
    std::fs::write(&entry_file, "export const x: number = 42;\n").unwrap();
    std::fs::write(
        temp_dir.join("slice.toml"),
        "# top comment\nentrypoint = \"src/main#entry.ts\" # inline comment with \"quotes\"\n",
    )
    .unwrap();

    let resolved = resolve_project_entrypoint(&temp_dir, None).unwrap();
    assert_eq!(resolved, entry_file);

    let cas_path = temp_dir.join("arc_clone.cas");
    let mut builder = CasBuilder::new("./index.ts");
    builder.add_module(
        "./index.ts",
        PathBuf::from("index.ts"),
        "const answer: number = 42;\n",
        |src| src.as_bytes().to_vec(),
    );
    builder.write_to_file(&cas_path).unwrap();

    let archive = MappedCasArchive::open(&cas_path).unwrap();
    let cloned = archive.clone();
    let inner_a: &MappedCasInner = &archive;
    let inner_b: &MappedCasInner = &cloned;
    assert!(std::ptr::eq(inner_a, inner_b));
    let entry = cloned.entrypoint_blob().unwrap();
    assert!(cloned.get_source_slice(entry).contains("42"));

    let raw_bundle = "var __slice_modules = {};//__SLICE_RC:0123456789abcdef\n// #slice-source: app.ts|/abs/app.ts|1,2\n  const msg = `line1\n  line2`;\n  // comment\n\n  console.log(msg);\n";
    let compacted = compact_js_for_boa_bytecode(raw_bundle);
    assert!(compacted.starts_with("var __slice_modules = {};//__SLICE_RC:0123456789abcdef\n"));
    assert!(compacted.contains("//@sm:app.ts|/abs/app.ts|1,2\n"));
    assert!(compacted.contains("const msg = `line1\n  line2`;\n"));
    assert!(!compacted.contains("// comment"));

    let wrapped_rc = format!("{{\n{}\n}}", compacted.trim_end());
    let (rc_u64, rc_hex) = extract_slice_rc_key(&wrapped_rc).expect("expected RC key");
    assert_eq!(rc_u64, 0x0123_4567_89ab_cdef);
    assert_eq!(rc_hex, b"0123456789abcdef");

    let _ = std::fs::remove_dir_all(&temp_dir);
}
