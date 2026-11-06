//! `slice` (Nexus-JS): Production Next-Generation MicroVM-Grade Edge Runtime CLI
//!
//! Production Commands:
//! - `slice init [dir]`
//! - `slice run [file.ts|file.tsx|file.js|file.jsx|archive.cas] [--allow-read=/path] [--allow-net=host] [--allow-env=KEY=VAL] [--quiet] [--metrics]`
//! - `slice pack <entry_or_dir> <output.cas> [--entry=./index.ts]`
//! - `slice compile <entry_or_dir> -o <standalone_binary>`
//! - `slice inspect <archive.cas|script.ts>`
//! - `slice serve [archive.cas|script.ts|script.tsx] [--port=3000] [--workers=N] [--tenant-dir=<dir>] [--smoke-test]`
//! - `slice mcp [--smoke-test] [--audit-egress=<secret>]`
//! - `slice repl`
//! - `slice doctor`
//! - `slice microvm-bench [--tenants=1000] [--json]`
//! - `slice verify-security`
//! - `slice info`

mod config_and_watch;
mod http_server;
mod mcp_and_repl;
mod test_runner;

#[cfg(test)]
mod cli_tests;

use crate::config_and_watch::{
    build_cas_archive_with_static, install_ctrlc_handler, OsNativeFileWatcher,
    ResolvedExecutionConfig,
};
use crate::http_server::cmd_serve;
use crate::mcp_and_repl::{cmd_doctor, cmd_mcp, cmd_microvm_bench, cmd_repl, cmd_verify_security};
use crate::test_runner::cmd_test;
use anyhow::{bail, Context, Result};
use slice_cas::{embed_cas_into_executable, MappedCasArchive};
use slice_engine::{compile_source_to_bytecode, disassemble_bytecode, SliceIsolate};
use slice_io::CompletionRingEngine;
use slice_sandbox::SliceManifest;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let builder = std::thread::Builder::new()
            .name("slice-main".to_string())
            .stack_size(16 * 1024 * 1024);
        let handle = builder.spawn(cli_entrypoint)?;
        match handle.join() {
            Ok(res) => res,
            Err(_) => bail!("Slice main thread panicked"),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        cli_entrypoint()
    }
}

fn cli_entrypoint() -> Result<()> {
    // 1. Check if this binary is a self-contained `SLICEEXE` compiled application (`slice compile`)
    if let Ok(Some(embedded_cas)) = MappedCasArchive::open_embedded_self() {
        return run_embedded_standalone(embedded_cas);
    }

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        // If `slice.toml` exists in the current directory, show a helpful hint alongside usage
        print_usage();
        return Ok(());
    }

    match args[1].as_str() {
        "init" => cmd_init(&args[2..]),
        "run" => cmd_run(&args[2..]),
        "test" => cmd_test(&args[2..]),
        "pack" => cmd_pack(&args[2..]),
        "compile" => cmd_compile(&args[2..]),
        "inspect" | "check" => cmd_inspect(&args[2..]),
        "serve" => cmd_serve(&args[2..]),
        "mcp" => cmd_mcp(&args[2..]),
        "repl" => cmd_repl(),
        "doctor" => cmd_doctor(),
        "microvm-bench" => cmd_microvm_bench(&args[2..]),
        "verify-security" => cmd_verify_security(),
        "info" | "version" | "--version" | "-v" | "-V" => cmd_info(),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => {
            // Allow `slice <file.ts>` shorthand
            cmd_run(&args[1..])
        }
    }
}

fn run_embedded_standalone(archive: MappedCasArchive) -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let quiet = args.iter().any(|a| a == "--quiet" || a == "-q");
    let print_metrics = args.iter().any(|a| a == "--metrics");
    let warm = args.iter().any(|a| a == "--warm");
    if warm {
        SliceIsolate::prewarm_boa_pool(1);
    }

    let mut exec_cfg = ResolvedExecutionConfig::from_cli_args(&args[1..])?;
    if let Some(toml_str) = archive.get_manifest_toml() {
        exec_cfg.apply_manifest_toml(toml_str, None);
    }
    let mut isolate = SliceIsolate::spawn(
        "standalone-exe",
        1,
        exec_cfg.caps,
        exec_cfg.fuel_limit,
        exec_cfg.mem_limit,
    );
    for sec in &exec_cfg.audit_secrets {
        isolate.add_egress_audit_secret(sec.clone());
    }
    let entry = archive
        .entrypoint_blob()
        .context("Embedded CAS archive has no entrypoint blob")?;
    let bc = archive.get_bytecode_slice(entry);
    let metrics = isolate.execute_bytecode(bc, Some(&archive))?;

    if metrics.return_value.starts_with("Uncaught ") {
        let annotated = archive.reconstruct_annotated_source(entry);
        let mapped = slice_cas::map_bundled_error_location(&annotated, &metrics.return_value);
        bail!("{}", mapped);
    }

    if !quiet {
        for line in &metrics.stdout {
            println!("{}", line);
        }
    }
    if print_metrics {
        println!(
            "[Slice Standalone] Tier: {:?} | Instantiate: {:.2} us | Exec: {:.2} us | Total: {:.2} us | Memory: {} B | Stat Syscalls: 0",
            metrics.highest_tier_reached,
            metrics.instantiate_time_ns as f64 / 1_000.0,
            metrics.execution_time_ns as f64 / 1_000.0,
            metrics.total_time_ns as f64 / 1_000.0,
            metrics.memory_used_bytes
        );
    }
    Ok(())
}

fn print_usage() {
    println!(
        "slice (Nexus-JS) v{} - Production MicroVM-Grade Edge Runtime",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("Core Commands:");
    println!("  slice init [dir]                               Scaffold a new TypeScript Edge project with slice.toml");
    println!("  slice run [target] [--warm] [--watch]          Execute a script, folder app, .cas bundle, or slice.toml");
    println!("  slice test [target] [--filter=PAT] [--json]    Run Time-Travel Test Suite (< 5us VFS+KV rollback per test)");
    println!("  slice serve [target] [--port=N] [--smoke-test] Launch multi-tenant WinterTC HTTP MicroVM server");
    println!("  slice pack <input> <output.cas>                Pack a file or folder app into a Zero-Stat BLAKE3 .cas archive");
    println!("  slice compile <input> -o <binary>              Compile a script, folder app, or .cas into a standalone binary");
    println!("  slice inspect <bundle.cas|script.ts|dir>       Disassemble SLICEBC/SLICEBOA/SLICESNP bytecode and inspect CAS Radix table");
    println!("  slice mcp [--smoke-test] [--audit-egress=SEC]  Start built-in Model Context Protocol (MCP) JSON-RPC 2.0 server");
    println!("  slice repl                                     Start interactive SES-Locked TypeScript/JS shell");
    println!("  slice doctor                                   Audit host OS kernel io_uring/IOCP, MPK, Warm Pool, and OCap");
    println!("  slice microvm-bench [--tenants=1000] [--json]  Run 1,000-tenant MicroVM cold-start & density benchmark");
    println!("  slice verify-security                          Run 5-vector OCap, SES, and Fuel containment verification");
    println!("  slice info                                     Display runtime architecture and active kernel rings");
}

fn cmd_info() -> Result<()> {
    let io_engine = CompletionRingEngine::new(64, true)?;
    println!(
        "slice (Nexus-JS) v{} [{}-{}]",
        env!("CARGO_PKG_VERSION"),
        env::consts::OS,
        env::consts::ARCH
    );
    println!(
        "  I/O Completion Engine : {}",
        io_engine.status.backend_name
    );
    println!(
        "  Zero-Stat CAS Format  : SLICE_CAS v{} (64-byte aligned BLAKE3 mmap + SLICEEXE + SLICEBC\\x01)",
        slice_cas::CAS_VERSION
    );
    println!("  TypeScript & JSX/TSX  : Oxc v0.152.0 AOT Compiler + Built-In SSR (`slice:jsx`) + Tree-Shaking");
    println!("  Execution Pipeline    : Tier-1 Precompiled Boa CodeBlock (`SLICEBC\\x01` Default) + Tier-2 SIMD + Tier-0 SLICESNP");
    println!("  Native Fast Intrinsics: FIPS-180-4 SHA-256 / RFC-2104 HMAC / SemVer / In-Memory VFS Slab");
    println!(
        "  Security Architecture : SES Lockdown + TC39 OCap Compartments + Hardware PKEY/Cage"
    );
    Ok(())
}

const DEFAULT_SLICE_D_TS: &str = r#"// Slice (Nexus-JS) Ambient TypeScript Declarations (`slice.d.ts`)
// Automatically included by `tsconfig.json` for IDE IntelliSense and `tsc --noEmit`.

declare function env(key: string): string;

declare namespace WinterTC {
    function route(method: string, url: string): string;
}

declare namespace Slice {
    interface HttpRequestSnapshot {
        method: string;
        path: string;
        query: string;
        tenantId: string;
        body: string;
        headers: Record<string, string>;
    }

    interface SandboxEvalOptions {
        fuel?: number;
        memoryBytes?: number;
        allowRead?: string[];
        allowWrite?: string[];
        allowNet?: string[];
        auditSecret?: string;
    }

    interface SandboxEvalOutcome {
        ok: boolean;
        value: string;
        stdout: string[];
        fuelUsed: number;
        timeUs: number;
        violation: string | null;
    }

    interface SseStreamController {
        send(event: { event?: string; data: unknown; id?: string; retry?: number }): void;
        close(): void;
    }

    interface TcpSocketOptions {
        secureTransport?: "off" | "on" | "starttls";
        allowHalfOpen?: boolean;
    }

    interface TcpSocket {
        readonly readable: ReadableStream<Uint8Array>;
        readonly writable: WritableStream<Uint8Array>;
        readonly closed: Promise<void>;
        close(): Promise<void>;
        startTls(): TcpSocket;
    }

    namespace kv {
        function get(key: string): string | null;
        function set(key: string, value: string, ttlMs?: number): boolean;
        function del(key: string): boolean;
        function incr(key: string, delta?: number): number;
    }

    function request(): HttpRequestSnapshot;
    function respond(status: number, body: string, contentType?: string, headers?: Record<string, string>): void;
    function waitUntil(task: Promise<unknown> | (() => unknown)): void;
    function streamSSE(callback: (stream: SseStreamController) => void | Promise<void>): Response;
    function evalSandbox(code: string, options?: SandboxEvalOptions): SandboxEvalOutcome;
    function checkpoint(): number;
    function restore(snapshotId: number): boolean;
    function connect(address: string | { hostname: string; port: number }, options?: TcpSocketOptions): TcpSocket;
}

declare module "slice:test" {
    export interface SliceMatchers<T = unknown> {
        toBe(expected: T): void;
        toEqual(expected: unknown): void;
        toContain(substring: string): void;
        toBeTruthy(): void;
        toBeFalsy(): void;
        toBeGreaterThan(n: number): void;
        toBeLessThan(n: number): void;
        toThrow(expectedSubstring?: string): void;
        toViolate(expectedViolationType?: string): void;
        toStayWithinFuel(maxFuel: number): void;
        toStayWithinMemory(maxBytes: number): void;
    }
    export function describe(suiteName: string, fn: () => void | Promise<void>): void;
    export function test(specName: string, fn: () => void | Promise<void>): void;
    export function it(specName: string, fn: () => void | Promise<void>): void;
    export function expect<T = unknown>(actual: T): SliceMatchers<T>;
}

declare module "slice:jsx" {
    export type VNode = {
        type: string | ((props: Record<string, unknown>) => unknown);
        props: Record<string, unknown>;
    };
    export const Fragment: unique symbol;
    export function jsx(type: VNode["type"] | symbol, props: Record<string, unknown>, key?: string | number): VNode;
    export function jsxs(type: VNode["type"] | symbol, props: Record<string, unknown>, key?: string | number): VNode;
    export function renderToString(node: unknown): string;
    export function escapeHtml(raw: string): string;
    export function raw(trustedHtml: string): { __slice_raw_html: string };
}

declare module "slice:jsx/jsx-runtime" {
    export { Fragment, jsx, jsxs, renderToString, escapeHtml, raw } from "slice:jsx";
    export namespace JSX {
        type Element = unknown;
        interface IntrinsicElements {
            [elemName: string]: Record<string, unknown>;
        }
    }
}

declare module "cloudflare:sockets" {
    export function connect(
        address: string | { hostname: string; port: number },
        options?: Slice.TcpSocketOptions
    ): Slice.TcpSocket;
}

declare module "*.wasm" {
    const wasmBytes: Uint8Array;
    export default wasmBytes;
}

declare interface URLPatternComponentResult {
    input: string;
    groups: Record<string, string | undefined>;
}

declare interface URLPatternResult {
    inputs: unknown[];
    protocol: URLPatternComponentResult;
    username: URLPatternComponentResult;
    password: URLPatternComponentResult;
    hostname: URLPatternComponentResult;
    port: URLPatternComponentResult;
    pathname: URLPatternComponentResult;
    search: URLPatternComponentResult;
    hash: URLPatternComponentResult;
}

declare interface URLPatternInit {
    protocol?: string;
    username?: string;
    password?: string;
    hostname?: string;
    port?: string;
    pathname?: string;
    search?: string;
    hash?: string;
    baseURL?: string;
}

declare class URLPattern {
    readonly protocol: string;
    readonly username: string;
    readonly password: string;
    readonly hostname: string;
    readonly port: string;
    readonly pathname: string;
    readonly search: string;
    readonly hash: string;
    readonly hasRegExpGroups: boolean;
    constructor(input?: string | URLPatternInit, baseURL?: string | { ignoreCase?: boolean }, options?: { ignoreCase?: boolean });
    test(input?: string | URLPatternInit, baseURL?: string): boolean;
    exec(input?: string | URLPatternInit, baseURL?: string): URLPatternResult | null;
}

declare class CompressionStream {
    readonly readable: ReadableStream<Uint8Array>;
    readonly writable: WritableStream<Uint8Array>;
    constructor(format: "gzip" | "deflate" | "deflate-raw");
}

declare class DecompressionStream {
    readonly readable: ReadableStream<Uint8Array>;
    readonly writable: WritableStream<Uint8Array>;
    constructor(format: "gzip" | "deflate" | "deflate-raw");
}

declare module "node:async_hooks" {
    export class AsyncLocalStorage<T = unknown> {
        run<R>(store: T, callback: (...args: any[]) => R, ...args: any[]): R;
        exit<R>(callback: (...args: any[]) => R, ...args: any[]): R;
        getStore(): T | undefined;
        enterWith(store: T): void;
        disable(): void;
        static bind<F extends (...args: any[]) => any>(fn: F): F;
        static snapshot(): <R>(fn: (...args: any[]) => R, ...args: any[]) => R;
    }
    export class AsyncResource {
        constructor(type: string, opts?: { triggerAsyncId?: number });
        runInAsyncScope<T, R>(fn: (this: T, ...args: any[]) => R, thisArg?: T, ...args: any[]): R;
        bind<F extends (...args: any[]) => any>(fn: F, thisArg?: any): F;
        static bind<F extends (...args: any[]) => any>(fn: F, type?: string, thisArg?: any): F;
        asyncId(): number;
        triggerAsyncId(): number;
        emitDestroy(): this;
    }
}
"#;

const DEFAULT_TSCONFIG_JSON: &str = r#"{
  "compilerOptions": {
    "target": "ES2024",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "noEmit": true,
    "strict": true,
    "skipLibCheck": true,
    "jsx": "react-jsx",
    "jsxImportSource": "slice:jsx",
    "types": []
  },
  "include": ["src/**/*", "tests/**/*", "slice.d.ts"]
}
"#;

pub(crate) fn cmd_init(args: &[String]) -> Result<()> {
    let target_dir = if args.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(&args[0])
    };

    std::fs::create_dir_all(target_dir.join("src"))?;
    let project_name = target_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("slice-edge-app");

    let manifest_path = target_dir.join("slice.toml");
    if !manifest_path.exists() {
        std::fs::write(
            &manifest_path,
            SliceManifest::default_template(project_name),
        )?;
    }

    let dts_path = target_dir.join("slice.d.ts");
    if !dts_path.exists() {
        std::fs::write(&dts_path, DEFAULT_SLICE_D_TS)?;
    }

    let tsconfig_path = target_dir.join("tsconfig.json");
    if !tsconfig_path.exists() {
        std::fs::write(&tsconfig_path, DEFAULT_TSCONFIG_JSON)?;
    }

    let entry_path = target_dir.join("src").join("index.ts");
    if !entry_path.exists() {
        let sample_ts = r#"// Slice (Nexus-JS) Edge MicroVM Entrypoint
interface TenantContext {
    service: string;
    version: number;
}

function fib(n: number): number {
    if (n <= 1) return n;
    return fib(n - 1) + fib(n - 2);
}

const stage: string = env("SERVICE_STAGE");
const f: number = fib(30);
const route_response: string = WinterTC.route("GET", "https://edge.slice.local/api/v1/status");
console.log(stage);
console.log(f);
console.log(route_response);
"#;
        std::fs::write(&entry_path, sample_ts)?;
    }

    println!(
        "Initialized Slice (Nexus-JS) project '{}' in {}",
        project_name,
        target_dir.display()
    );
    println!("  Created {}", manifest_path.display());
    println!("  Created {}", dts_path.display());
    println!("  Created {}", tsconfig_path.display());
    println!("  Created {}", entry_path.display());
    println!("Next steps:");
    println!("  slice run                        (Execute with slice.toml OCap policy)");
    println!("  slice compile ./src/index.ts -o app  (Build standalone zero-dependency binary)");
    println!("  slice serve --port=3000          (Start multi-tenant HTTP MicroVM server)");
    Ok(())
}

pub(crate) fn cmd_run(args: &[String]) -> Result<()> {
    let mut exec_cfg = ResolvedExecutionConfig::from_cli_args(args)?;
    let mut quiet = false;
    let mut print_metrics = false;
    let mut watch_mode = false;
    let mut watch_once = false;
    let mut target_opt: Option<String> = None;

    for arg in args {
        if arg == "--quiet" || arg == "-q" {
            quiet = true;
        } else if arg == "--metrics" {
            print_metrics = true;
        } else if arg == "--watch" {
            watch_mode = true;
        } else if arg == "--watch-once" {
            watch_mode = true;
            watch_once = true;
        } else if !arg.starts_with('-') && target_opt.is_none() {
            target_opt = Some(arg.clone());
        }
    }

    let target = match target_opt {
        Some(t) => t,
        None => {
            let manifest_path = Path::new("slice.toml");
            if manifest_path.exists() {
                let raw = std::fs::read_to_string(manifest_path)?;
                let m = SliceManifest::parse_toml(&raw);
                m.entrypoint
            } else {
                bail!("Missing target script/archive path (and no slice.toml found in current directory)");
            }
        }
    };

    let path = Path::new(&target);
    let mut bundled_for_sourcemap = String::new();
    let metrics = if target.ends_with(".cas") {
        let archive = MappedCasArchive::open(path)?;
        if let Some(toml_str) = archive.get_manifest_toml() {
            exec_cfg.apply_manifest_toml(toml_str, path.parent());
        }
        let prewarm_pool = exec_cfg.warm_pool;
        let deterministic_seed = exec_cfg.deterministic_seed;
        let entry = archive
            .entrypoint_blob()
            .context("CAS archive has no valid entrypoint blob")?;
        let bytecode = archive.get_bytecode_slice(entry);
        let mut isolate = SliceIsolate::spawn(
            "tenant-cli",
            1,
            exec_cfg.caps.clone(),
            exec_cfg.fuel_limit,
            exec_cfg.mem_limit,
        );
        isolate.set_deterministic_seed(deterministic_seed);
        for sec in &exec_cfg.audit_secrets {
            isolate.add_egress_audit_secret(sec.clone());
        }
        if prewarm_pool {
            SliceIsolate::prewarm_boa_bundle(bytecode);
            let snap = isolate.checkpoint();
            let _ = isolate.execute_bytecode(bytecode, Some(&archive));
            isolate.restore(snap);
            isolate.reset_for_request("tenant-cli", 1, exec_cfg.fuel_limit, exec_cfg.mem_limit);
        }
        let m = isolate.execute_bytecode(bytecode, Some(&archive))?;
        if m.return_value.starts_with("Uncaught ") {
            bundled_for_sourcemap = archive.reconstruct_annotated_source(entry);
        }
        m
    } else {
        exec_cfg.apply_target_path_manifest(path);
        let entry_file = slice_cas::resolve_project_entrypoint(path, None)?;
        if let Ok(raw_entry) = std::fs::read_to_string(&entry_file) {
            exec_cfg.apply_inline_directives(&raw_entry, entry_file.parent());
        }
        let prewarm_pool = exec_cfg.warm_pool;
        let deterministic_seed = exec_cfg.deterministic_seed;
        let bundled_source = slice_cas::bundle_entrypoint_with_deps(&entry_file)?;
        if !prewarm_pool && exec_cfg.cli_boacas_override.is_none() {
            slice_engine::set_thread_exper_boacas_override(Some(false));
        }
        let bytecode = compile_source_to_bytecode(&bundled_source);
        if !prewarm_pool && exec_cfg.cli_boacas_override.is_none() {
            slice_engine::set_thread_exper_boacas_override(None);
        }
        bundled_for_sourcemap = bundled_source;
        let mut isolate = SliceIsolate::spawn(
            "tenant-cli",
            1,
            exec_cfg.caps.clone(),
            exec_cfg.fuel_limit,
            exec_cfg.mem_limit,
        );
        isolate.set_deterministic_seed(deterministic_seed);
        for sec in &exec_cfg.audit_secrets {
            isolate.add_egress_audit_secret(sec.clone());
        }
        if prewarm_pool {
            SliceIsolate::prewarm_boa_bundle(&bytecode);
            let snap = isolate.checkpoint();
            let _ = isolate.execute_bytecode(&bytecode, None);
            isolate.restore(snap);
            isolate.reset_for_request("tenant-cli", 1, exec_cfg.fuel_limit, exec_cfg.mem_limit);
        }
        isolate.execute_bytecode(&bytecode, None)?
    };

    if metrics.return_value.starts_with("Uncaught ") {
        let mapped =
            slice_cas::map_bundled_error_location(&bundled_for_sourcemap, &metrics.return_value);
        bail!("{}", mapped);
    }

    if !quiet {
        for line in &metrics.stdout {
            println!("{}", line);
        }
    }

    if print_metrics {
        println!(
            "[Slice Telemetry] Tier: {:?} | Instantiate: {:.2} us | Exec: {:.2} us | Total: {:.2} us | Memory: {} B | Stat Syscalls: {}",
            metrics.highest_tier_reached,
            metrics.instantiate_time_ns as f64 / 1_000.0,
            metrics.execution_time_ns as f64 / 1_000.0,
            metrics.total_time_ns as f64 / 1_000.0,
            metrics.memory_used_bytes,
            metrics.os_stat_syscalls
        );
    }

    if watch_mode && !target.ends_with(".cas") {
        if !watch_once {
            install_ctrlc_handler();
        }
        let mut watcher = OsNativeFileWatcher::new(path);
        while watcher.wait_for_change(watch_once) {
            let reload_start = Instant::now();
            let entry_file = slice_cas::resolve_project_entrypoint(path, None)?;
            let bundled_source = slice_cas::bundle_entrypoint_with_deps(&entry_file)?;
            let bytecode = compile_source_to_bytecode(&bundled_source);
            let mut isolate = SliceIsolate::spawn(
                "tenant-cli-watch",
                1,
                exec_cfg.caps.clone(),
                exec_cfg.fuel_limit,
                exec_cfg.mem_limit,
            );
            isolate.set_deterministic_seed(exec_cfg.deterministic_seed);
            for sec in &exec_cfg.audit_secrets {
                isolate.add_egress_audit_secret(sec.clone());
            }
            let reload_metrics = isolate.execute_bytecode(&bytecode, None)?;
            let reload_ms = reload_start.elapsed().as_secs_f64() * 1_000.0;
            if !quiet {
                for line in &reload_metrics.stdout {
                    println!("{}", line);
                }
                println!("[Slice Watch] Reloaded in {:.2} ms", reload_ms);
            }
            if watch_once {
                break;
            }
        }
    }

    Ok(())
}

fn cmd_pack(args: &[String]) -> Result<()> {
    if args.len() < 2 {
        bail!("Usage: slice pack <input_file_or_dir> <output.cas> [--entry=./index.ts] [--static=./public]");
    }
    let input_path = Path::new(&args[0]);
    let output_path = Path::new(&args[1]);
    let mut entrypoint = String::from("./index.ts");
    let mut static_dir_opt: Option<PathBuf> = None;

    let mut exper_override: Option<bool> = None;

    for arg in &args[2..] {
        if let Some(e) = arg.strip_prefix("--entry=") {
            entrypoint = e.to_string();
        } else if let Some(sd) = arg.strip_prefix("--static=") {
            static_dir_opt = Some(PathBuf::from(sd));
        } else if arg == "--boacas" || arg == "--exper-boacas" {
            exper_override = Some(true);
        } else if arg == "--no-boacas" || arg == "--no-exper-boacas" {
            exper_override = Some(false);
        } else if let Some(b_str) = arg
            .strip_prefix("--boacas=")
            .or_else(|| arg.strip_prefix("--exper-boacas="))
        {
            exper_override = slice_sandbox::parse_bool_setting(b_str);
        } else if let Some(t_str) = arg.strip_prefix("--tier=") {
            if let Some(t) = slice_sandbox::parse_tier_setting(t_str) {
                slice_engine::set_tier_override(t);
            }
        }
    }
    if let Some(ov) = exper_override {
        slice_engine::set_exper_boacas_override(Some(ov));
    }

    let start = Instant::now();
    let header = build_cas_archive_with_static(
        input_path,
        output_path,
        &entrypoint,
        static_dir_opt.as_deref(),
    )?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1_000.0;
    let file_size = std::fs::metadata(output_path)?.len();

    println!(
        "Packed {} unique BLAKE3 module blobs into {} ({} bytes) in {:.2} ms",
        header.blobs.len(),
        output_path.display(),
        file_size,
        elapsed_ms
    );
    Ok(())
}

fn cmd_compile(args: &[String]) -> Result<()> {
    if args.is_empty() {
        bail!("Usage: slice compile <input_script_or_dir_or_cas> [-o <output_binary>] [--entry=./index.ts] [--static=./public]");
    }
    let input_path = Path::new(&args[0]);
    let mut output_bin: Option<PathBuf> = None;
    let mut entrypoint = String::from("./index.ts");
    let mut static_dir_opt: Option<PathBuf> = None;

    let mut exper_override: Option<bool> = None;

    let mut i = 1;
    while i < args.len() {
        if args[i] == "-o" || args[i] == "--output" {
            if i + 1 < args.len() {
                output_bin = Some(PathBuf::from(&args[i + 1]));
                i += 2;
                continue;
            }
        } else if let Some(out) = args[i].strip_prefix("-o=") {
            output_bin = Some(PathBuf::from(out));
        } else if let Some(e) = args[i].strip_prefix("--entry=") {
            entrypoint = e.to_string();
        } else if let Some(sd) = args[i].strip_prefix("--static=") {
            static_dir_opt = Some(PathBuf::from(sd));
        } else if args[i] == "--boacas" || args[i] == "--exper-boacas" {
            exper_override = Some(true);
        } else if args[i] == "--no-boacas" || args[i] == "--no-exper-boacas" {
            exper_override = Some(false);
        } else if let Some(b_str) = args[i]
            .strip_prefix("--boacas=")
            .or_else(|| args[i].strip_prefix("--exper-boacas="))
        {
            exper_override = slice_sandbox::parse_bool_setting(b_str);
        } else if let Some(t_str) = args[i].strip_prefix("--tier=") {
            if let Some(t) = slice_sandbox::parse_tier_setting(t_str) {
                slice_engine::set_tier_override(t);
            }
        }
        i += 1;
    }
    if let Some(ov) = exper_override {
        slice_engine::set_exper_boacas_override(Some(ov));
    }

    let default_name = input_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("slice_app");
    let mut out_path = output_bin.unwrap_or_else(|| PathBuf::from(default_name));
    if cfg!(windows) && out_path.extension().is_none() {
        out_path.set_extension("exe");
    }

    let start = Instant::now();
    let temp_cas = env::temp_dir().join("slice_compile_staging.cas");
    let cas_to_embed = if input_path.extension().and_then(|e| e.to_str()) == Some("cas") {
        input_path.to_path_buf()
    } else {
        build_cas_archive_with_static(
            input_path,
            &temp_cas,
            &entrypoint,
            static_dir_opt.as_deref(),
        )?;
        temp_cas.clone()
    };

    let current_exe = env::current_exe()?;
    let total_bytes = embed_cas_into_executable(&current_exe, &cas_to_embed, &out_path)?;
    if temp_cas.exists() {
        let _ = std::fs::remove_file(&temp_cas);
    }

    println!(
        "Compiled standalone MicroVM executable {} ({:.2} MB) in {:.2} ms",
        out_path.display(),
        total_bytes as f64 / (1024.0 * 1024.0),
        start.elapsed().as_secs_f64() * 1_000.0
    );
    Ok(())
}

fn cmd_inspect(args: &[String]) -> Result<()> {
    if args.is_empty() {
        bail!("Usage: slice inspect <archive.cas | script.ts>");
    }
    let path = Path::new(&args[0]);
    if path.extension().and_then(|e| e.to_str()) == Some("cas") {
        let archive = MappedCasArchive::open(path)?;
        println!("=========================================================================");
        println!(
            "  SLICE_CAS v{} ARCHIVE INSPECTOR: {}",
            archive.header.version,
            path.display()
        );
        println!("=========================================================================");
        println!(
            "  Entrypoint Specifier : {}",
            archive.header.entrypoint_specifier
        );
        println!(
            "  Entrypoint BLAKE3    : {}",
            archive.header.entrypoint_hash
        );
        println!("  Unique Module Blobs  : {}", archive.header.blobs.len());
        println!(
            "  Radix Index Entries  : {}",
            archive.header.radix_resolution_map.len()
        );
        println!("-------------------------------------------------------------------------");
        let mut blobs: Vec<_> = archive.header.blobs.values().collect();
        blobs.sort_by_key(|b| b.offset);
        for blob in blobs.iter().take(15) {
            println!(
                "  [Blob {}..{}] Specifier: {} | SrcOffset: {} ({} B) | BytecodeOffset: {} ({} B)",
                &blob.hash_hex[..12],
                &blob.hash_hex[56..],
                blob.original_specifier,
                blob.offset,
                blob.source_len,
                blob.bytecode_offset,
                blob.bytecode_len
            );
            let bc = archive.get_bytecode_slice(blob);
            if let Ok(disasm) = disassemble_bytecode(bc) {
                for line in disasm {
                    println!("      {}", line);
                }
            }
        }
        if blobs.len() > 15 {
            println!("  ... ({} additional blobs omitted)", blobs.len() - 15);
        }
        println!("=========================================================================");
    } else {
        let entry_file = slice_cas::resolve_project_entrypoint(path, None)?;
        let bundled = slice_cas::bundle_entrypoint_with_deps(&entry_file)?;
        let bc = compile_source_to_bytecode(&bundled);
        let disasm = disassemble_bytecode(&bc)?;
        println!("=========================================================================");
        println!(
            "  SLICESNP BYTECODE DISASSEMBLY: {} ({} bytes)",
            path.display(),
            bc.len()
        );
        println!("=========================================================================");
        for line in disasm {
            println!("  {}", line);
        }
        println!("=========================================================================");
    }
    Ok(())
}
