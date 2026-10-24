# Chapter 06: AI Agent "Code Mode" Sandbox, `< 5 us` Checkpoints & Native MCP Server (`slice mcp`)

Modern AI coding agents (Claude Code, Cursor, Tree-of-Thought orchestrators, and "Code Mode" MCP clients) generate and execute untrusted TypeScript snippets to explore APIs, transform data, and test multi-file patches. Traditional microVMs (Firecracker) take `125 ms` and `30+ MB` to boot per tool call.

Slice provides **three native AI Agent primitives** that run in **microseconds**:
1. **`Slice.evalSandbox(code, opts)`**: Nested sub-isolate execution with strict capability attenuation (`~27 us`).
2. **`Slice.checkpoint()` & `Slice.restore(id)`**: `< 5 us` in-memory `node:fs` VFS + `Slice.kv` state snapshots and instant rollback.
3. **`slice mcp`**: Built-in JSON-RPC 2.0 Model Context Protocol (MCP) server over `stdio` with `--audit-egress` secret exfiltration protection.

---

## 1. Dynamic Sub-Isolate Spawning (`Slice.evalSandbox(code, opts)`)

Implemented in [`SliceIsolate::eval_sandbox`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L1766-L1864) and `__slice_eval_sandbox`:

### Strict Capability Attenuation Rules
- **Filesystem Intersection:** A child sandbox's `allowRead` and `allowWrite` prefixes are **intersected** with the parent isolate's `CapabilityToken`. If the parent only has access to `/workspace` and the child requests `allowRead: ["/etc"]`, the intersection is empty and any `fs.readFileSync("/etc/shadow")` inside the child traps with `FsReadDenied`.
- **Fuel & Memory Deduction:** The child's `fuel` and `memoryBytes` limits are clamped to the parent's remaining budget (`min(parent_remaining, child_requested)`), and all fuel consumed by the child is deducted from the parent's fuel counter.
- **Re-Entrant Thread-Local State:** `__slice_eval_sandbox` saves the parent isolate's thread-local state (`BOA_STDOUT`, `BOA_VIOLATION`, `BOA_ACTIVE_TENANT`, `BOA_ACTIVE_CAPS`, `BOA_KV_MEM_BUDGET`, `BOA_DET_STATE`) before running the child isolate and restores it immediately after, making nested sandboxes 100% conflict-free.

### Options & Return Signature
```typescript
interface SandboxEvalOptions {
    tenantId?: string;
    fuel?: number;
    memoryBytes?: number;
    allowRead?: string[];
    allowWrite?: string[];
    deterministicSeed?: number;
    auditSecret?: string;
}

interface SandboxEvalOutcome {
    ok: boolean;
    stdout: string[];
    returnValue: string;
    fuelUsed: number;
    memoryUsedBytes: number;
    totalTimeUs: number;
    violation: string | null;
}
```

---

## 2. Sub-5us VFS & KV State Checkpoints (`Slice.checkpoint()` & `Slice.restore(id)`)

Implemented in [`checkpoint_tenant_state`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3306-L3335) and [`restore_tenant_state`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3337-L3353):

- **`const snapId: number = Slice.checkpoint()`**: Captures an instant in-memory snapshot of all files/directories in the thread-local `node:fs` VFS slab (`BOA_VFS_CACHE` / `BOA_VFS_DIRS`) **and** all keys belonging to the active tenant across all 16 shards of `EDGE_KV_SLAB` (`Slice.kv`).
- **`const ok: boolean = Slice.restore(snapId)`**: Restores both the `node:fs` VFS slab and the tenant's `Slice.kv` keys to the exact state captured at `snapId` in `< 5 microseconds`.

### Runnable Tree-of-Thought (ToT) Agent Example

```typescript
import fs from "node:fs";

class TreeOfThoughtAgent {
    public run(): string {
        // 1. Establish baseline state in VFS and Slice.kv
        fs.writeFileSync("/workspace/model.ts", "export const weights = [1, 2, 3];");
        Slice.kv.set("agent:step", "baseline", 0);

        // 2. Capture < 5 us checkpoint before exploring untrusted LLM candidate branches
        const snapId = Slice.checkpoint();

        // 3. Candidate Branch 1: Mutates VFS + KV, then attempts prototype pollution
        fs.writeFileSync("/workspace/model.ts", "CORRUPTED_WEIGHTS");
        Slice.kv.set("agent:step", "corrupted-branch-1", 0);
        const branch1 = Slice.evalSandbox("Object.prototype.polluted = true;", { fuel: 50_000 });
        if (!branch1.ok) {
            // Instant rollback of both VFS and KV!
            Slice.restore(snapId);
        }

        // 4. Candidate Branch 2: Valid TypeScript evaluated in attenuated child sandbox
        const branch2 = Slice.evalSandbox(
            "const score: number = 48 * 2; console.log('candidate-2-pass'); score",
            { fuel: 250_000, allowRead: ["/workspace"] }
        );

        return JSON.stringify({
            vfsRestored: fs.readFileSync("/workspace/model.ts", "utf8"),
            kvRestored: Slice.kv.get("agent:step"),
            branch1Blocked: !branch1.ok,
            branch2Ok: branch2.ok,
            branch2Score: branch2.returnValue,
            branch2TimeUs: branch2.totalTimeUs
        });
    }
}

console.log(new TreeOfThoughtAgent().run());
```

---

## 3. Built-In Model Context Protocol Server (`slice mcp`)

`slice mcp` (implemented in [`cmd_mcp`](file:///p:/opensource/slice/crates/slice-cli/src/main.rs#L1014-L1268)) runs a compliant **JSON-RPC 2.0 Model Context Protocol (`2024-11-05`)** server over `stdin`/`stdout`.

### Configuring in Claude Code / Cursor (`mcp.json`)
```json
{
  "mcpServers": {
    "slice-microvm": {
      "command": "slice",
      "args": [
        "mcp",
        "--allow-read=/workspace",
        "--allow-write=/workspace",
        "--audit-egress=sk_live_production_secret"
      ]
    }
  }
}
```

### Exposed MCP Tools (`tools/list` & `tools/call`)

| Tool Name | Arguments | Description |
| :--- | :--- | :--- |
| **`slice_eval_ts`** | `{ code, fuel?, memory?, allowRead?, allowWrite?, auditSecret? }` | Compiles (via Oxc) & executes TypeScript/JavaScript inside an attenuated `SliceIsolate` sandbox (`~27-186 us`). |
| **`slice_kv_get`** | `{ key }` | Reads `key` from the MCP host's 16-way sharded `Slice.kv` partition. |
| **`slice_kv_set`** | `{ key, value }` | Writes `key` -> `value` into the MCP host's 16-way sharded `Slice.kv` partition. |
| **`slice_checkpoint`** | `{ action: "checkpoint" \| "restore", snapshot_id? }` | Creates or restores a `< 5 us` in-memory VFS + KV checkpoint for agent state rollback. |

### Verifying `slice mcp` with `--smoke-test`
```bash
slice mcp --smoke-test
```

