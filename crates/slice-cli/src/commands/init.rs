use anyhow::Result;
use slice_sandbox::SliceManifest;
use std::path::PathBuf;

pub(crate) const DEFAULT_SLICE_D_TS: &str = r#"// Slice (Nexus-JS) Ambient TypeScript Declarations (`slice.d.ts`)
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
        tenantId?: string;
        fuel?: number;
        memoryBytes?: number;
        allowRead?: string[];
        allowWrite?: string[];
        allowNet?: string[];
        allowExec?: string[];
        allowEnv?: string[];
        deterministicSeed?: number;
        auditSecret?: string;
        auditSecrets?: string[];
        checkpoints?: boolean;
        maxCheckpoints?: number;
    }

    interface SandboxEvalOutcome {
        ok: boolean;
        returnValue: string;
        returnJson: string | null;
        stdout: string[];
        fuelUsed: number;
        memoryUsedBytes: number;
        peakMemoryBytes: number;
        spawnTimeNs: number;
        compileTimeNs: number;
        execTimeNs: number;
        totalTimeUs: number;
        checkpointsTaken: number;
        restoresPerformed: number;
        restoreTimeNs: number;
        violation: string | null;
        error: string | null;
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
    function releaseCheckpoint(snapshotId: number): boolean;
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

pub(crate) const DEFAULT_TSCONFIG_JSON: &str = r#"{
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
