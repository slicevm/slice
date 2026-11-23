//! WinterTC Phase N3 (`btoa`, `atob`, `TextEncoder.encodeInto`, `TextDecoder`, `zlib`),
//! RFC 7578 `multipart/form-data` & `structuredClone` Helpers, and `Buffer` / `URL` Codec Bindings.

mod buffer_and_url;
mod encoding_and_zlib;
mod multipart_and_clone;

pub(crate) use self::buffer_and_url::{
    register_buffer_host_functions, register_url_and_usp_host_functions,
};

pub(crate) fn register_wintertc_phase_n3_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::js_string;
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_btoa"), ctx)
        .unwrap_or(false)
    {
        return;
    }
    encoding_and_zlib::register_encoding_and_zlib_host_functions(ctx);
    multipart_and_clone::register_multipart_and_clone_host_functions(ctx);
}
