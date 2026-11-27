use crate::{
    ExtCallCtx, ExtError, ExtFuture, ExtMethod, ExtResult, ExtValue, ExtensionRegistry,
    SliceExtension, SliceIsolate, compile_source_to_bytecode,
};
use slice_sandbox::{CapabilityToken, SandboxViolation};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct TestStoreExt {
    data: Arc<Mutex<HashMap<String, String>>>,
}

impl TestStoreExt {
    fn new() -> Self {
        let mut map = HashMap::new();
        map.insert("greeting".to_string(), "hello-from-host".to_string());
        Self {
            data: Arc::new(Mutex::new(map)),
        }
    }
}

const STORE_METHODS: &[ExtMethod] = &[
    ExtMethod::sync("get").with_deterministic(true),
    ExtMethod::r#async("put").with_allow_in_checkpoint(false),
    ExtMethod::sync("echoBytes")
        .with_max_arg_bytes(32)
        .with_max_result_bytes(32)
        .with_fuel(100, 2),
    ExtMethod::sync("oversizedResult").with_max_result_bytes(8),
    ExtMethod::r#async("slow"),
];

impl SliceExtension for TestStoreExt {
    fn name(&self) -> &'static str {
        "store"
    }

    fn methods(&self) -> &'static [ExtMethod] {
        STORE_METHODS
    }

    fn call(&self, ctx: &ExtCallCtx, method: &str, arg: ExtValue) -> ExtResult {
        match method {
            "get" => {
                let key = match arg {
                    ExtValue::Json(serde_json::Value::String(s)) => s,
                    _ => return Err(ExtError::InvalidArgument("expected string key".to_string())),
                };
                let val = self.data.lock().unwrap().get(&key).cloned();
                Ok(ExtValue::Json(serde_json::json!({
                    "tenant": ctx.tenant_id,
                    "value": val,
                })))
            }
            "echoBytes" => match arg {
                ExtValue::Bytes(mut b) => {
                    b.reverse();
                    Ok(ExtValue::Bytes(b))
                }
                _ => Err(ExtError::InvalidArgument("expected bytes".to_string())),
            },
            "oversizedResult" => Ok(ExtValue::Bytes(vec![0xAA; 64])),
            _ => Err(ExtError::Denied(format!("store.{method}"))),
        }
    }

    fn call_async(&self, ctx: &ExtCallCtx, method: &str, arg: ExtValue) -> ExtFuture {
        let data = Arc::clone(&self.data);
        let cancel = Arc::clone(&ctx.cancel);
        match method {
            "put" => Box::pin(async move {
                let ExtValue::Json(obj) = arg else {
                    return Err(ExtError::InvalidArgument(
                        "expected JSON object".to_string(),
                    ));
                };
                let k = obj["key"].as_str().unwrap_or("").to_string();
                let v = obj["value"].as_str().unwrap_or("").to_string();
                data.lock().unwrap().insert(k.clone(), v.clone());
                Ok(ExtValue::Json(serde_json::json!({ "ok": true, "key": k })))
            }),
            "slow" => Box::pin(async move {
                // Pending future that yields until cancelled.
                struct WaitUntilCancel(Arc<std::sync::atomic::AtomicBool>);
                impl std::future::Future for WaitUntilCancel {
                    type Output = ExtResult;
                    fn poll(
                        self: std::pin::Pin<&mut Self>,
                        _cx: &mut std::task::Context<'_>,
                    ) -> std::task::Poll<Self::Output> {
                        if self.0.load(std::sync::atomic::Ordering::Acquire) {
                            std::task::Poll::Ready(Err(ExtError::Cancelled))
                        } else {
                            std::task::Poll::Pending
                        }
                    }
                }
                WaitUntilCancel(cancel).await
            }),
            other => {
                let target = format!("store.{other}");
                Box::pin(async move { Err(ExtError::Denied(target)) })
            }
        }
    }
}

#[test]
fn test_v0150_host_extensions_sync_async_bytes_frozen_and_attenuation() {
    let registry = ExtensionRegistry::new().with(Arc::new(TestStoreExt::new()));
    let caps = CapabilityToken::pure_compute().with_extension("store");

    let code = r#"
        const frozenExt = Object.isFrozen(Slice.ext);
        const frozenNs = Object.isFrozen(Slice.ext.store);
        const frozenFn = Object.isFrozen(Slice.ext.store.get);

        const initial = Slice.ext.store.get("greeting");
        const putRes = await Slice.ext.store.put({ key: "k1", value: "v1" });
        const afterPut = Slice.ext.store.get("k1");
        const rev = Slice.ext.store.echoBytes(new Uint8Array([1, 2, 3, 4]));

        // Child sandbox attenuation: parent has "store", child requests only "store.get"
        const childNarrow = Slice.evalSandbox(`
            const got = Slice.ext.store.get("k1");
            let putDeniedCode = null;
            try {
                await Slice.ext.store.put({ key: "k2", value: "v2" });
            } catch (e) {
                putDeniedCode = e.code;
            }
            JSON.stringify({ got: got.value, putDeniedCode });
        `, { allowExt: ["store.get"] });

        // Child sandbox without allowExt inherits NO extensions
        const childNone = Slice.evalSandbox(`
            let code = null;
            try {
                Slice.ext.store.get("k1");
            } catch (e) {
                code = e.code;
            }
            code;
        `, {});

        JSON.stringify({
            frozenExt,
            frozenNs,
            frozenFn,
            initial: initial.value,
            putOk: putRes.ok,
            afterPut: afterPut.value,
            rev: Array.from(rev),
            childNarrowOk: childNarrow.ok,
            childNarrowVal: JSON.parse(childNarrow.returnValue),
            childNoneOk: childNone.ok,
            childNoneVal: childNone.returnValue,
        });
    "#;

    let bc = compile_source_to_bytecode(code);
    let mut iso = SliceIsolate::spawn_with_extensions(
        "ext-tenant-1",
        1,
        caps,
        20_000_000,
        16 * 1024 * 1024,
        registry,
    );
    let metrics = iso.execute_bytecode(&bc, None).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&metrics.return_value).unwrap();

    assert_eq!(parsed["frozenExt"], true);
    assert_eq!(parsed["frozenNs"], true);
    assert_eq!(parsed["frozenFn"], true);
    assert_eq!(parsed["initial"], "hello-from-host");
    assert_eq!(parsed["putOk"], true);
    assert_eq!(parsed["afterPut"], "v1");
    assert_eq!(parsed["rev"], serde_json::json!([4, 3, 2, 1]));
    assert_eq!(parsed["childNarrowOk"], true);
    assert_eq!(parsed["childNarrowVal"]["got"], "v1");
    assert_eq!(
        parsed["childNarrowVal"]["putDeniedCode"],
        "ERR_SLICE_EXT_DENIED"
    );
    assert_eq!(parsed["childNoneOk"], true);
    assert_eq!(parsed["childNoneVal"], "ERR_SLICE_EXT_DENIED");
}

#[test]
fn test_v0150_host_extensions_deny_bounds_dlp_determinism_checkpoint_and_timeout() {
    let registry = ExtensionRegistry::new().with(Arc::new(TestStoreExt::new()));

    // 1. Deny-by-default: ungranted extension throws catchable ERR_SLICE_EXT_DENIED and promotes if uncaught
    let ungranted_bc = compile_source_to_bytecode(r#"Slice.ext.store.get("greeting");"#);
    let mut iso_denied = SliceIsolate::spawn_with_extensions(
        "ext-denied",
        2,
        CapabilityToken::pure_compute(),
        20_000_000,
        16 * 1024 * 1024,
        registry.clone(),
    );
    let err = iso_denied
        .execute_bytecode(&ungranted_bc, None)
        .unwrap_err();
    assert!(
        matches!(err, SandboxViolation::ExtensionDenied { ref target, .. } if target == "store.get"),
        "expected ExtensionDenied for store.get, got {err:?}"
    );
    assert!(
        iso_denied
            .denied_capabilities()
            .iter()
            .any(|d| d.contains("store.get"))
    );

    // 2. Argument & result size bounds
    let bounds_bc = compile_source_to_bytecode(
        r#"
        let argErr = false;
        try { Slice.ext.store.echoBytes(new Uint8Array(64)); } catch (e) { argErr = true; }
        let resErr = false;
        try { Slice.ext.store.oversizedResult(null); } catch (e) { resErr = true; }
        JSON.stringify({ argErr, resErr });
        "#,
    );
    let mut iso_bounds = SliceIsolate::spawn_with_extensions(
        "ext-bounds",
        3,
        CapabilityToken::pure_compute().with_extension("store"),
        20_000_000,
        16 * 1024 * 1024,
        registry.clone(),
    );
    let m_bounds = iso_bounds.execute_bytecode(&bounds_bc, None).unwrap();
    let v_bounds: serde_json::Value = serde_json::from_str(&m_bounds.return_value).unwrap();
    assert_eq!(v_bounds["argErr"], true);
    assert_eq!(v_bounds["resErr"], true);

    // 3. Ring 4 Egress DLP blocks secret passed to extension
    let dlp_bc = compile_source_to_bytecode(
        r#"await Slice.ext.store.put({ key: "x", value: "sk_live_ext_secret_777" });"#,
    );
    let mut iso_dlp = SliceIsolate::spawn_with_extensions(
        "ext-dlp",
        4,
        CapabilityToken::pure_compute().with_extension("store"),
        20_000_000,
        16 * 1024 * 1024,
        registry.clone(),
    );
    iso_dlp.add_egress_audit_secret("sk_live_ext_secret_777");
    let dlp_err = iso_dlp.execute_bytecode(&dlp_bc, None).unwrap_err();
    assert!(
        matches!(dlp_err, SandboxViolation::EgressSecretLeakBlocked { .. }),
        "expected EgressSecretLeakBlocked, got {dlp_err:?}"
    );

    // 4. Determinism guard: deterministic=true allowed, deterministic=false denied under deterministic_seed
    let det_bc = compile_source_to_bytecode(
        r#"
        const ok = Slice.ext.store.get("greeting").value;
        let nonDetCode = null;
        try { await Slice.ext.store.put({ key: "a", value: "b" }); } catch (e) { nonDetCode = e.code; }
        JSON.stringify({ ok, nonDetCode });
        "#,
    );
    let mut iso_det = SliceIsolate::spawn_with_extensions(
        "ext-det",
        5,
        CapabilityToken::pure_compute().with_extension("store"),
        20_000_000,
        16 * 1024 * 1024,
        registry.clone(),
    );
    iso_det.set_deterministic_seed(Some(42));
    let m_det = iso_det.execute_bytecode(&det_bc, None).unwrap();
    let v_det: serde_json::Value = serde_json::from_str(&m_det.return_value).unwrap();
    assert_eq!(v_det["ok"], "hello-from-host");
    assert_eq!(v_det["nonDetCode"], "ERR_SLICE_EXT_DENIED");

    // 5. Checkpoint region guard: allow_in_checkpoint=false refused after Slice.checkpoint()
    let cp_bc = compile_source_to_bytecode(
        r#"
        const snap = Slice.checkpoint();
        let cpDenied = null;
        try { await Slice.ext.store.put({ key: "a", value: "b" }); } catch (e) { cpDenied = e.code; }
        Slice.restore(snap);
        cpDenied;
        "#,
    );
    let mut iso_cp = SliceIsolate::spawn_with_extensions(
        "ext-cp",
        6,
        CapabilityToken::pure_compute()
            .with_extension("store")
            .with_checkpoints(true),
        20_000_000,
        16 * 1024 * 1024,
        registry.clone(),
    );
    let m_cp = iso_cp.execute_bytecode(&cp_bc, None).unwrap();
    assert_eq!(m_cp.return_value, "ERR_SLICE_EXT_DENIED");

    // 6. Wall-clock timeout cancels async extension future
    let slow_bc = compile_source_to_bytecode(r#"await Slice.ext.store.slow(null);"#);
    let mut iso_slow = SliceIsolate::spawn_with_extensions(
        "ext-slow",
        7,
        CapabilityToken::pure_compute().with_extension("store"),
        20_000_000,
        16 * 1024 * 1024,
        registry,
    );
    iso_slow.set_wall_clock_limit_ms(Some(25));
    let start = std::time::Instant::now();
    let slow_err = iso_slow.execute_bytecode(&slow_bc, None).unwrap_err();
    assert!(
        matches!(slow_err, SandboxViolation::WallClockExceeded { .. }),
        "expected WallClockExceeded, got {slow_err:?}"
    );
    assert!(start.elapsed() < Duration::from_secs(2));
}
