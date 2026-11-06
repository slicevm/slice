//! Boa's implementation of ECMAScript's Scripts.
//!
//! This module contains the [`Script`] type, which represents a [**Script Record**][script].
//!
//! More information:
//!  - [ECMAScript reference][spec]
//!
//! [spec]: https://tc39.es/ecma262/#sec-scripts
//! [script]: https://tc39.es/ecma262/#sec-script-records

use std::path::{Path, PathBuf};

use rustc_hash::FxHashMap;

use boa_gc::{Finalize, Gc, GcRefCell, Trace};
use boa_parser::{Parser, Source, source::ReadChar};

use crate::{
    Context, HostDefined, JsResult, JsString, JsValue, Module, SpannedSourceText,
    bytecompiler::{ByteCompiler, global_declaration_instantiation_context},
    environments::EnvironmentStack,
    js_string,
    realm::Realm,
    spanned_source_text::SourceText,
    vm::{ActiveRunnable, CallFrame, CallFrameFlags, CodeBlock},
};

/// ECMAScript's [**Script Record**][spec].
///
/// [spec]: https://tc39.es/ecma262/#sec-script-records
#[derive(Clone, Trace, Finalize)]
pub struct Script {
    inner: Gc<Inner>,
}

impl std::fmt::Debug for Script {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Script")
            .field("realm", &self.inner.realm.addr())
            .field("phase", &self.inner.phase.borrow())
            .field("loaded_modules", &self.inner.loaded_modules)
            .finish()
    }
}

#[derive(Trace, Debug, Finalize)]
enum ScriptPhase {
    Ast(#[unsafe_ignore_trace] boa_ast::Script),
    Codeblock(Gc<CodeBlock>),
}

#[derive(Trace, Finalize)]
struct Inner {
    realm: Realm,
    phase: GcRefCell<ScriptPhase>,
    source_text: SourceText,
    loaded_modules: GcRefCell<FxHashMap<JsString, Module>>,
    host_defined: HostDefined,
    path: Option<PathBuf>,
}

impl Script {
    /// Gets the realm of this script.
    #[must_use]
    pub fn realm(&self) -> &Realm {
        &self.inner.realm
    }

    /// Returns the [`ECMAScript specification`][spec] defined [`\[\[HostDefined\]\]`][`HostDefined`] field of the [`Module`].
    ///
    /// [spec]: https://tc39.es/ecma262/#script-record
    #[must_use]
    pub fn host_defined(&self) -> &HostDefined {
        &self.inner.host_defined
    }

    /// Gets the loaded modules of this script.
    pub(crate) fn loaded_modules(&self) -> &GcRefCell<FxHashMap<JsString, Module>> {
        &self.inner.loaded_modules
    }

    /// Abstract operation [`ParseScript ( sourceText, realm, hostDefined )`][spec].
    ///
    /// Parses the provided `src` as an ECMAScript script, returning an error if parsing fails.
    ///
    /// [spec]: https://tc39.es/ecma262/#sec-parse-script
    pub fn parse<R: ReadChar>(
        src: Source<'_, R>,
        realm: Option<Realm>,
        context: &mut Context,
    ) -> JsResult<Self> {
        let path = src.path().map(Path::to_path_buf);
        let mut parser = Parser::new(src);
        parser.set_identifier(context.next_parser_identifier());
        if context.is_strict() {
            parser.set_strict();
        }
        let scope = context.realm().scope().clone();
        let (mut code, source) = parser.parse_script_with_source(&scope, context.interner_mut())?;
        if !context.optimizer_options().is_empty() {
            context.optimize_statement_list(code.statements_mut());
        }

        let source_text = SourceText::new(source);

        Ok(Self {
            inner: Gc::new(Inner {
                realm: realm.unwrap_or_else(|| context.realm().clone()),
                phase: GcRefCell::new(ScriptPhase::Ast(code)),
                source_text,
                loaded_modules: GcRefCell::default(),
                host_defined: HostDefined::default(),
                path,
            }),
        })
    }

    /// Compiles the codeblock of this script.
    ///
    /// This is a no-op if this has been called previously.
    pub fn codeblock(&self, context: &mut Context) -> JsResult<Gc<CodeBlock>> {
        let cb = {
            let phase = self.inner.phase.borrow();
            let source = match &*phase {
                ScriptPhase::Codeblock(codeblock) => return Ok(codeblock.clone()),
                ScriptPhase::Ast(source) => source,
            };

            let mut annex_b_function_names = Vec::new();

            global_declaration_instantiation_context(
                &mut annex_b_function_names,
                source,
                self.inner.realm.scope(),
                context,
            )?;

            let spanned_source_text = SpannedSourceText::new_source_only(self.get_source());

            let mut compiler = ByteCompiler::new(
                js_string!("<main>"),
                source.strict(),
                false,
                self.inner.realm.scope().clone(),
                self.inner.realm.scope().clone(),
                false,
                false,
                context.interner_mut(),
                false,
                spanned_source_text,
                self.path().map(Path::to_owned).into(),
            );

            #[cfg(feature = "annex-b")]
            {
                compiler.annex_b_function_names = annex_b_function_names;
            }

            compiler.global_declaration_instantiation(source);
            compiler.compile_statement_list(source.statements(), true, false);

            Gc::new(compiler.finish())
        };

        *self.inner.phase.borrow_mut() = ScriptPhase::Codeblock(cb.clone());

        Ok(cb)
    }

    /// Evaluates this script and returns its result.
    ///
    /// Note that this won't run any scheduled promise jobs; you need to call [`Context::run_jobs`]
    /// on the context or [`JobExecutor::run_jobs`] on the provided queue to run them.
    ///
    /// [`JobExecutor::run_jobs`]: crate::job::JobExecutor::run_jobs
    pub fn evaluate(&self, context: &mut Context) -> JsResult<JsValue> {
        self.prepare_run(context)?;
        let record = context.run();

        context.vm.pop_frame();

        record.consume()
    }

    /// Evaluates this script and returns its result, periodically yielding to the executor
    /// in order to avoid blocking the current thread.
    ///
    /// This uses an implementation defined amount of "clock cycles" that need to pass before
    /// execution is suspended. See [`Script::evaluate_async_with_budget`] if you want to also
    /// customize this parameter.
    #[allow(clippy::future_not_send)]
    pub async fn evaluate_async(&self, context: &mut Context) -> JsResult<JsValue> {
        self.evaluate_async_with_budget(context, 256).await
    }

    /// Evaluates this script and returns its result, yielding to the executor each time `budget`
    /// number of "clock cycles" pass.
    ///
    /// Note that "clock cycle" is in quotation marks because we can't determine exactly how many
    /// CPU clock cycles a VM instruction will take, but all instructions have a "cost" associated
    /// with them that depends on their individual complexity. We'd recommend benchmarking with
    /// different budget sizes in order to find the ideal yielding time for your application.
    #[allow(clippy::future_not_send)]
    pub async fn evaluate_async_with_budget(
        &self,
        context: &mut Context,
        budget: u32,
    ) -> JsResult<JsValue> {
        self.prepare_run(context)?;

        let record = context.run_async_with_budget(budget).await;

        context.vm.pop_frame();

        record.consume()
    }

    fn prepare_run(&self, context: &mut Context) -> JsResult<()> {
        let codeblock = self.codeblock(context)?;

        let global_env = EnvironmentStack::new();
        context.vm.push_frame_with_stack(
            CallFrame::new(
                codeblock.clone(),
                Some(ActiveRunnable::Script(self.clone())),
                global_env,
                self.inner.realm.clone(),
            )
            .with_env_fp(0)
            .with_flags(CallFrameFlags::EXIT_EARLY),
            JsValue::undefined(),
            JsValue::null(),
        );

        self.realm().resize_global_env();

        context
            .global_declaration_instantiation(&codeblock)
            .inspect_err(|_| {
                context.vm.pop_frame();
            })?;

        Ok(())
    }

    pub(super) fn path(&self) -> Option<&Path> {
        self.inner.path.as_deref()
    }

    pub(super) fn get_source(&self) -> SourceText {
        self.inner.source_text.clone()
    }

    /// Compiles this script into a `CodeBlock` and serializes the entire `CodeBlock` tree
    /// (including nested function `CodeBlock`s, constants, `BindingLocator`s, exception `Handler`s,
    /// and `InlineCache` descriptors) plus the global scope binding count into a binary buffer.
    pub fn compile_to_slicebc(&self, context: &mut Context) -> JsResult<Vec<u8>> {
        let cb = self.codeblock(context)?;
        let global_bindings = self.inner.realm.scope().num_bindings();
        let mut out = Vec::with_capacity(16_384);
        out.extend_from_slice(&global_bindings.to_le_bytes());
        encode_codeblock(&cb, &mut out);
        Ok(out)
    }

    /// Deserializes a precompiled `CodeBlock` tree produced by [`Script::compile_to_slicebc`]
    /// directly into a ready-to-evaluate [`Script`] in `ScriptPhase::Codeblock(cb)`, bypassing
    /// `boa_lexer`, `boa_parser`, `Scope::analyze_scope`, and `ByteCompiler` at runtime.
    #[must_use]
    pub fn from_slicebc_bytes(bytes: &[u8], context: &mut Context) -> Option<Self> {
        let mut pos = 0usize;
        let global_bindings = read_u32(bytes, &mut pos)?;
        let empty_global_scope = boa_ast::scope::Scope::new_global();
        let mut scope_cache: Vec<Option<boa_ast::scope::Scope>> = Vec::new();
        let cb = decode_codeblock(bytes, &mut pos, &empty_global_scope, &mut scope_cache)?;

        let realm = context.realm().clone();
        let realm_scope = realm.scope();
        for &lex_idx in &cb.global_lexs {
            if let Some(crate::vm::Constant::String(name)) = cb.constants.get(lex_idx as usize) {
                drop(realm_scope.create_mutable_binding(name.clone(), false));
            }
        }
        while realm_scope.num_bindings() < global_bindings {
            let dummy = JsString::from(format!("__slice_g{}", realm_scope.num_bindings()));
            drop(realm_scope.create_mutable_binding(dummy, false));
        }

        Some(Self {
            inner: Gc::new(Inner {
                realm,
                phase: GcRefCell::new(ScriptPhase::Codeblock(cb)),
                source_text: SourceText::default(),
                loaded_modules: GcRefCell::default(),
                host_defined: HostDefined::default(),
                path: None,
            }),
        })
    }
}

#[inline]
fn write_u32(dst: &mut Vec<u8>, val: u32) {
    dst.extend_from_slice(&val.to_le_bytes());
}

#[inline]
fn write_js_string(dst: &mut Vec<u8>, s: &JsString) {
    match s.as_str().variant() {
        boa_string::JsStrVariant::Latin1(bytes) => {
            dst.push(0u8);
            write_u32(dst, bytes.len() as u32);
            dst.extend_from_slice(bytes);
        }
        boa_string::JsStrVariant::Utf16(units) => {
            dst.push(1u8);
            write_u32(dst, units.len() as u32);
            for &u in units {
                dst.extend_from_slice(&u.to_le_bytes());
            }
        }
    }
}

fn encode_codeblock(cb: &CodeBlock, dst: &mut Vec<u8>) {
    dst.extend_from_slice(&cb.flags.get().bits().to_le_bytes());
    write_u32(dst, cb.length);
    write_u32(dst, cb.parameter_length);
    write_u32(dst, cb.register_count);
    let this_mode_tag = match cb.this_mode {
        crate::builtins::function::ThisMode::Lexical => 0u8,
        crate::builtins::function::ThisMode::Strict => 1u8,
        crate::builtins::function::ThisMode::Global => 2u8,
    };
    dst.push(this_mode_tag);

    write_u32(dst, cb.mapped_arguments_binding_indices.len() as u32);
    for &opt_idx in &cb.mapped_arguments_binding_indices {
        write_u32(dst, opt_idx.unwrap_or(u32::MAX));
    }

    write_u32(dst, cb.bytecode.bytes.len() as u32);
    dst.extend_from_slice(&cb.bytecode.bytes);

    write_u32(dst, cb.constants.len() as u32);
    for c in &cb.constants {
        match c {
            crate::vm::Constant::String(s) => {
                dst.push(0u8);
                write_js_string(dst, s);
            }
            crate::vm::Constant::Function(child) => {
                dst.push(1u8);
                encode_codeblock(child, dst);
            }
            crate::vm::Constant::BigInt(bi) => {
                dst.push(2u8);
                let s = bi.to_string_radix(10);
                write_u32(dst, s.len() as u32);
                dst.extend_from_slice(s.as_bytes());
            }
            crate::vm::Constant::Scope(sc) => {
                dst.push(3u8);
                write_u32(dst, sc.num_bindings_non_local());
            }
        }
    }

    write_u32(dst, cb.bindings.len() as u32);
    for b in &cb.bindings {
        write_js_string(dst, b.name());
        match b.scope() {
            boa_ast::scope::BindingLocatorScope::GlobalObject => {
                dst.push(0u8);
                write_u32(dst, 0);
            }
            boa_ast::scope::BindingLocatorScope::GlobalDeclarative => {
                dst.push(1u8);
                write_u32(dst, 0);
            }
            boa_ast::scope::BindingLocatorScope::Stack(idx) => {
                dst.push(2u8);
                write_u32(dst, idx);
            }
        }
        write_u32(dst, b.binding_index());
    }

    write_u32(dst, cb.handlers.len() as u32);
    for h in &cb.handlers {
        write_u32(dst, h.start.as_u32());
        write_u32(dst, h.end.as_u32());
        write_u32(dst, h.environment_count);
    }

    write_u32(dst, cb.ic.len() as u32);
    for ic in &cb.ic {
        write_js_string(dst, &ic.name);
    }

    write_js_string(dst, cb.name());

    write_u32(dst, cb.global_lexs.len() as u32);
    for &idx in &cb.global_lexs {
        write_u32(dst, idx);
    }

    write_u32(dst, cb.global_fns.len() as u32);
    for gf in &cb.global_fns {
        write_u32(dst, gf.name_index);
        write_u32(dst, gf.function_index);
    }

    write_u32(dst, cb.global_vars.len() as u32);
    for &idx in &cb.global_vars {
        write_u32(dst, idx);
    }
}

#[inline]
fn read_u8(src: &[u8], pos: &mut usize) -> Option<u8> {
    let b = *src.get(*pos)?;
    *pos += 1;
    Some(b)
}

#[inline]
fn read_u16(src: &[u8], pos: &mut usize) -> Option<u16> {
    let end = pos.checked_add(2)?;
    let slice = src.get(*pos..end)?;
    *pos = end;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

#[inline]
fn read_u32(src: &[u8], pos: &mut usize) -> Option<u32> {
    let end = pos.checked_add(4)?;
    let slice = src.get(*pos..end)?;
    *pos = end;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[inline]
fn read_bytes<'a>(src: &'a [u8], pos: &mut usize, len: usize) -> Option<&'a [u8]> {
    let end = pos.checked_add(len)?;
    let slice = src.get(*pos..end)?;
    *pos = end;
    Some(slice)
}

#[inline]
fn read_js_string(src: &[u8], pos: &mut usize) -> Option<JsString> {
    let kind = read_u8(src, pos)?;
    let len = read_u32(src, pos)? as usize;
    if kind == 0 {
        let bytes = read_bytes(src, pos, len)?;
        Some(JsString::from(boa_string::JsStr::latin1(bytes)))
    } else {
        let byte_len = len.checked_mul(2)?;
        let raw = read_bytes(src, pos, byte_len)?;
        let mut units = Vec::with_capacity(len);
        for chunk in raw.chunks_exact(2) {
            units.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }
        Some(JsString::from(units.as_slice()))
    }
}

fn get_or_build_scope_with_len(
    n: u32,
    empty_global_scope: &boa_ast::scope::Scope,
    cache: &mut Vec<Option<boa_ast::scope::Scope>>,
) -> boa_ast::scope::Scope {
    if n == 0 {
        return empty_global_scope.clone();
    }
    let idx = n as usize;
    if idx < cache.len() {
        if let Some(s) = &cache[idx] {
            return s.clone();
        }
    } else {
        cache.resize(idx + 1, None);
    }
    let s = boa_ast::scope::Scope::new_global();
    for i in 0..n {
        let name = JsString::from(format!("b{i}"));
        drop(s.create_mutable_binding(name, true));
    }
    cache[idx] = Some(s.clone());
    s
}

fn decode_codeblock(
    src: &[u8],
    pos: &mut usize,
    empty_global_scope: &boa_ast::scope::Scope,
    scope_cache: &mut Vec<Option<boa_ast::scope::Scope>>,
) -> Option<Gc<CodeBlock>> {
    let flags_bits = read_u16(src, pos)?;
    let flags = crate::vm::CodeBlockFlags::from_bits_retain(flags_bits);
    let length = read_u32(src, pos)?;
    let parameter_length = read_u32(src, pos)?;
    let register_count = read_u32(src, pos)?;
    let this_mode = match read_u8(src, pos)? {
        0 => crate::builtins::function::ThisMode::Lexical,
        1 => crate::builtins::function::ThisMode::Strict,
        _ => crate::builtins::function::ThisMode::Global,
    };

    let mapped_len = read_u32(src, pos)? as usize;
    let mut mapped_arguments_binding_indices = thin_vec::ThinVec::with_capacity(mapped_len);
    for _ in 0..mapped_len {
        let v = read_u32(src, pos)?;
        mapped_arguments_binding_indices.push(if v == u32::MAX { None } else { Some(v) });
    }

    let bc_len = read_u32(src, pos)? as usize;
    let bc_bytes = read_bytes(src, pos, bc_len)?;
    let bytecode = crate::vm::opcode::Bytecode {
        bytes: bc_bytes.to_vec().into_boxed_slice(),
    };

    let const_len = read_u32(src, pos)? as usize;
    let mut constants = thin_vec::ThinVec::with_capacity(const_len);
    for _ in 0..const_len {
        let tag = read_u8(src, pos)?;
        let c = match tag {
            0 => crate::vm::Constant::String(read_js_string(src, pos)?),
            1 => crate::vm::Constant::Function(decode_codeblock(
                src,
                pos,
                empty_global_scope,
                scope_cache,
            )?),
            2 => {
                let slen = read_u32(src, pos)? as usize;
                let sbytes = read_bytes(src, pos, slen)?;
                let s = std::str::from_utf8(sbytes).ok()?;
                let bi = crate::JsBigInt::from_string(s)?;
                crate::vm::Constant::BigInt(bi)
            }
            _ => {
                let n = read_u32(src, pos)?;
                crate::vm::Constant::Scope(get_or_build_scope_with_len(
                    n,
                    empty_global_scope,
                    scope_cache,
                ))
            }
        };
        constants.push(c);
    }

    let bind_len = read_u32(src, pos)? as usize;
    let mut bindings_vec = Vec::with_capacity(bind_len);
    for _ in 0..bind_len {
        let name = read_js_string(src, pos)?;
        let scope_kind = read_u8(src, pos)?;
        let stack_idx = read_u32(src, pos)?;
        let binding_index = read_u32(src, pos)?;
        let mut loc = empty_global_scope.get_identifier_reference(name).locator();
        let bscope = match scope_kind {
            0 => boa_ast::scope::BindingLocatorScope::GlobalObject,
            1 => boa_ast::scope::BindingLocatorScope::GlobalDeclarative,
            _ => boa_ast::scope::BindingLocatorScope::Stack(stack_idx),
        };
        loc.set_scope(bscope);
        loc.set_binding_index(binding_index);
        bindings_vec.push(loc);
    }

    let handler_len = read_u32(src, pos)? as usize;
    let mut handlers = thin_vec::ThinVec::with_capacity(handler_len);
    for _ in 0..handler_len {
        let start = crate::vm::opcode::Address::new(read_u32(src, pos)?);
        let end = crate::vm::opcode::Address::new(read_u32(src, pos)?);
        let environment_count = read_u32(src, pos)?;
        handlers.push(crate::vm::Handler {
            start,
            end,
            environment_count,
        });
    }

    let ic_len = read_u32(src, pos)? as usize;
    let mut ic_vec = Vec::with_capacity(ic_len);
    for _ in 0..ic_len {
        let name = read_js_string(src, pos)?;
        ic_vec.push(crate::vm::InlineCache::new(name));
    }

    let fn_name = read_js_string(src, pos)?;
    let source_info = crate::vm::source_info::SourceInfo::new(
        crate::vm::source_info::SourceMap::new(
            Box::default(),
            crate::vm::source_info::SourcePath::None,
        ),
        fn_name,
        SpannedSourceText::new_empty(),
    );

    let glex_len = read_u32(src, pos)? as usize;
    let mut glex_vec = Vec::with_capacity(glex_len);
    for _ in 0..glex_len {
        glex_vec.push(read_u32(src, pos)?);
    }

    let gfn_len = read_u32(src, pos)? as usize;
    let mut gfn_vec = Vec::with_capacity(gfn_len);
    for _ in 0..gfn_len {
        let name_index = read_u32(src, pos)?;
        let function_index = read_u32(src, pos)?;
        gfn_vec.push(crate::vm::GlobalFunctionBinding {
            name_index,
            function_index,
        });
    }

    let gvar_len = read_u32(src, pos)? as usize;
    let mut gvar_vec = Vec::with_capacity(gvar_len);
    for _ in 0..gvar_len {
        gvar_vec.push(read_u32(src, pos)?);
    }

    Some(Gc::new(CodeBlock {
        flags: std::cell::Cell::new(flags),
        length,
        parameter_length,
        register_count,
        this_mode,
        mapped_arguments_binding_indices,
        bytecode,
        constants,
        bindings: bindings_vec.into_boxed_slice(),
        handlers,
        ic: ic_vec.into_boxed_slice(),
        source_info,
        global_lexs: glex_vec.into_boxed_slice(),
        global_fns: gfn_vec.into_boxed_slice(),
        global_vars: gvar_vec.into_boxed_slice(),
        debug_id: CodeBlock::get_next_codeblock_id(),
        #[cfg(feature = "trace")]
        traced: std::cell::Cell::new(false),
    }))
}
