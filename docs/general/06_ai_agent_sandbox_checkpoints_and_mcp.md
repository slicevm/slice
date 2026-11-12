# Chapter 06: AI Agents, Nested MicroVM Sandboxes (`Slice.evalSandbox`), `< 5 us` Checkpoints & `slice mcp`

Modern AI coding assistants (Claude Code, Cursor, Windsurf), Tree-of-Thought (ToT) reasoning engines, and multi-tenant plugin platforms need to **execute untrusted, LLM-generated or user-submitted TypeScript code** thousands of times per minute.

Traditional approaches force an impossible choice:
- Running `eval()` or `vm.runInNewContext()` in Node.js has **zero real security** (trivial prototype escapes, infinite `while(true){}` hangs, and full filesystem access).
- Booting a Docker container or Firecracker microVM for every LLM tool call takes **`125+ milliseconds`** and **`30+ MB` of RAM**, making parallel Tree-of-Thought exploration slow and expensive.

**Slice gives you three purpose-built AI & Plugin Sandbox primitives that operate in microseconds:**
1. **`Slice.evalSandbox(code, options)`**: Spawn a nested, capability-attenuated TypeScript sub-sandbox from inside your TypeScript code in tens of microseconds (see [`tests/reports/latest.md`](../../tests/reports/latest.md)).
2. **`Slice.checkpoint()` & `Slice.restore(snapId)`**: Capture and roll back the entire in-memory filesystem (`node:fs`) and `Slice.kv` state in **`< 5 microseconds`**.
3. **`slice mcp`**: A built-in JSON-RPC 2.0 **Model Context Protocol (MCP)** server over `stdio` ready to plug directly into Claude Code, Cursor, or your own LLM orchestrator.

---

## 1. Running Untrusted Code in Nested Sub-Sandboxes (`Slice.evalSandbox`)

Inside any Slice script or `slice serve` HTTP handler, you can call `Slice.evalSandbox(code, options)` to compile (with Oxc TypeScript support!) and execute an untrusted code snippet inside a child `SliceIsolate`.

### 1.1 Complete `Slice.evalSandbox` Options & Outcome Reference

```typescript
interface SandboxEvalOptions {
    /** Optional sub-tenant suffix namespaced under `<parent>:sub:<tenantId>` */
    tenantId?: string;
    /** Maximum CPU instruction fuel budget for the child (default: 500,000) */
    fuel?: number;
    /** Maximum memory ceiling in bytes for the child (default: 4,194,304) */
    memoryBytes?: number;
    /** Allowed VFS read path prefixes (INTERSECTED with parent's allowed read prefixes!) */
    allowRead?: string[];
    /** Allowed VFS write path prefixes (INTERSECTED with parent's allowed write prefixes!) */
    allowWrite?: string[];
    /** Allowed outbound network hosts (INTERSECTED with parent's allowed network hosts!) */
    allowNet?: string[];
    /** Allowed subprocess binaries (INTERSECTED with parent's allowed exec binaries!) */
    allowExec?: string[];
    /** Allowed environment variables (INTERSECTED with parent's allowed env keys!) */
    allowEnv?: string[];
    /** Optional deterministic seed for Math.random() and Date.now() inside the child */
    deterministicSeed?: number;
    /** Optional secret string to block from leaking in child stdout or returnValue */
    auditSecret?: string;
    /** Optional list of secret strings to block from leaking in child stdout, returnValue, or outbound fetch() */
    auditSecrets?: string[];
    /** Whether guest Slice.checkpoint() and Slice.restore() calls are allowed (default: true, attenuated by parent) */
    checkpoints?: boolean;
    /** Maximum number of guest checkpoints allowed during evaluation (attenuated by parent) */
    maxCheckpoints?: number;
}

interface SandboxEvalOutcome {
    /** true if execution succeeded without any OCap, SES, Fuel, Memory, or uncaught JS error */
    ok: boolean;
    /** Captured console.log / print lines emitted by the child sandbox (preserved even on trap) */
    stdout: string[];
    /** Stringified final expression return value of the child script (supports top-level await) */
    returnValue: string;
    /** JSON serialization of the final return value when JSON-safe, otherwise `null` */
    returnJson: string | null;
    /** Exact CPU instruction fuel consumed by the child (clamped to fuel limit on trap) */
    fuelUsed: number;
    /** Memory bytes used by the child */
    memoryUsedBytes: number;
    /** Peak memory bytes used by the child */
    peakMemoryBytes: number;
    /** Isolate instantiation time in nanoseconds */
    spawnTimeNs: number;
    /** Oxc TypeScript + bytecode compilation time in nanoseconds */
    compileTimeNs: number;
    /** VM execution time in nanoseconds */
    execTimeNs: number;
    /** Total wall-clock time of the child sandbox evaluation in microseconds */
    totalTimeUs: number;
    /** Number of guest `Slice.checkpoint()` snapshots taken during evaluation */
    checkpointsTaken: number;
    /** Number of guest `Slice.restore()` rollbacks performed during evaluation */
    restoresPerformed: number;
    /** Total wall-clock time spent inside `Slice.restore()` in nanoseconds */
    restoreTimeNs: number;
    /** Human-readable sandbox violation message if an OCap/SES/Fuel/Memory trap occurred, otherwise `null` */
    violation: string | null;
    /** Uncaught JavaScript exception or SyntaxError message if `ok === false` due to a JS error, otherwise `null` */
    error: string | null;
}
```

### 1.2 How Strict Capability Attenuation Works (No Privilege Escalation)
A child sandbox spawned via `Slice.evalSandbox` can **never** escalate privileges beyond its parent isolate:
- **Filesystem Intersection:** If the parent isolate only has `--allow-read=/workspace/tenant-a` and the child requests `allowRead: ["/etc", "/workspace/tenant-a/public"]`, Slice intersects the two sets so the child can **only** read `/workspace/tenant-a/public` (and `/etc` is denied!).
- **Fuel & Memory Clamping:** The child's `fuel` and `memoryBytes` budgets are clamped to the parent's remaining fuel and memory, and all fuel consumed by the child is deducted from the parent's budget.
- **Inherited Egress DLP Secrets:** Every secret registered on the parent isolate (via `--audit-egress` or `--env-file=.env`) is automatically enforced on the child sandbox's `stdout`, `returnValue`, and outbound `fetch()`.
- **Inline Source Directive Lockdown (`// @slice-*`):** Any `// @slice-allow-read`, `// @slice-allow-write`, `// @slice-allow-net`, `// @slice-allow-env`, `// @slice-env-file`, `// @slice-fuel`, or `// @slice-memory-kb` comments embedded inside AI-generated code passed to `Slice.evalSandbox`, `slice mcp` (`slice_eval_ts`), or default CLI `slice run` (where `trust_directives = false` by default) can **never** escalate privileges or raise fuel/memory ceilings beyond what the host explicitly granted.

### 1.3 Example: Evaluating Customer Pricing Formulas Safely
```typescript
const userFormulaTs = `
    interface Cart { subtotal: number; vip: boolean; }
    const cart: Cart = { subtotal: 12000, vip: true };
    const discount: number = cart.vip ? cart.subtotal * 0.15 : 0;
    console.log("Calculated discount:", discount);
    cart.subtotal - discount
`;

const result = Slice.evalSandbox(userFormulaTs, {
    tenantId: "customer-plugin-88",
    fuel: 100_000,
    memoryBytes: 512 * 1024,
    deterministicSeed: 42,
});

if (result.ok) {
    console.log("Final Price Cents:", result.returnValue); // "10200"
    console.log("Plugin Logs:", result.stdout);            // ["Calculated discount: 1800"]
    console.log("Ran in:", result.totalTimeUs, "us");
} else {
    console.error("Plugin blocked:", result.violation);
}
```

---

## 2. Time-Travel State Checkpoints (`Slice.checkpoint()` & `Slice.restore()`)

When an AI agent or multi-step workflow modifies files (`node:fs`) or key-value state (`Slice.kv`) and then hits an error or security violation, cleaning up partial mutations is notoriously error-prone.

Slice provides `< 5 microsecond` copy-on-write state snapshots:
- **`const snapId: number = Slice.checkpoint()`**: Takes an immediate in-memory snapshot of:
  1. Every file, directory, deletion tombstone, and custom modification timestamp in the `node:fs` VFS slab (`BOA_VFS_CACHE`, `BOA_VFS_DIRS`, `BOA_VFS_TOMBSTONES`, & `BOA_VFS_MTIMES`), and
  2. Every key-value pair and counter belonging to the active tenant across all 16 shards of `Slice.kv`.
- **`const restored: boolean = Slice.restore(snapId)`**: Rewinds both `node:fs` and `Slice.kv` back to the exact state at `snapId` in under 5 microseconds (verifying tenant ownership).
- **`const released: boolean = Slice.releaseCheckpoint(snapId)`**: Explicitly frees snapshot `snapId` from the bounded per-thread checkpoint slab (capped at 64 snapshots with FIFO eviction and automatic cleanup on request/sandbox teardown).

### Recipe: Tree-of-Thought (ToT) AI Code Exploration with Instant Rollback

Suppose an LLM agent generates 3 candidate patches that mutate files and state. You can test each candidate against your invariants and roll back failed candidates in microseconds:

```typescript
import * as fs from "node:fs";

// 1. Seed initial project state in VFS and Slice.kv
fs.writeFileSync("/tmp/config.json", JSON.stringify({ version: 1, safeMode: true }));
Slice.kv.set("agent:status", "ready", 0);

// 2. Capture a < 5 us baseline checkpoint
const baselineSnap = Slice.checkpoint();

const candidates = [
    // Candidate 1: Corrupts /tmp/config.json and attempts prototype pollution
    `
        const fs = require("node:fs");
        fs.writeFileSync("/tmp/config.json", "CORRUPTED_BY_CANDIDATE_1");
        Slice.kv.set("agent:status", "compromised", 0);
        Object.prototype.polluted = true;
    `,
    // Candidate 2: Infinite loop DoS attempt
    `
        Slice.kv.set("agent:status", "hung", 0);
        while (true) {}
    `,
    // Candidate 3: Clean, valid TypeScript transformation
    `
        const fs = require("node:fs");
        const cfg = JSON.parse(fs.readFileSync("/tmp/config.json", "utf8"));
        cfg.version = 2;
        fs.writeFileSync("/tmp/config.json", JSON.stringify(cfg));
        Slice.kv.set("agent:status", "upgraded-to-v2", 0);
        cfg.version
    `,
];

for (let i = 0; i < candidates.length; i++) {
    const outcome = Slice.evalSandbox(candidates[i], {
        fuel: 50_000,
        allowRead: ["/tmp"],
        allowWrite: ["/tmp"],
    });

    if (!outcome.ok) {
        // Candidate failed or violated policy -> Rewind VFS and Slice.kv in < 5 us!
        Slice.restore(baselineSnap);
        console.log(`Candidate #${i + 1} rejected (${outcome.violation}) -> State rolled back!`);
    } else {
        console.log(`Candidate #${i + 1} succeeded in ${outcome.totalTimeUs} us!`);
    }
}

console.log("Final /tmp/config.json:", fs.readFileSync("/tmp/config.json", "utf8"));
// -> {"version":2,"safeMode":true}
console.log("Final Slice.kv status:", Slice.kv.get("agent:status"));
// -> "upgraded-to-v2"
```

---

## 3. Using Slice as an MCP Server (`slice mcp`) in Claude Code & Cursor

Slice ships with a built-in **Model Context Protocol (`2024-11-05` JSON-RPC 2.0)** server over `stdio`. Connecting `slice mcp` to Claude Code, Cursor, or any MCP client gives your AI assistant a sub-millisecond TypeScript sandbox, persistent sharded KV memory, and `< 5 us` state checkpoints.

### 3.1 Adding `slice mcp` to Claude Code or Cursor (`mcp.json` / `.cursor/mcp.json`)

```json
{
  "mcpServers": {
    "slice-sandbox": {
      "command": "slice",
      "args": [
        "mcp",
        "--allow-read=/workspace",
        "--allow-write=/tmp",
        "--allow-net=api.github.com:443",
        "--audit-egress=sk_live_my_secret_api_key"
      ]
    }
  }
}
```

### 3.2 Complete Reference of Tools Exposed by `slice mcp`

When your AI assistant connects to `slice mcp`, it discovers 4 tools via `tools/list`:

#### 1. `slice_eval_ts`
Compiles and executes TypeScript or JavaScript (including top-level `await`) inside a capability-attenuated `SliceIsolate`. Capability arguments (`allowRead`, `allowWrite`, `allowNet`, `allowExec`, `allowEnv`) accept either a string (comma-separated) or an array of strings (`string[]`).
- **Input Arguments:**
  ```json
  {
    "code": "const items: number[] = [10, 20, 30]; await Promise.resolve(items.reduce((a, b) => a + b, 0))",
    "fuel": 500000,
    "memory": 4194304,
    "allowRead": ["/workspace"],
    "allowNet": ["api.github.com:443"],
    "auditSecrets": ["sk_live_optional_extra_secret", "db_prod_secret"],
    "deterministicSeed": 42,
    "checkpoints": true,
    "maxCheckpoints": 8
  }
  ```
- **Returns:** JSON `SandboxEvalOutcome` (`ok`, `stdout`, `returnValue`, `returnJson`, `fuelUsed`, `memoryUsedBytes`, `peakMemoryBytes`, `spawnTimeNs`, `compileTimeNs`, `execTimeNs`, `totalTimeUs`, `checkpointsTaken`, `restoresPerformed`, `restoreTimeNs`, `violation`, `error`).

#### 2. `slice_kv_set`
Stores a key-value pair inside the MCP host's sharded `Slice.kv` store so the agent can persist intermediate plans or results across tool calls.
- **Input Arguments:** `{ "key": "agent:plan_step", "value": "step-2-complete" }`

#### 3. `slice_kv_get`
Reads a key from the MCP host's `Slice.kv` store.
- **Input Arguments:** `{ "key": "agent:plan_step" }`

#### 4. `slice_checkpoint`
Takes a `< 5 us` snapshot of the MCP host's VFS + KV state (`"action": "checkpoint"`) or restores a previous snapshot (`"action": "restore", "snapshot_id": 1`).
- **Input Arguments (Create Checkpoint):** `{ "action": "checkpoint" }` -> returns `{"checkpointId": 1}`
- **Input Arguments (Restore Checkpoint):** `{ "action": "restore", "snapshot_id": 1 }` -> returns `{"restored": true, "snapshotId": 1}`

### 3.3 Testing `slice mcp` Locally
Run the built-in 6-step JSON-RPC 2.0 smoke test to see the exact requests and responses:
```bash
slice mcp --smoke-test
```
