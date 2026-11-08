use crate::config::ResolvedExecutionConfig;
use anyhow::Result;
use slice_engine::SliceIsolate;
use std::io::Write;

/// Maximum allowed MCP JSON-RPC 2.0 request line length (`4 MB`) (`CLI-SEC-04`).
pub(crate) const MAX_MCP_LINE_BYTES: usize = 4 * 1024 * 1024;

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
    let mut stdout = std::io::stdout();
    let mut line = String::new();
    while stdin.read_line(&mut line)? > 0 {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            if let Some(resp_json) = handle_mcp_jsonrpc_line(trimmed, &mut isolate) {
                writeln!(stdout, "{}", resp_json)?;
                stdout.flush()?;
            }
        }
        line.clear();
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
                            "fuel": { "type": "integer", "description": "Maximum CPU instruction fuel budget" },
                            "memory": { "type": "integer", "description": "Maximum memory ceiling in bytes" },
                            "allowRead": { "type": "string", "description": "Optional allowed VFS read prefix" },
                            "allowWrite": { "type": "string", "description": "Optional allowed VFS write prefix" },
                            "allowNet": { "type": "string", "description": "Optional allowed network host" },
                            "allowExec": { "type": "string", "description": "Optional allowed subprocess binary" },
                            "auditSecret": { "type": "string", "description": "Optional secret to guard against egress leakage" }
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
                    if let Some(m) = args.get("memory").and_then(|v| v.as_u64()) {
                        opts.memory_bytes = Some(m as usize);
                    }
                    if let Some(r) = args.get("allowRead").and_then(|v| v.as_str()) {
                        opts.allow_read.push(r.to_string());
                    }
                    if let Some(w) = args.get("allowWrite").and_then(|v| v.as_str()) {
                        opts.allow_write.push(w.to_string());
                    }
                    // CLI-SEC-04: Wire `allowNet` and `allowExec` into `SandboxEvalOptions`
                    if let Some(nh) = args.get("allowNet").and_then(|v| v.as_str()) {
                        opts.allow_net.push(nh.to_string());
                    }
                    if let Some(eb) = args.get("allowExec").and_then(|v| v.as_str()) {
                        opts.allow_exec.push(eb.to_string());
                    }
                    if let Some(s) = args.get("auditSecret").and_then(|v| v.as_str()) {
                        opts.audit_secret = Some(s.to_string());
                    }
                    let outcome = isolate.eval_sandbox(code, opts);
                    let payload = serde_json::json!({
                        "ok": outcome.ok,
                        "value": outcome.return_value,
                        "stdout": outcome.stdout,
                        "fuelUsed": outcome.fuel_used,
                        "timeUs": outcome.total_time_us,
                        "violation": outcome.violation
                    });
                    serde_json::json!({
                        "content": [{ "type": "text", "text": payload.to_string() }],
                        "isError": !outcome.ok
                    })
                }
                "slice_kv_set" => {
                    let key = args.get("key").and_then(|k| k.as_str()).unwrap_or("");
                    let val = args.get("value").and_then(|v| v.as_str()).unwrap_or("");
                    let ok = isolate.kv_set(key, val, 0).is_ok();
                    serde_json::json!({
                        "content": [{ "type": "text", "text": serde_json::json!({"ok": ok, "key": key}).to_string() }],
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
                        let snap_id = args
                            .get("snapshot_id")
                            .and_then(|s| s.as_u64())
                            .unwrap_or(0);
                        let restored = isolate.restore(snap_id);
                        serde_json::json!({
                            "content": [{ "type": "text", "text": serde_json::json!({"action": "restore", "snapshot_id": snap_id, "restored": restored}).to_string() }],
                            "isError": !restored
                        })
                    } else {
                        let snap_id = isolate.checkpoint();
                        serde_json::json!({
                            "content": [{ "type": "text", "text": serde_json::json!({"action": "checkpoint", "snapshot_id": snap_id}).to_string() }],
                            "isError": false
                        })
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
