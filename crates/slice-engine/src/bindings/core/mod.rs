//! Core Always-On Native Host Bindings (`console`, Stack Sourcemapping, Lazy Builtins/Preludes,
//! Fast SHA-256/HMAC-SHA256, Base64/UTF-8, `process.env`, Event-Loop Timers, `process` Control).

mod env_timers_process;
mod fast_codecs_and_hash;
mod runtime_and_loaders;

pub(crate) fn register_slice_native_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::js_string;
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_console_log"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    runtime_and_loaders::register_runtime_and_loader_host_functions(ctx);
    crate::bindings::deterministic_and_os::register_core_os_host_functions(ctx);
    fast_codecs_and_hash::register_fast_codecs_and_hash_host_functions(ctx);
    env_timers_process::register_env_timers_process_host_functions(ctx);
}
