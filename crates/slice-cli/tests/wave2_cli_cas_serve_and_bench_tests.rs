use crate::http_server::{serve_connection_with_isolate, spawn_worker_isolate};
use crate::{LAST_CLI_EXIT_CODE, cmd_microvm_bench, cmd_run};
use slice_cas::{CasBuilder, MappedCasArchive};
use slice_engine::compile_source_to_bytecode;
use slice_sandbox::CapabilityToken;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

fn temp_cli_dir(prefix: &str) -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), id));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp cli dir");
    dir
}

#[test]
fn test_wave2_cli_run_and_serve_multiblob_cas_without_source_files_on_disk() {
    let work_dir = temp_cli_dir("slice_wave2_cli_multiblob");
    let src_tree = work_dir.join("project");
    std::fs::create_dir_all(src_tree.join("src").join("routes")).unwrap();

    // Include `.d.ts` to verify `ingest_directory` ignores declarations end-to-end
    std::fs::write(
        src_tree.join("src").join("types.d.ts"),
        "export interface RouteReply { version: string; path: string; }\n",
    )
    .unwrap();
    std::fs::write(
        src_tree.join("src").join("routes").join("formatter.ts"),
        r#"
export class ReplyFormatter {
    private tag: string;
    constructor(tag: string) {
        this.tag = tag;
    }
    format(pathname: string): string {
        return JSON.stringify({ tag: this.tag, path: pathname });
    }
}
"#,
    )
    .unwrap();
    std::fs::write(
        src_tree.join("src").join("index.ts"),
        r#"
import { ReplyFormatter } from "./routes/formatter.js";
const fmt = new ReplyFormatter("v1-multiblob");
export default {
    async fetch(req: Request): Promise<Response> {
        const u = new URL(req.url);
        return new Response(fmt.format(u.pathname), {
            status: 200,
            headers: { "content-type": "application/json" },
        });
    }
};
"#,
    )
    .unwrap();

    let cas_file = work_dir.join("multiblob_http.cas");
    let mut builder = CasBuilder::new("./src/index.ts");
    let count = builder
        .ingest_directory(&src_tree, &compile_source_to_bytecode)
        .unwrap();
    assert_eq!(count, 2);
    builder.write_to_file(&cas_file).unwrap();

    // Delete `src_tree` completely so zero source files exist on disk!
    std::fs::remove_dir_all(&src_tree).unwrap();
    assert!(!src_tree.exists());

    // 1. `slice run` cold and `--warm` on multi-blob Tier-1 `.cas`
    let cas_arg = cas_file.to_string_lossy().to_string();
    cmd_run(&[cas_arg.clone(), "--quiet".to_string()]).expect("cold slice run multiblob .cas");
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);
    cmd_run(&[cas_arg, "--warm".to_string(), "--quiet".to_string()])
        .expect("warm slice run multiblob .cas");
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    // 2. `slice serve` connection dispatch with `Some(&mapped_cas)`
    let mapped = MappedCasArchive::open(&cas_file).unwrap();
    let entry_bc = mapped
        .get_bytecode_slice(mapped.entrypoint_blob().unwrap())
        .to_vec();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let srv_handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let caps = CapabilityToken::pure_compute();
        let mut iso = spawn_worker_isolate(1, None, &caps, &[], 5_000_000, 16 * 1024 * 1024);
        let mut rbuf = vec![0u8; 16384];
        let mut wbuf = Vec::with_capacity(4096);
        serve_connection_with_isolate(
            stream,
            1,
            &mut iso,
            &entry_bc,
            Some(&mapped),
            None,
            None,
            5_000_000,
            16 * 1024 * 1024,
            &mut rbuf,
            &mut wbuf,
            None,
            None,
        )
        .unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client
        .write_all(b"GET /orders/42 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut resp = String::new();
    client.read_to_string(&mut resp).unwrap();
    srv_handle.join().unwrap();
    assert!(resp.starts_with("HTTP/1.1 200 OK\r\n"), "resp={resp}");
    assert!(
        resp.contains(r#"{"tag":"v1-multiblob","path":"/orders/42"}"#),
        "resp={resp}"
    );

    // 3. `--tenant-dir` multi-blob `.cas` dispatch + zero-downtime in-place hot-swap on Windows!
    let tenant_dir = work_dir.join("tenants");
    std::fs::create_dir_all(&tenant_dir).unwrap();
    let alpha_cas = tenant_dir.join("tenant-alpha.cas");
    std::fs::copy(&cas_file, &alpha_cas).unwrap();

    let listener2 = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port2 = listener2.local_addr().unwrap().port();
    let td_clone = tenant_dir.clone();
    let default_bc = compile_source_to_bytecode("console.log('unused');");
    let srv_handle2 = std::thread::spawn(move || {
        let caps = CapabilityToken::pure_compute();
        let mut iso = spawn_worker_isolate(10, None, &caps, &[], 5_000_000, 16 * 1024 * 1024);
        let mut rbuf = vec![0u8; 16384];
        let mut wbuf = Vec::with_capacity(4096);
        for _ in 0..2 {
            let (stream, _) = listener2.accept().unwrap();
            serve_connection_with_isolate(
                stream,
                10,
                &mut iso,
                &default_bc,
                None,
                Some(&td_clone),
                None,
                5_000_000,
                16 * 1024 * 1024,
                &mut rbuf,
                &mut wbuf,
                None,
                None,
            )
            .unwrap();
        }
    });

    let mut c1 = TcpStream::connect(("127.0.0.1", port2)).unwrap();
    c1.write_all(
        b"GET /hot/1 HTTP/1.1\r\nHost: localhost\r\nX-Slice-Tenant: tenant-alpha\r\nConnection: close\r\n\r\n",
    )
    .unwrap();
    let mut r1 = String::new();
    c1.read_to_string(&mut r1).unwrap();
    assert!(
        r1.contains(r#"{"tag":"v1-multiblob","path":"/hot/1"}"#),
        "r1={r1}"
    );

    // Overwrite `tenant-alpha.cas` in-place while cached in `TENANT_CAS_CACHE` (verifying `open_detached`!)
    let mut b_v2 = CasBuilder::new("./src/index.ts");
    b_v2.add_module(
        "./src/dep.ts",
        PathBuf::from("./src/dep.ts"),
        "export const LABEL: string = 'v2-hotswapped-multiblob';\n",
        compile_source_to_bytecode,
    );
    b_v2.add_module(
        "./src/index.ts",
        PathBuf::from("./src/index.ts"),
        r#"
import { LABEL } from "./dep.js";
export default {
    fetch(req: Request): Response {
        return new Response(LABEL + ":" + new URL(req.url).pathname);
    }
};
"#,
        compile_source_to_bytecode,
    );
    // Ensure distinct file size & wait past 50ms stat TTL so hot-swap triggers deterministically
    std::thread::sleep(std::time::Duration::from_millis(65));
    b_v2.write_to_file(&alpha_cas)
        .expect("in-place overwrite of cached tenant .cas must not hit os error 1224");

    let mut c2 = TcpStream::connect(("127.0.0.1", port2)).unwrap();
    c2.write_all(
        b"GET /hot/2 HTTP/1.1\r\nHost: localhost\r\nX-Slice-Tenant: tenant-alpha\r\nConnection: close\r\n\r\n",
    )
    .unwrap();
    let mut r2 = String::new();
    c2.read_to_string(&mut r2).unwrap();
    srv_handle2.join().unwrap();
    assert!(r2.contains("v2-hotswapped-multiblob:/hot/2"), "r2={r2}");

    let _ = std::fs::remove_dir_all(&work_dir);
}

#[test]
fn test_wave2_microvm_bench_verifies_500_module_cas_and_compute_results() {
    cmd_microvm_bench(&["--json".to_string()]).expect(
        "cmd_microvm_bench must pass all 1000-tenant, 500-module CAS, and fib(40) assertions",
    );
}
