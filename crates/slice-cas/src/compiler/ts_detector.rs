//! Zero-Allocation Oxc AST TypeScript & JSX/TSX Construct Detector (`program_has_ts_or_jsx`).
//!
//! Determines in a single short-circuiting arena pass whether an already-parsed Oxc `Program`
//! contains any TypeScript type annotations, declarations, modifiers, decorators, auto-accessors,
//! `using` declarations, or JSX elements/fragments that require `oxc::transformer::Transformer`
//! and `oxc::codegen::Codegen` lowering before ECMAScript execution.

use oxc::ast::ast::{
    AccessorProperty, ArrowFunctionExpression, CallExpression, CatchParameter, Class,
    ClassHeritage, Decorator, ExportAllDeclaration, ExportFromDeclaration, ExportNamedDeclaration,
    ExportSpecifier, Expression, FormalParameter, Function, FunctionType, ImportDeclaration,
    ImportSpecifier, JSXElement, JSXFragment, MethodDefinition, MethodDefinitionType,
    NewExpression, Program, PropertyDefinition, PropertyDefinitionType, Statement, TSAsExpression,
    TSClassImplements, TSEnumDeclaration, TSExportAssignment, TSExternalModuleDeclaration,
    TSGlobalDeclaration, TSImportEqualsDeclaration, TSIndexSignature, TSInstantiationExpression,
    TSInterfaceDeclaration, TSNamespaceDeclaration, TSNamespaceExportDeclaration,
    TSNonNullExpression, TSSatisfiesExpression, TSSignature, TSThisParameter, TSType,
    TSTypeAliasDeclaration, TSTypeAnnotation, TSTypeAssertion, TSTypeParameterDeclaration,
    TSTypeParameterInstantiation, TaggedTemplateExpression, VariableDeclaration,
    VariableDeclarationKind, VariableDeclarator,
};
use oxc::ast_visit::{Visit, walk};
use oxc::syntax::scope::ScopeFlags;

struct TsOrJsxAstDetector {
    found: bool,
}

impl<'a> Visit<'a> for TsOrJsxAstDetector {
    #[inline]
    fn visit_statement(&mut self, it: &Statement<'a>) {
        if !self.found {
            walk::walk_statement(self, it);
        }
    }

    #[inline]
    fn visit_expression(&mut self, it: &Expression<'a>) {
        if !self.found {
            walk::walk_expression(self, it);
        }
    }

    #[inline]
    fn visit_jsx_element(&mut self, _it: &JSXElement<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_jsx_fragment(&mut self, _it: &JSXFragment<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_decorator(&mut self, _it: &Decorator<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_accessor_property(&mut self, _it: &AccessorProperty<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_type(&mut self, _it: &TSType<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_type_annotation(&mut self, _it: &TSTypeAnnotation<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_type_parameter_declaration(&mut self, _it: &TSTypeParameterDeclaration<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_type_parameter_instantiation(&mut self, _it: &TSTypeParameterInstantiation<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_this_parameter(&mut self, _it: &TSThisParameter<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_enum_declaration(&mut self, _it: &TSEnumDeclaration<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_interface_declaration(&mut self, _it: &TSInterfaceDeclaration<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_type_alias_declaration(&mut self, _it: &TSTypeAliasDeclaration<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_class_implements(&mut self, _it: &TSClassImplements<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_signature(&mut self, _it: &TSSignature<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_index_signature(&mut self, _it: &TSIndexSignature<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_external_module_declaration(&mut self, _it: &TSExternalModuleDeclaration<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_namespace_declaration(&mut self, _it: &TSNamespaceDeclaration<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_global_declaration(&mut self, _it: &TSGlobalDeclaration<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_import_equals_declaration(&mut self, _it: &TSImportEqualsDeclaration<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_export_assignment(&mut self, _it: &TSExportAssignment<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_namespace_export_declaration(&mut self, _it: &TSNamespaceExportDeclaration<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_as_expression(&mut self, _it: &TSAsExpression<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_satisfies_expression(&mut self, _it: &TSSatisfiesExpression<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_type_assertion(&mut self, _it: &TSTypeAssertion<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_non_null_expression(&mut self, _it: &TSNonNullExpression<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_ts_instantiation_expression(&mut self, _it: &TSInstantiationExpression<'a>) {
        self.found = true;
    }

    #[inline]
    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        if it.declare
            || matches!(
                it.kind,
                VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
            )
        {
            self.found = true;
            return;
        }
        walk::walk_variable_declaration(self, it);
    }

    #[inline]
    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if it.definite || it.type_annotation.is_some() {
            self.found = true;
            return;
        }
        walk::walk_variable_declarator(self, it);
    }

    #[inline]
    fn visit_catch_parameter(&mut self, it: &CatchParameter<'a>) {
        if it.type_annotation.is_some() {
            self.found = true;
            return;
        }
        walk::walk_catch_parameter(self, it);
    }

    #[inline]
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        if it.declare
            || it.body.is_none()
            || matches!(
                it.r#type,
                FunctionType::TSDeclareFunction | FunctionType::TSEmptyBodyFunctionExpression
            )
            || it.type_parameters.is_some()
            || it.this_param.is_some()
            || it.return_type.is_some()
        {
            self.found = true;
            return;
        }
        walk::walk_function(self, it, flags);
    }

    #[inline]
    fn visit_formal_parameter(&mut self, it: &FormalParameter<'a>) {
        if it.optional
            || it.accessibility.is_some()
            || it.readonly
            || it.r#override
            || it.type_annotation.is_some()
            || !it.decorators.is_empty()
        {
            self.found = true;
            return;
        }
        walk::walk_formal_parameter(self, it);
    }

    #[inline]
    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        if it.type_parameters.is_some() || it.return_type.is_some() {
            self.found = true;
            return;
        }
        walk::walk_arrow_function_expression(self, it);
    }

    #[inline]
    fn visit_class(&mut self, it: &Class<'a>) {
        if it.r#abstract
            || it.declare
            || it.type_parameters.is_some()
            || !it.implements.is_empty()
            || !it.decorators.is_empty()
        {
            self.found = true;
            return;
        }
        walk::walk_class(self, it);
    }

    #[inline]
    fn visit_class_heritage(&mut self, it: &ClassHeritage<'a>) {
        if it.type_arguments.is_some() {
            self.found = true;
            return;
        }
        walk::walk_class_heritage(self, it);
    }

    #[inline]
    fn visit_method_definition(&mut self, it: &MethodDefinition<'a>) {
        if it.r#type == MethodDefinitionType::TSAbstractMethodDefinition
            || it.r#override
            || it.optional
            || it.accessibility.is_some()
            || !it.decorators.is_empty()
        {
            self.found = true;
            return;
        }
        walk::walk_method_definition(self, it);
    }

    #[inline]
    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        if it.r#type == PropertyDefinitionType::TSAbstractPropertyDefinition
            || it.declare
            || it.r#override
            || it.optional
            || it.definite
            || it.readonly
            || it.accessibility.is_some()
            || it.type_annotation.is_some()
            || !it.decorators.is_empty()
        {
            self.found = true;
            return;
        }
        walk::walk_property_definition(self, it);
    }

    #[inline]
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if it.type_arguments.is_some() {
            self.found = true;
            return;
        }
        walk::walk_call_expression(self, it);
    }

    #[inline]
    fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
        if it.type_arguments.is_some() {
            self.found = true;
            return;
        }
        walk::walk_new_expression(self, it);
    }

    #[inline]
    fn visit_tagged_template_expression(&mut self, it: &TaggedTemplateExpression<'a>) {
        if it.type_arguments.is_some() {
            self.found = true;
            return;
        }
        walk::walk_tagged_template_expression(self, it);
    }

    #[inline]
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        if it.import_kind.is_type() {
            self.found = true;
            return;
        }
        walk::walk_import_declaration(self, it);
    }

    #[inline]
    fn visit_import_specifier(&mut self, it: &ImportSpecifier<'a>) {
        if it.import_kind.is_type() {
            self.found = true;
            return;
        }
        walk::walk_import_specifier(self, it);
    }

    #[inline]
    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        if it.export_kind.is_type() {
            self.found = true;
            return;
        }
        walk::walk_export_named_declaration(self, it);
    }

    #[inline]
    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
        if it.export_kind.is_type() {
            self.found = true;
            return;
        }
        walk::walk_export_from_declaration(self, it);
    }

    #[inline]
    fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
        if it.export_kind.is_type() {
            self.found = true;
            return;
        }
        walk::walk_export_all_declaration(self, it);
    }

    #[inline]
    fn visit_export_specifier(&mut self, it: &ExportSpecifier<'a>) {
        if it.export_kind.is_type() {
            self.found = true;
            return;
        }
        walk::walk_export_specifier(self, it);
    }
}

/// Returns `true` if `program` contains any TypeScript or JSX AST construct requiring
/// Oxc `Transformer` + `Codegen` lowering. Short-circuits on the first match with zero heap allocations.
#[inline]
pub fn program_has_ts_or_jsx<'a>(program: &Program<'a>) -> bool {
    let mut detector = TsOrJsxAstDetector { found: false };
    detector.visit_program(program);
    detector.found
}
