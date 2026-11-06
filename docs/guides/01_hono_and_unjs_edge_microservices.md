# Guide 01: Building & Deploying Hono (`v4`) and UnJS Edge Microservices on Slice

Because Slice implements the WinterTC Minimum Common Web Platform API (`Request`, `Response`, `Headers`, `URL`, `URLSearchParams`, `ReadableStream`, `crypto.subtle`, `TextEncoder`/`TextDecoder`, and `export default { fetch }`) and compiles bundles ahead-of-time into **`SLICEBC\x01` Precompiled Boa `CodeBlock` CAS** containers with Oxc AST deferred function module spans, WinterTC frameworks like **Hono (`v4`)**, **`itty-router` (`v5`)**, and the **UnJS / Nitro ecosystem (`ufo`, `destr`, `ohash`, `radix3`, `cookie`)** run unmodified on Slice at **`13.11 ms - 15.52 ms` cold** and **`0.55 ms - 0.56 ms` warm** (`2.54x - 2.65x` faster cold than Bun, `59x - 75x` faster warm than Bun, and `7.36x - 7.39x` faster cold than Node.js).

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

See [`tests/frameworks/01-hono-edge-microservice`](file:///p:/opensource/slice/tests/frameworks/01-hono-edge-microservice/src/index.ts) and [`tests/frameworks/02-itty-router-unjs-nitro-stack`](file:///p:/opensource/slice/tests/frameworks/02-itty-router-unjs-nitro-stack/src/index.ts) for complete, benchmarked reference implementations verified across Slice, Bun, Deno, and Node.js.

