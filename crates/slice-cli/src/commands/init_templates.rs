//! Static project scaffolding templates for `slice init` (Default WinterTC TS + SvelteKit 3).

pub(crate) const DEFAULT_SLICE_D_TS: &str = r#"// Slice (Nexus-JS) Ambient TypeScript Declarations (`slice.d.ts`)
// Automatically included by `tsconfig.json` for IDE IntelliSense and `tsc --noEmit`.

declare var process: {
    env: Record<string, string | undefined>;
    argv: string[];
    exitCode?: number;
    readonly platform: string;
    readonly arch: string;
    cwd(): string;
    exit(code?: number): never;
};

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
        virtualMounts?: string[];
        allowNet?: string[];
        allowPrivateNetworks?: boolean;
        allowExec?: string[];
        allowEnv?: string[];
        deterministicSeed?: number;
        auditSecret?: string;
        auditSecrets?: string[];
        checkpoints?: boolean;
        maxCheckpoints?: number;
        timeoutMs?: number;
    }

    interface SandboxEvalOutcome {
        ok: boolean;
        exitCode: number | null;
        processExited: boolean;
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
        deniedCapabilities: string[];
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

pub(crate) const DEFAULT_SAMPLE_INDEX_TS: &str = r#"// Slice (Nexus-JS) WinterTC Edge MicroVM Entrypoint
export interface ServiceStatus {
    status: string;
    stage: string;
    runtime: string;
    method: string;
    url: string;
}

export function buildStatus(method: string, url: string): ServiceStatus {
    const stage = process.env.SERVICE_STAGE ?? "production";
    const runtime = process.env.RUNTIME_ENGINE ?? "Slice-NexusJS";
    return {
        status: "ok",
        stage,
        runtime,
        method,
        url,
    };
}

export function handleRequest(request: Request): Response {
    return Response.json(buildStatus(request.method, request.url));
}

export default {
    fetch: handleRequest,
};

console.log(JSON.stringify(buildStatus("CLI", "https://edge.slice.local/api/v1/status")));
"#;

pub(crate) const DEFAULT_SAMPLE_TEST_TS: &str = r#"import { describe, test, expect } from "slice:test";
import { handleRequest } from "../src/index.ts";

describe("WinterTC Edge Handler", () => {
    test("returns 200 JSON service status", async () => {
        const req = new Request("https://edge.slice.local/api/v1/status", { method: "GET" });
        const res = handleRequest(req);
        expect(res.status).toBe(200);
        const body = await res.json();
        expect(body.status).toBe("ok");
        expect(body.method).toBe("GET");
    });
});
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

pub(crate) const SVELTEKIT_VITE_CONFIG_JS: &str = r#"import { sveltekit } from '@sveltejs/kit/vite';
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

pub(crate) const SVELTEKIT_APP_HTML: &str = r#"<!doctype html>
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

pub(crate) const SVELTEKIT_HOOKS_SERVER_JS: &str = r#"/** @type {import('@sveltejs/kit').Handle} */
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

pub(crate) const SVELTEKIT_PAGE_SERVER_JS: &str = r#"/** @type {import('./$types').PageServerLoad} */
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

pub(crate) const SVELTEKIT_PAGE_SVELTE: &str = r#"<script>
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

pub(crate) const SVELTEKIT_API_ITEMS_SERVER_JS: &str = r#"import { json } from '@sveltejs/kit';

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

pub(crate) fn format_sveltekit_manifest_toml(project_name: &str) -> String {
    format!(
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
    )
}

pub(crate) fn format_sveltekit_package_json(project_name: &str) -> String {
    format!(
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
    )
}
