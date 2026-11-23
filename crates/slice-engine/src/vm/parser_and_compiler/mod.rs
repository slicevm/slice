//! Recursive-Descent ECMAScript/TypeScript Lexer, Parser, `SLICESNP`/`SLICEBOA` Compiler, and Disassembler.

mod compiler;
mod disassembler;
mod lexer;
mod parser_expr;
mod parser_stmt;
mod tier0_ast_gate;
mod tier0_scope_and_hazards;

pub(crate) use crate::slicesnp_codec::decode_slicesnp_cached;
pub use compiler::{SLICEERR_MAGIC, compile_source_to_bytecode};
pub(crate) use compiler::{
    compact_js_for_boa_bytecode, compile_source_for_sandbox_eval, extract_slice_rc_key,
};
pub use disassembler::disassemble_bytecode;
pub(crate) use parser_expr::expr_to_dotted_chain;
pub use tier0_ast_gate::is_tier0_slicesnp_compatible_ast;
