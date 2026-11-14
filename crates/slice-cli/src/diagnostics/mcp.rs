use crate::config::ResolvedExecutionConfig;
use anyhow::Result;
use slice_engine::SliceIsolate;
use std::io::{BufRead, Write};

/// Maximum allowed MCP JSON-RPC 2.0 request line length (`4 MB`) (`CLI-SEC-04`).
pub(crate) const MAX_MCP_LINE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum McpLineRead {
    Eof,
    Line,
    TooLong,
}

/// Reads a single `\n`-delimited JSON-RPC line from `reader` into `out` while bounding
/// in-memory growth strictly at `max_bytes` and draining any oversized remainder without OOM.
pub(crate) fn read_bounded_mcp_line<R: BufRead>(
    reader: &mut R,
    out: &mut Vec<u8>,
    max_bytes: usize,
) -> std::io::Result<McpLineRead> {
    out.clear();
    let mut over_limit = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if out.is_empty() && !over_limit {
                return Ok(McpLineRead::Eof);
            }
            return Ok(if over_limit {
                McpLineRead::TooLong
            } else {
                McpLineRead::Line
            });
        }
        if let Some(nl_pos) = memchr::memchr(b'\n', available) {
            if !over_limit {
                if out.len().saturating_add(nl_pos) > max_bytes {
                    over_limit = true;
                    out.clear();
                } else {
                    out.extend_from_slice(&available[..nl_pos]);
                }
            }
            reader.consume(nl_pos + 1);
            return Ok(if over_limit {
                McpLineRead::TooLong
            } else {
                McpLineRead::Line
            });
        }
        let chunk_len = available.len();
        if !over_limit {
            if out.len().saturating_add(chunk_len) > max_bytes {
                over_limit = true;
                out.clear();
            } else {
                out.extend_from_slice(available);
            }
        }
        reader.consume(chunk_len);
    }
}

fn push_string_or_array_arg(args: &serde_json::Value, key: &str, out: &mut Vec<String>) {
    if let Some(val) = args.get(key) {
        if let Some(s) = val.as_str() {
            for item in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                out.push(item.to_string());
            }
        } else if let Some(arr) = val.as_array() {
            for elem in arr {
                if let Some(s) = elem.as_str() {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        out.push(trimmed.to_string());
                    }
                }
            }
        }
    }
}

pub(crate) fn cmd_mcp(args: &[String]) -> Result<()> {
    let exec_cfg = ResolvedExecutionConfig::from_cli_args(args)?;
    let smoke_test = args.iter().any(|a| a == "--smoke-test");

    SliceIsolate::prewarm_boa_pool(2);
    let mut isolate = SliceIsolate::spawn(
        "mcp-agent-host",
        100,
        exec_cfg.caps,
        exec_cfg.fuel_limit,
        exec_cfg.mem_limit,
    );
    isolate.set_deterministic_seed(exec_cfg.deterministic_seed);
    for sec in &exec_cfg.audit_secrets {
        isolate.add_egress_audit_secret(sec.clone());
    }

    if smoke_test {
        let requests = [
            concat!(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","clientInfo":{"name":"slice-smoke","version":""#,
                env!("CARGO_PKG_VERSION"),
                r#""}}}"#
            ),
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"const x: number = 21 * 2; console.log('mcp-eval=' + x); x","fuel":500000}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"slice_kv_set","arguments":{"key":"agent_state","value":"ready"}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"slice_kv_get","arguments":{"key":"agent_state"}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"slice_checkpoint","arguments":{"action":"checkpoint"}}}"#,
        ];
        println!("=========================================================================");
        println!("  SLICE (NEXUS-JS) MODEL CONTEXT PROTOCOL (MCP) JSON-RPC 2.0 SMOKE TEST");
        println!("=========================================================================");
        for req in requests {
            if let Some(resp) = handle_mcp_jsonrpc_line(req, &mut isolate) {
                println!("  -> {}", resp);
            }
        }
        println!("=========================================================================");
        return Ok(());
    }

    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut stdout = std::io::stdout();
    let mut raw_line = Vec::with_capacity(4096);
    loop {
        match read_bounded_mcp_line(&mut reader, &mut raw_line, MAX_MCP_LINE_BYTES)? {
            McpLineRead::Eof => break,
            McpLineRead::TooLong => {
                let err_resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32600, "message": "Invalid Request: MCP JSON-RPC line exceeds 4 MB limit" }
                });
                writeln!(stdout, "{}", err_resp)?;
                stdout.flush()?;
            }
            McpLineRead::Line => {
                let line_str = match std::str::from_utf8(&raw_line) {
                    Ok(s) => s.trim(),
                    Err(e) => {
                        let err_resp = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": null,
                            "error": { "code": -32700, "message": format!("Parse error: invalid UTF-8 ({})", e) }
                        });
                        writeln!(stdout, "{}", err_resp)?;
                        stdout.flush()?;
                        continue;
                    }
                };
                if !line_str.is_empty() {
                    if let Some(resp_json) = handle_mcp_jsonrpc_line(line_str, &mut isolate) {
                        writeln!(stdout, "{}", resp_json)?;
                        stdout.flush()?;
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn handle_mcp_jsonrpc_line(line: &str, isolate: &mut SliceIsolate) -> Option<String> {
    if line.len() > MAX_MCP_LINE_BYTES {
        return Some(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32600, "message": "Invalid Request: MCP JSON-RPC line exceeds 4 MB limit" }
            })
            .to_string(),
        );
    }

    let parsed: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return Some(
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("Parse error: {}", e) }
                })
                .to_string(),
            );
        }
    };

    let id = parsed.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method = parsed.get("method").and_then(|m| m.as_str()).unwrap_or("");

    // Notifications (such as `notifications/initialized`) do not expect a response
    if method.starts_with("notifications/") {
        return None;
    }

    let result = match method {
        "initialize" => serde_json::json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": {
                "name": "slice-nexusjs-mcp",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {
                "tools": {}
            }
        }),
        "ping" => serde_json::json!({}),
        "tools/list" => serde_json::json!({
            "tools": [
                {
                    "name": "slice_eval_ts",
                    "description": "Execute untrusted TypeScript/JavaScript inside a sub-microsecond OCap/SES Slice MicroVM sandbox",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "code": { "type": "string", "description": "TypeScript or JavaScript source code to evaluate" },
                            "fuel": { "type": "integer", "description": "Optional CPU instruction fuel budget, attenuated against the server --fuel ceiling" },
                            "memory": { "type": "integer", "description": "Optional memory ceiling in bytes, attenuated against the server --memory ceiling" },
                            "memoryBytes": { "type": "integer", "description": "Optional memory ceiling in bytes (alias for memory), attenuated against the server --memory ceiling" },
                            "allowRead": { "type": ["string", "array"], "description": "Optional allowed VFS read prefix(es) attenuated against server grants" },
                            "allowWrite": { "type": ["string", "array"], "description": "Optional allowed VFS write prefix(es) attenuated against server grants" },
                            "allowNet": { "type": ["string", "array"], "description": "Optional allowed network host(s) attenuated against server grants" },
                            "allowExec": { "type": ["string", "array"], "description": "Optional allowed subprocess binary(ies) attenuated against server grants" },
                            "allowEnv": { "type": ["string", "array"], "description": "Optional allowed environment variable key(s) attenuated against server grants" },
                            "auditSecret": { "type": ["string", "array"], "description": "Optional secret(s) to guard against egress leakage" },
                            "auditSecrets": { "type": ["string", "array"], "description": "Optional array of secrets to guard against egress leakage" },
                            "deterministicSeed": { "type": "integer", "description": "Optional deterministic seed for Math.random() and Date.now()" },
                            "checkpoints": { "type": "boolean", "description": "Whether guest Slice.checkpoint() / Slice.restore() calls are allowed (attenuated against server grants)" },
                            "maxCheckpoints": { "type": "integer", "description": "Maximum number of guest checkpoints allowed during evaluation (attenuated against remaining server --max-checkpoints quota)" }
                        },
                        "required": ["code"]
                    }
                },
                {
                    "name": "slice_kv_get",
                    "description": "Read a key from the zero-copy sharded Slice.kv store",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "key": { "type": "string" }
                        },
                        "required": ["key"]
                    }
                },
                {
                    "name": "slice_kv_set",
                    "description": "Write a key-value pair to the zero-copy sharded Slice.kv store",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "key": { "type": "string" },
                            "value": { "type": "string" }
                        },
                        "required": ["key", "value"]
                    }
                },
                {
                    "name": "slice_checkpoint",
                    "description": "Create or restore an in-memory VFS + KV state snapshot for Tree-of-Thought branching",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "action": { "type": "string", "enum": ["checkpoint", "restore"] },
                            "snapshot_id": { "type": "integer" }
                        },
                        "required": ["action"]
                    }
                }
            ]
        }),
        "tools/call" => {
            let params = parsed
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            match tool_name {
                "slice_eval_ts" => {
                    let code = args.get("code").and_then(|c| c.as_str()).unwrap_or("");
                    let mut opts = slice_engine::SandboxEvalOptions::default();
                    if let Some(f) = args.get("fuel").and_then(|v| v.as_u64()) {
                        opts.fuel = Some(f);
                    }
                    if let Some(m) = args
                        .get("memory")
                        .or_else(|| args.get("memoryBytes"))
                        .and_then(|v| v.as_u64())
                    {
                        opts.memory_bytes = Some(usize::try_from(m).unwrap_or(usize::MAX));
                    }
                    push_string_or_array_arg(&args, "allowRead", &mut opts.allow_read);
                    push_string_or_array_arg(&args, "allowWrite", &mut opts.allow_write);
                    push_string_or_array_arg(&args, "allowNet", &mut opts.allow_net);
                    push_string_or_array_arg(&args, "allowExec", &mut opts.allow_exec);
                    push_string_or_array_arg(&args, "allowEnv", &mut opts.allow_env);
                    push_string_or_array_arg(&args, "auditSecret", &mut opts.audit_secrets);
                    push_string_or_array_arg(&args, "auditSecrets", &mut opts.audit_secrets);
                    if let Some(seed) = args.get("deterministicSeed").and_then(|v| v.as_u64()) {
                        opts.deterministic_seed = Some(seed);
                    }
                    if let Some(cp) = args
                        .get("checkpoints")
                        .or_else(|| args.get("allowCheckpoints"))
                        .and_then(|v| v.as_bool())
                    {
                        opts.checkpoints = Some(cp);
                    }
                    if let Some(max_cp) = args.get("maxCheckpoints").and_then(|v| v.as_u64()) {
                        opts.max_checkpoints = Some(u32::try_from(max_cp).unwrap_or(u32::MAX));
                    }
                    // Strictly attenuate per-call fuel, memory, and checkpoint limits against
                    // the host MCP isolate's configured ceilings (never raise fuel_limit,
                    // memory_limit_bytes, or reset session checkpoints_taken).
                    isolate.compartment.fuel_consumed = 0;
                    isolate.sync_live_kv_memory();
                    let outcome = isolate.eval_sandbox(code, opts);
                    isolate.compartment.fuel_consumed = 0;
                    isolate.sync_live_kv_memory();
                    let payload = serde_json::json!({
                        "ok": outcome.ok,
                        "value": outcome.return_value,
                        "returnValue": outcome.return_value,
                        "returnJson": outcome.return_json,
                        "error": outcome.error,
                        "violation": outcome.violation,
                        "stdout": outcome.stdout,
                        "fuelUsed": outcome.fuel_used,
                        "memoryUsedBytes": outcome.memory_used_bytes,
                        "peakMemoryBytes": outcome.peak_memory_bytes,
                        "spawnTimeNs": outcome.spawn_time_ns,
                        "compileTimeNs": outcome.compile_time_ns,
                        "execTimeNs": outcome.exec_time_ns,
                        "timeUs": outcome.total_time_us,
                        "totalTimeUs": outcome.total_time_us,
                        "checkpointsTaken": outcome.checkpoints_taken,
                        "restoresPerformed": outcome.restores_performed,
                        "restoreTimeNs": outcome.restore_time_ns
                    });
                    serde_json::json!({
                        "content": [{ "type": "text", "text": payload.to_string() }],
                        "isError": !outcome.ok
                    })
                }
                "slice_kv_set" => {
                    let key = args.get("key").and_then(|k| k.as_str()).unwrap_or("");
                    let val = args.get("value").and_then(|v| v.as_str()).unwrap_or("");
                    isolate.sync_live_kv_memory();
                    let res = isolate.kv_set(key, val, 0);
                    isolate.sync_live_kv_memory();
                    let ok = res.is_ok();
                    let body = match res {
                        Ok(()) => serde_json::json!({"ok": true, "key": key}),
                        Err(violation) => serde_json::json!({
                            "ok": false,
                            "key": key,
                            "violation": violation.to_string()
                        }),
                    };
                    serde_json::json!({
                        "content": [{ "type": "text", "text": body.to_string() }],
                        "isError": !ok
                    })
                }
                "slice_kv_get" => {
                    let key = args.get("key").and_then(|k| k.as_str()).unwrap_or("");
                    let val = isolate.kv_get(key);
                    serde_json::json!({
                        "content": [{ "type": "text", "text": serde_json::json!({"key": key, "value": val}).to_string() }],
                        "isError": false
                    })
                }
                "slice_checkpoint" => {
                    let action = args
                        .get("action")
                        .and_then(|a| a.as_str())
                        .unwrap_or("checkpoint");
                    if action == "restore" {
                        if !isolate.compartment.capabilities.allow_checkpoints {
                            let violation = slice_sandbox::SandboxViolation::CheckpointDenied {
                                tenant_id: isolate.compartment.tenant_id.clone(),
                                reason:
                                    "guest checkpoints/restores are disabled for this compartment"
                                        .to_string(),
                            };
                            serde_json::json!({
                                "content": [{ "type": "text", "text": serde_json::json!({"action": "restore", "restored": false, "violation": violation.to_string()}).to_string() }],
                                "isError": true
                            })
                        } else {
                            let snap_id = args
                                .get("snapshot_id")
                                .and_then(|s| s.as_u64())
                                .unwrap_or(0);
                            let restored = isolate.restore(snap_id);
                            if restored {
                                isolate.restores_performed =
                                    isolate.restores_performed.saturating_add(1);
                                isolate.sync_live_kv_memory();
                            }
                            serde_json::json!({
                                "content": [{ "type": "text", "text": serde_json::json!({"action": "restore", "snapshot_id": snap_id, "restored": restored}).to_string() }],
                                "isError": !restored
                            })
                        }
                    } else {
                        match isolate
                            .compartment
                            .verify_checkpoint(isolate.checkpoints_taken)
                        {
                            Ok(()) => {
                                let snap_id = isolate.checkpoint();
                                isolate.checkpoints_taken =
                                    isolate.checkpoints_taken.saturating_add(1);
                                serde_json::json!({
                                    "content": [{ "type": "text", "text": serde_json::json!({"action": "checkpoint", "snapshot_id": snap_id}).to_string() }],
                                    "isError": false
                                })
                            }
                            Err(violation) => serde_json::json!({
                                "content": [{ "type": "text", "text": serde_json::json!({"action": "checkpoint", "ok": false, "violation": violation.to_string()}).to_string() }],
                                "isError": true
                            }),
                        }
                    }
                }
                _ => {
                    return Some(
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32601, "message": format!("Unknown MCP tool: {}", tool_name) }
                        })
                        .to_string(),
                    );
                }
            }
        }
        _ => {
            return Some(
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("Method not found: {}", method) }
                })
                .to_string(),
            );
        }
    };

    Some(
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        })
        .to_string(),
    )
}
