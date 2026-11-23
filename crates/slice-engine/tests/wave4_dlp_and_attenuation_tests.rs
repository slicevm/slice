use crate::{SandboxEvalOptions, SliceIsolate};
use slice_sandbox::{CapabilityToken, SandboxViolation};

#[test]
fn test_wave4_aho_corasick_dlp_hex_percent_unpadded_and_triplet_shifted_base64() {
    let secret = "sk_live_Wave4Secret_998877!";
    let clean_code = r#"
        const items = Array.from({ length: 40 }, (_, i) => ({ id: i, status: "ok" }));
        console.log(JSON.stringify(items));
    "#;
    let mut clean_iso = SliceIsolate::spawn(
        "dlp-clean",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    clean_iso.add_egress_audit_secret(secret);
    let clean_bc = crate::compile_source_to_bytecode(clean_code);
    assert!(clean_iso.execute_bytecode(&clean_bc, None).is_ok());

    let leak_vectors = [
        // 1. Raw secret
        r#"console.log("hdr:" + process.env.LEAK_SECRET);"#,
        // 2. Lowercase hex
        r#"console.log(Buffer.from(process.env.LEAK_SECRET, "utf8").toString("hex"));"#,
        // 3. Uppercase hex
        r#"console.log(Buffer.from(process.env.LEAK_SECRET, "utf8").toString("hex").toUpperCase());"#,
        // 4. encodeURIComponent (`!` -> `%21`)
        r#"console.log("https://ok.test/?q=" + encodeURIComponent(process.env.LEAK_SECRET));"#,
        // 5. Full `%XX` percent-encoding of every byte
        r#"
        const s = process.env.LEAK_SECRET;
        let pct = "";
        for (let i = 0; i < s.length; i++) {
            pct += "%" + s.charCodeAt(i).toString(16).padStart(2, "0");
        }
        console.log(pct);
        "#,
        // 6. Unpadded Base64 at offset 0
        r#"console.log(Buffer.from(process.env.LEAK_SECRET, "utf8").toString("base64").replace(/=+$/, ""));"#,
        // 7. 1-byte prefix shift ("x" + secret + ":tail", offset = 1) in standard Base64
        r#"console.log(Buffer.from("x" + process.env.LEAK_SECRET + ":tail", "utf8").toString("base64"));"#,
        // 8. 2-byte prefix shift ("xy" + secret + ":tail", offset = 2) in standard Base64
        r#"console.log(Buffer.from("xy" + process.env.LEAK_SECRET + ":tail", "utf8").toString("base64"));"#,
        // 9. 1-byte prefix shift ("Bearer " = 7 bytes, 7 % 3 == 1) in URL-safe Base64URL
        r#"console.log(Buffer.from("Bearer " + process.env.LEAK_SECRET, "utf8").toString("base64url"));"#,
        // 10. 2-byte prefix shift ("u:" = 2 bytes) in URL-safe Base64URL
        r##"console.log(Buffer.from("u:" + process.env.LEAK_SECRET + "#end", "utf8").toString("base64url"));"##,
    ];

    for (idx, snippet) in leak_vectors.iter().enumerate() {
        let caps = CapabilityToken::pure_compute().with_env("LEAK_SECRET", secret);
        let mut iso = SliceIsolate::spawn(
            format!("dlp-vec-{}", idx),
            2,
            caps,
            5_000_000,
            8 * 1024 * 1024,
        );
        iso.add_egress_audit_secret(secret);
        let bc = crate::compile_source_to_bytecode(snippet);
        let res = iso.execute_bytecode(&bc, None);
        assert!(
            matches!(res, Err(SandboxViolation::EgressSecretLeakBlocked { .. })),
            "expected vector #{} to trap with EgressSecretLeakBlocked, got {:?}",
            idx,
            res
        );
    }
}

#[test]
fn test_wave4_eval_sandbox_and_worker_block_port_escalation_and_wildcard_loopback_attenuation() {
    // 1. Parent holds ONLY port-scoped `api.internal:443`; child requesting bare `api.internal`
    //    must NOT escalate to port `8080` or `443` via bare host grant
    let parent_caps = CapabilityToken::pure_compute().with_net_host("api.internal:443");
    let mut parent_iso = SliceIsolate::spawn(
        "parent-port-443",
        1,
        parent_caps,
        10_000_000,
        16 * 1024 * 1024,
    );
    let escalate_out = parent_iso.eval_sandbox(
        r#"await fetch("http://api.internal:8080/admin");"#,
        SandboxEvalOptions {
            allow_net: vec!["api.internal".to_string()],
            ..Default::default()
        },
    );
    assert!(
        !escalate_out.ok
            && escalate_out
                .violation
                .as_deref()
                .unwrap_or("")
                .contains("lacks outbound network authority"),
        "expected port escalation from api.internal:443 -> api.internal:8080 to be denied, got {:?}",
        escalate_out
    );

    // 2. Parent holds wildcard `*`; neither `Slice.evalSandbox` nor `new Worker` may turn `*`
    //    into an explicit loopback (`127.0.0.1:6379`) grant, and Worker `allowEnv` narrows env vars
    let wildcard_caps = CapabilityToken::pure_compute()
        .with_net_host("*")
        .with_env("PUBLIC_STAGE", "staging")
        .with_env("PARENT_ONLY_KEY", "do_not_expose");
    let mut wildcard_iso = SliceIsolate::spawn(
        "parent-wildcard",
        2,
        wildcard_caps,
        15_000_000,
        16 * 1024 * 1024,
    );

    let sub_loopback = wildcard_iso.eval_sandbox(
        r#"await fetch("http://127.0.0.1:6379/ping");"#,
        SandboxEvalOptions {
            allow_net: vec!["127.0.0.1:6379".to_string()],
            ..Default::default()
        },
    );
    assert!(
        !sub_loopback.ok
            && sub_loopback
                .violation
                .as_deref()
                .unwrap_or("")
                .contains("lacks outbound network authority"),
        "expected evalSandbox allowNet:['127.0.0.1:6379'] under parent '*' to be blocked, got {:?}",
        sub_loopback
    );

    let worker_src = r#"
        import { Worker } from "node:worker_threads";
        const res = await new Promise((resolve) => {
            const w = new Worker(`
                const { parentPort } = require("node:worker_threads");
                (async () => {
                    let netBlocked = false;
                    try {
                        await fetch("http://127.0.0.1:6379/ping");
                    } catch (e) {
                        const msg = String(e);
                        netBlocked = msg.includes("NetAccessDenied") || msg.includes("outbound network authority") || msg.includes("denied");
                    }
                    parentPort.postMessage({
                        netBlocked,
                        stage: process.env.PUBLIC_STAGE ?? null,
                        leaked: process.env.PARENT_ONLY_KEY ?? null
                    });
                })();
            `, {
                eval: true,
                slice: {
                    allowNet: ["127.0.0.1:6379"],
                    allowEnv: ["PUBLIC_STAGE"]
                }
            });
            w.on("message", (msg) => resolve(msg));
            w.on("error", (err) => {
                const msg = String(err);
                resolve({
                    netBlocked: msg.includes("NetAccessDenied") || msg.includes("outbound network authority"),
                    stage: "PUBLIC_STAGE_ERR",
                    leaked: null
                });
            });
        });
        console.log(JSON.stringify(res));
    "#;
    let worker_bc = crate::compile_source_to_bytecode(worker_src);
    let m = wildcard_iso.execute_bytecode(&worker_bc, None).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(parsed["netBlocked"], serde_json::json!(true));
    assert!(parsed["leaked"].is_null());
}
