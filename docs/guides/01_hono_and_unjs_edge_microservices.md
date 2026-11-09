# Guide 01: Building & Deploying Hono (`v4`), UnJS, and SvelteKit (`v3`) Edge Microservices on Slice

Because Slice implements the WinterTC Minimum Common Web Platform API (`Request`, `Response`, `Headers`, `URL`, `URLSearchParams`, `ReadableStream`, `crypto.subtle`, `TextEncoder`/`TextDecoder`, and `export default { fetch }`) and compiles bundles ahead-of-time into **`SLICEBC\x01` Precompiled Boa `CodeBlock` CAS** containers with Oxc AST deferred function module spans and live ES module bindings, WinterTC frameworks like **Hono (`v4`)**, **`itty-router` (`v5`)**, the **UnJS / Nitro ecosystem (`ufo`, `destr`, `ohash`, `radix3`, `cookie`)**, and **SvelteKit (`@sveltejs/kit@3.1.1` + `svelte@5.57.3`)** run unmodified on Slice.

---

## 1. Building a Hono (`v4`) Edge Microservice

### Step 1: Scaffold & Install `hono`
```bash
slice init ./my-hono-service
cd ./my-hono-service
npm init -y
npm install hono ms
```

### Step 2: Author `src/index.ts`
```typescript
import { Hono } from "hono";
import { getCookie, setCookie } from "hono/cookie";
import ms from "ms";

const app = new Hono();

app.use("*", async (c, next) => {
    await next();
    c.header("x-powered-by", "Slice-Hono-Edge");
});

app.get("/api/v1/health", (c) => {
    return c.json({
        status: "healthy",
        ttlMs: ms("15m"),
        visits: Slice.kv.incr("health:hits", 1),
    });
});

app.post("/api/v1/orders/:orderId", async (c) => {
    const orderId = c.req.param("orderId");
    const session = getCookie(c, "sid") ?? "anon";
    const body = await c.req.json<{ amountCents: number }>();
    setCookie(c, "last_order", orderId, { path: "/", httpOnly: true });
    return c.json({ orderId, session, amountCents: body.amountCents }, 201);
});

export default app;
```

### Step 3: Serve, Pack, or Compile to a Single Binary
```bash
# Run with multi-worker HTTP/1.1 Keep-Alive server:
slice serve . --port=3000 --workers=4

# Or compile into a single zero-dependency native executable:
slice compile . -o ./my-hono-bin
./my-hono-bin
```

---

## 2. Building an `itty-router` + UnJS (`radix3`, `ufo`, `destr`, `ohash`) Edge Stack

See [`tests/frameworks/01-hono-edge-microservice`](../../tests/frameworks/01-hono-edge-microservice/src/index.ts) and [`tests/frameworks/02-itty-router-unjs-nitro-stack`](../../tests/frameworks/02-itty-router-unjs-nitro-stack/src/index.ts) for complete, benchmarked reference implementations verified across Slice, Bun, Deno, and Node.js.

---

## 3. Scaffolding & Deploying a Full-Stack SvelteKit (`v3.1.1`) + Svelte 5 (`v5.57.3`) SSR Application

Slice includes a built-in project scaffolder (`slice init --template=sveltekit`) and custom WinterTC adapter (`adapter-slice.js`) for full-stack SvelteKit 3 + Svelte 5 SSR applications (`+page.svelte`, `+page.server.js` `load` & `actions`, `+server.js` JSON endpoints, `hooks.server.js`, and `devalue` `__data.json` hydration):

```bash
# 1. Scaffold a SvelteKit 3 + Svelte 5 project with adapter-slice:
slice init ./my-sveltekit-app --template=sveltekit
cd ./my-sveltekit-app

# 2. Install dependencies & run the Vite 8 + SvelteKit SSR build:
npm install
npm run build

# 3. Pack the SSR server + client static assets into a single zero-stat .cas archive:
slice pack ./build/slice-entry.js ./build/app.cas --static=./build/client

# 4. Serve with multi-worker HTTP/1.1 Keep-Alive and BLAKE3 ETag static assets:
slice serve ./build/app.cas --port=3000 --workers=8
```

See [`tests/frameworks/04-sveltekit-edge-ssr`](../../tests/frameworks/04-sveltekit-edge-ssr/src/index.ts) for the full 12-request benchmark and verification suite.


