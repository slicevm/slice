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

pub(crate) const SVELTEKIT_ADAPTER_SLICE_JS: &str = r#"import fs from 'node:fs';
import path from 'node:path';

/**
 * @sveltejs/kit Adapter for Slice Runtime (`@slicevm/adapter-sveltekit`)
 * Supports SvelteKit 3 (`generateServerInstance`) and SvelteKit 2 (`generateManifest`).
 * Emits:
 *   - `build/client/` (static assets & immutable client chunks for `--static`)
 *   - `build/server/` (SvelteKit SSR server chunks + server instance)
 *   - `build/slice-entry.js` (WinterTC `export default { fetch }` entrypoint)
 */
export default function adapterSlice(options = {}) {
    const out = options.out || 'build';
    return {
        name: '@slicevm/adapter-sveltekit',
        async adapt(builder) {
            const tmp = builder.getBuildDirectory('adapter-slice');
            builder.rimraf(out);
            builder.rimraf(tmp);
            builder.mkdirp(tmp);
            builder.mkdirp(`${out}/client`);
            builder.mkdirp(`${out}/server`);

            builder.writeClient(`${out}/client`);
            builder.writePrerendered(`${out}/client`);
            builder.copy(builder.getServerDirectory(), `${out}/server`);

            let entryHeader;
            if (typeof builder.generateServerInstance === 'function') {
                builder.generateServerInstance(`${out}/server/instance.js`, {
                    serverDirectory: `${out}/server`
                });
                entryHeader = `import { server } from './server/instance.js';\n`;
            } else {
                const manifestStr = builder.generateManifest({ relativePath: '.' });
                fs.writeFileSync(
                    `${out}/server/manifest.js`,
                    `export const manifest = ${manifestStr};\n`
                );
                entryHeader = `import { Server } from './server/index.js';\nimport { manifest } from './server/manifest.js';\nconst server = new Server(manifest);\n`;
            }

            const entryCode = `${entryHeader}
let initPromise = null;

export default {
    async fetch(request, env, ctx) {
        if (!initPromise) {
            initPromise = server.init({ env: env || (typeof process !== 'undefined' && process.env) || {} });
        }
        await initPromise;
        return await server.respond(request, {
            getClientAddress() {
                return request.headers.get('x-forwarded-for') || '127.0.0.1';
            },
            platform: {
                env,
                ctx,
                Slice: typeof Slice !== 'undefined' ? Slice : undefined
            }
        });
    }
};
`;
            fs.writeFileSync(path.join(out, 'slice-entry.js'), entryCode);
            builder.log.minor(`Wrote Slice WinterTC entrypoint to ${out}/slice-entry.js and client assets to ${out}/client`);
        }
    };
}
"#;

fn init_sveltekit_template(target_dir: &std::path::Path, project_name: &str) -> Result<()> {
    std::fs::create_dir_all(
        target_dir
            .join("src")
            .join("routes")
            .join("api")
            .join("items"),
    )?;

    let manifest_path = target_dir.join("slice.toml");
    if !manifest_path.exists() {
        let toml_content = format!(
            r#"[package]
name = "{project_name}"
version = "0.1.0"
entrypoint = "./build/slice-entry.js"

[server]
static_dir = "./build/client"

[limits]
fuel = 50_000_000
memory_mb = 64

[capabilities]
allow_env = ["NODE_ENV", "ORIGIN", "PORT"]
"#
        );
        std::fs::write(&manifest_path, toml_content)?;
    }

    let dts_path = target_dir.join("slice.d.ts");
    if !dts_path.exists() {
        std::fs::write(&dts_path, DEFAULT_SLICE_D_TS)?;
    }

    let pkg_path = target_dir.join("package.json");
    if !pkg_path.exists() {
        let pkg_json = format!(
            r#"{{
  "name": "{project_name}",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {{
    "dev": "vite dev",
    "build": "vite build",
    "pack": "slice pack ./build/slice-entry.js ./build/app.cas --static=./build/client",
    "serve": "slice serve ./build/app.cas"
  }},
  "devDependencies": {{
    "@sveltejs/kit": "^3.1.1",
    "@sveltejs/vite-plugin-svelte": "^7.3.1",
    "svelte": "^5.57.3",
    "vite": "^8.3.7"
  }}
}}
"#
        );
        std::fs::write(&pkg_path, pkg_json)?;
    }

    let adapter_path = target_dir.join("adapter-slice.js");
    if !adapter_path.exists() {
        std::fs::write(&adapter_path, SVELTEKIT_ADAPTER_SLICE_JS)?;
    }

    let vite_cfg_path = target_dir.join("vite.config.js");
    if !vite_cfg_path.exists() {
        let vite_cfg = r#"import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
import adapterSlice from './adapter-slice.js';

export default defineConfig({
    plugins: [
        sveltekit({
            adapter: adapterSlice({ out: 'build' })
        })
    ]
});
"#;
        std::fs::write(&vite_cfg_path, vite_cfg)?;
    }

    let app_html_path = target_dir.join("src").join("app.html");
    if !app_html_path.exists() {
        let app_html = r#"<!doctype html>
<html lang="en">
    <head>
        <meta charset="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>SvelteKit on Slice Runtime</title>
        %sveltekit.head%
    </head>
    <body data-sveltekit-preload-data="hover">
        <div style="display: contents">%sveltekit.body%</div>
    </body>
</html>
"#;
        std::fs::write(&app_html_path, app_html)?;
    }

    let hooks_path = target_dir.join("src").join("hooks.server.js");
    if !hooks_path.exists() {
        let hooks_js = r#"/** @type {import('@sveltejs/kit').Handle} */
export async function handle({ event, resolve }) {
    event.locals.requestId = 'req-slice-edge';
    const response = await resolve(event);
    response.headers.set('x-powered-by', 'SvelteKit-on-Slice');
    return response;
}

/** @type {import('@sveltejs/kit').HandleServerError} */
export function handleError({ status, error }) {
    if (status !== 404 && error?.status !== 404) {
        console.error(error);
    }
}
"#;
        std::fs::write(&hooks_path, hooks_js)?;
    }

    let page_srv_path = target_dir
        .join("src")
        .join("routes")
        .join("+page.server.js");
    if !page_srv_path.exists() {
        let page_srv = r#"/** @type {import('./$types').PageServerLoad} */
export async function load({ url, cookies, locals }) {
    const visitor = url.searchParams.get('name') || 'Edge Developer';
    const theme = cookies.get('theme') || 'light';
    const hits = typeof Slice !== 'undefined' && Slice.kv
        ? Slice.kv.incr('sveltekit_page_views', 1)
        : 1;

    return {
        title: 'SvelteKit 3 + Svelte 5 SSR on Slice',
        visitor,
        theme,
        requestId: locals.requestId || 'none',
        hits,
        runtime: 'Slice WinterTC Isolate'
    };
}

/** @type {import('./$types').Actions} */
export const actions = {
    setTheme: async ({ request, cookies }) => {
        const form = await request.formData();
        const theme = String(form.get('theme') || 'dark');
        cookies.set('theme', theme, { path: '/', httpOnly: true });
        return { success: true, theme };
    }
};
"#;
        std::fs::write(&page_srv_path, page_srv)?;
    }

    let page_svelte_path = target_dir.join("src").join("routes").join("+page.svelte");
    if !page_svelte_path.exists() {
        let page_svelte = r#"<script>
    let { data, form } = $props();
</script>

<main data-theme={data.theme}>
    <h1>{data.title}</h1>
    <p class="greeting">Hello, <strong>{data.visitor}</strong>!</p>
    <p class="stats">Runtime: <code>{data.runtime}</code> | Request ID: <code>{data.requestId}</code> | Theme: <code>{data.theme}</code> | KV Page Views: <span id="hits">{data.hits}</span></p>
    {#if form?.success}
        <p class="action-result">Updated theme to: <strong>{form.theme}</strong></p>
    {/if}
</main>
"#;
        std::fs::write(&page_svelte_path, page_svelte)?;
    }

    let api_srv_path = target_dir
        .join("src")
        .join("routes")
        .join("api")
        .join("items")
        .join("+server.js");
    if !api_srv_path.exists() {
        let api_srv = r#"import { json } from '@sveltejs/kit';

/** @type {import('./$types').RequestHandler} */
export async function GET({ url }) {
    const tag = url.searchParams.get('tag') || 'all';
    const count = typeof Slice !== 'undefined' && Slice.kv
        ? Slice.kv.incr('sveltekit_api_calls', 1)
        : 1;

    return json({
        ok: true,
        framework: 'SvelteKit',
        tag,
        count,
        items: ['slice-cas', 'slice-engine', 'slice-sandbox']
    });
}

/** @type {import('./$types').RequestHandler} */
export async function POST({ request }) {
    const body = await request.json();
    return json({
        created: true,
        received: body
    }, { status: 201 });
}
"#;
        std::fs::write(&api_srv_path, api_srv)?;
    }

    println!(
        "Initialized Slice SvelteKit (SvelteKit 3 + Svelte 5) project '{}' in {}",
        project_name,
        target_dir.display()
    );
    println!("  Created {}", manifest_path.display());
    println!("  Created {}", pkg_path.display());
    println!("  Created {}", adapter_path.display());
    println!("  Created {}", vite_cfg_path.display());
    println!("  Created {}", page_svelte_path.display());
    println!("  Created {}", api_srv_path.display());
    println!("Next steps:");
    println!("  npm install && npm run build");
    println!("  slice pack ./build/slice-entry.js ./build/app.cas --static=./build/client");
    println!("  slice serve ./build/app.cas --port=3000");
    Ok(())
}

pub(crate) fn cmd_init(args: &[String]) -> Result<()> {
    let mut target_dir = PathBuf::from(".");
    let mut template = String::from("default");
    let mut dir_set = false;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if let Some(t) = arg.strip_prefix("--template=") {
            template = t.to_ascii_lowercase();
        } else if (arg == "--template" || arg == "-t") && i + 1 < args.len() {
            template = args[i + 1].to_ascii_lowercase();
            i += 1;
        } else if !arg.starts_with('-') && !dir_set {
            target_dir = PathBuf::from(arg);
            dir_set = true;
        }
        i += 1;
    }

    std::fs::create_dir_all(target_dir.join("src"))?;
    let project_name = target_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("slice-edge-app");

    if template == "sveltekit" || template == "svelte" {
        return init_sveltekit_template(&target_dir, project_name);
    }

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
