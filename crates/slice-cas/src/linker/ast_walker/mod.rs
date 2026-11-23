//! Oxc AST Statement/Expression Walkers, Top-Level `await` Detector, and Span Slicer.
//!
//! Organizes AST walking and scoped identifier replacement into focused submodules:
//! - [`expr_visitor`]: `walk_ast_expressions` and `walk_ast_call_expressions`.
//! - [`top_level_await`]: `has_top_level_await` and `wrap_top_level_await_script`.
//! - [`mutations_and_edits`]: `slice_with_edits` and `collect_mutated_binding_names`.
//! - [`scoped_ident_walker`]: Lexical-shadowing-aware identifier reference edit collectors.

pub mod expr_visitor;
pub mod mutations_and_edits;
pub mod scoped_ident_walker;
pub mod top_level_await;

pub use expr_visitor::{walk_ast_call_expressions, walk_ast_expressions};
pub use mutations_and_edits::{collect_mutated_binding_names, slice_with_edits};
pub use scoped_ident_walker::{
    collect_expr_identifier_edits, collect_live_import_reference_edits,
    collect_top_level_cjs_reference_edits,
};
pub use top_level_await::{has_top_level_await, wrap_top_level_await_script};

pub fn collect_binding_pattern_names(
    pat: &oxc::ast::ast::BindingPattern<'_>,
    out: &mut Vec<(String, String)>,
) {
    use oxc::ast::ast::BindingPattern;
    match pat {
        BindingPattern::BindingIdentifier(ident) => {
            let n = ident.name.to_string();
            out.push((n.clone(), n));
        }
        BindingPattern::ObjectPattern(obj) => {
            for prop in &obj.properties {
                collect_binding_pattern_names(&prop.value, out);
            }
            if let Some(rest) = &obj.rest {
                collect_binding_pattern_names(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(arr) => {
            for elem in arr.elements.iter().flatten() {
                collect_binding_pattern_names(elem, out);
            }
            if let Some(rest) = &arr.rest {
                collect_binding_pattern_names(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(assign) => {
            collect_binding_pattern_names(&assign.left, out);
        }
    }
}
