use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{Delimiter, Group, LineColumn, TokenStream, TokenTree};
use syn::parse::Parser;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Attribute, Item, ItemMod, ItemUse, Stmt, Token, punctuated::Punctuated};

use super::super::cfg_context::{CfgContext, CfgDecision};
use super::{
    macro_safety::{MacroDefinitionScope, local_macro_safety},
    macro_syntax::{
        is_dependency_neutral_expression_macro, is_dependency_neutral_token_macro,
        is_dependency_observing_macro, is_json_macro, is_matches_macro,
        resolves_to_standard_thread_local, thread_local_declarations,
    },
    module_index::{
        UseAlias, collect_generic_bindings, collect_item_generic_bindings, collect_use_aliases,
        collect_use_bindings, collect_use_glob_imports, declared_item_name, item_attributes,
    },
};

#[derive(Clone)]
pub(crate) struct ObservedPath {
    pub(crate) path: syn::Path,
    pub(crate) evidence_span: LineColumn,
    pub(crate) requires_complete_resolution: bool,
    pub(crate) local_bindings: BTreeSet<String>,
    pub(crate) aliases: BTreeMap<String, Vec<UseAlias>>,
    pub(crate) glob_imports: Vec<UseAlias>,
}

#[derive(Clone)]
pub(crate) struct ScopedUse {
    pub(crate) item: ItemUse,
    pub(crate) aliases: BTreeMap<String, Vec<UseAlias>>,
    pub(crate) glob_imports: Vec<UseAlias>,
}

fn known_builtin_attribute(path: &syn::Path) -> bool {
    path.get_ident().is_some_and(|name| {
        matches!(
            name.to_string().as_str(),
            "allow"
                | "cold"
                | "deprecated"
                | "default"
                | "deny"
                | "doc"
                | "export_name"
                | "forbid"
                | "global_allocator"
                | "ignore"
                | "inline"
                | "link"
                | "link_name"
                | "link_section"
                | "macro_export"
                | "macro_use"
                | "must_use"
                | "no_main"
                | "no_implicit_prelude"
                | "no_mangle"
                | "no_std"
                | "non_exhaustive"
                | "panic_handler"
                | "path"
                | "proc_macro"
                | "proc_macro_attribute"
                | "proc_macro_derive"
                | "repr"
                | "should_panic"
                | "test"
                | "warn"
        )
    })
}

pub(crate) struct PathVisitor<'a> {
    pub(crate) cfg_context: &'a CfgContext,
    pub(crate) external_crates: &'a BTreeSet<String>,
    pub(crate) declared_items: &'a BTreeSet<String>,
    /// macro_rules names reach child modules through textual scope, so any
    /// crate-wide definition may shadow a prelude macro of the same name.
    pub(crate) crate_macro_rules_names: &'a BTreeSet<String>,
    pub(crate) observe_attribute_paths: bool,
    pub(crate) paths: Vec<ObservedPath>,
    pub(crate) scoped_uses: Vec<ScopedUse>,
    pub(crate) local_bindings: BTreeSet<String>,
    pub(crate) local_item_bindings: BTreeSet<String>,
    pub(crate) aliases: BTreeMap<String, Vec<UseAlias>>,
    pub(crate) glob_imports: Vec<UseAlias>,
    pub(crate) macro_bindings: BTreeSet<String>,
    pub(crate) local_macro_bindings: BTreeSet<String>,
    pub(crate) safe_local_macro_bindings: BTreeSet<String>,
    pub(crate) observing_local_macro_bindings: BTreeSet<String>,
    pub(crate) inactive_expr_cfg: bool,
    pub(crate) in_expr: bool,
    pub(crate) in_item_macro: bool,
    pub(crate) cfg_guarded_macro: bool,
    pub(crate) no_implicit_prelude: bool,
    pub(crate) macro_spans: Vec<LineColumn>,
    pub(crate) cfg_spans: Vec<LineColumn>,
    pub(crate) strict_path_spans: Vec<LineColumn>,
}

impl<'ast> Visit<'ast> for PathVisitor<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        match self.cfg_context.evaluate_attributes(item_attributes(item)) {
            CfgDecision::Inactive => return,
            CfgDecision::Unknown => self.cfg_spans.push(item.span().start()),
            CfgDecision::Active => {}
        }
        let outer_bindings = self.local_bindings.clone();
        let outer_item_macro = self.in_item_macro;
        let outer_cfg_guarded_macro = self.cfg_guarded_macro;
        if let Item::Macro(item_macro) = item {
            self.in_item_macro = !item_macro.mac.path.is_ident("macro_rules");
            self.cfg_guarded_macro = has_cfg_attribute(&item_macro.attrs);
        }
        collect_item_generic_bindings(item, &mut self.local_bindings);
        syn::visit::visit_item(self, item);
        self.local_bindings = outer_bindings;
        self.in_item_macro = outer_item_macro;
        self.cfg_guarded_macro = outer_cfg_guarded_macro;
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        match self.cfg_context.evaluate_attributes(&local.attrs) {
            CfgDecision::Inactive => return,
            CfgDecision::Unknown => self.cfg_spans.push(local.span().start()),
            CfgDecision::Active => {}
        }
        collect_pattern_bindings(&local.pat, &mut self.local_bindings);
        syn::visit::visit_local(self, local);
    }

    fn visit_field(&mut self, field: &'ast syn::Field) {
        match self.cfg_context.evaluate_attributes(&field.attrs) {
            CfgDecision::Inactive => return,
            CfgDecision::Unknown => self.cfg_spans.push(field.span().start()),
            CfgDecision::Active => {}
        }
        syn::visit::visit_field(self, field);
    }

    fn visit_visibility(&mut self, _visibility: &'ast syn::Visibility) {
        // Restricted visibility paths define access scope; they do not reference a dependency.
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if self.inactive_expr_cfg {
            return;
        }
        if path.segments.len() == 1
            && path
                .get_ident()
                .is_some_and(|name| self.macro_bindings.contains(&name.to_string()))
        {
            return;
        }
        self.paths.push(ObservedPath {
            path: path.clone(),
            evidence_span: path.span().start(),
            requires_complete_resolution: false,
            local_bindings: self.local_bindings.clone(),
            aliases: self.aliases.clone(),
            glob_imports: self.glob_imports.clone(),
        });
        syn::visit::visit_path(self, path);
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        let outer_bindings = self.local_bindings.clone();
        let outer_item_bindings = self.local_item_bindings.clone();
        let outer_aliases = self.aliases.clone();
        let outer_glob_imports = self.glob_imports.clone();
        let outer_macro_bindings = self.macro_bindings.clone();
        let outer_local_macro_bindings = self.local_macro_bindings.clone();
        let outer_safe_local_macro_bindings = self.safe_local_macro_bindings.clone();
        let outer_observing_local_macro_bindings = self.observing_local_macro_bindings.clone();
        let mut block_scope_bindings = self.local_bindings.clone();
        for statement in &block.stmts {
            if let Stmt::Local(local) = statement
                && self.cfg_context.evaluate_attributes(&local.attrs) != CfgDecision::Inactive
            {
                collect_pattern_bindings(&local.pat, &mut block_scope_bindings);
            }
            if let Stmt::Item(item) = statement
                && self.cfg_context.evaluate_attributes(item_attributes(item))
                    != CfgDecision::Inactive
            {
                if let Item::Use(item) = item {
                    collect_use_aliases(
                        &item.tree,
                        Vec::new(),
                        item.leading_colon.is_some(),
                        &mut self.aliases,
                    );
                    collect_use_glob_imports(
                        &item.tree,
                        Vec::new(),
                        item.leading_colon.is_some(),
                        &mut self.glob_imports,
                    );
                    collect_use_bindings(&item.tree, Vec::new(), &mut self.macro_bindings);
                }
                if let Item::Macro(item) = item
                    && let Some(ident) = &item.ident
                {
                    self.macro_bindings.insert(ident.to_string());
                    self.local_macro_bindings.insert(ident.to_string());
                    if let Some(observes_arguments) = local_macro_safety(
                        item,
                        &MacroDefinitionScope {
                            declared_items: &block_scope_bindings,
                            macro_bindings: &self.macro_bindings,
                            no_implicit_prelude: self.no_implicit_prelude,
                        },
                    ) {
                        self.safe_local_macro_bindings.insert(ident.to_string());
                        if observes_arguments {
                            self.observing_local_macro_bindings
                                .insert(ident.to_string());
                        } else {
                            self.observing_local_macro_bindings
                                .remove(&ident.to_string());
                        }
                    } else {
                        self.safe_local_macro_bindings.remove(&ident.to_string());
                        self.observing_local_macro_bindings
                            .remove(&ident.to_string());
                    }
                }
                if let Some(name) = declared_item_name(item) {
                    self.local_bindings.insert(name.clone());
                    self.local_item_bindings.insert(name.clone());
                    block_scope_bindings.insert(name);
                }
            }
        }
        for statement in &block.stmts {
            match statement {
                Stmt::Item(Item::Use(item)) => {
                    if self.cfg_context.evaluate_attributes(&item.attrs) != CfgDecision::Inactive {
                        self.scoped_uses.push(ScopedUse {
                            item: item.clone(),
                            aliases: self.aliases.clone(),
                            glob_imports: self.glob_imports.clone(),
                        });
                    }
                }
                Stmt::Local(local) => {
                    self.visit_local(local);
                    collect_pattern_bindings(&local.pat, &mut self.local_bindings);
                }
                _ => self.visit_stmt(statement),
            }
        }
        self.local_bindings = outer_bindings;
        self.local_item_bindings = outer_item_bindings;
        self.aliases = outer_aliases;
        self.glob_imports = outer_glob_imports;
        self.macro_bindings = outer_macro_bindings;
        self.local_macro_bindings = outer_local_macro_bindings;
        self.safe_local_macro_bindings = outer_safe_local_macro_bindings;
        self.observing_local_macro_bindings = outer_observing_local_macro_bindings;
    }

    fn visit_expr(&mut self, expression: &'ast syn::Expr) {
        if self.inactive_expr_cfg {
            return;
        }
        let outer_in_expr = self.in_expr;
        let outer_inactive = self.inactive_expr_cfg;
        self.in_expr = true;
        self.inactive_expr_cfg = false;
        syn::visit::visit_expr(self, expression);
        self.in_expr = outer_in_expr;
        self.inactive_expr_cfg = outer_inactive;
    }

    fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
        let outer_bindings = self.local_bindings.clone();
        for input in &closure.inputs {
            collect_pattern_bindings(input, &mut self.local_bindings);
        }
        syn::visit::visit_expr_closure(self, closure);
        self.local_bindings = outer_bindings;
    }

    fn visit_expr_for_loop(&mut self, expression: &'ast syn::ExprForLoop) {
        let outer_bindings = self.local_bindings.clone();
        collect_pattern_bindings(&expression.pat, &mut self.local_bindings);
        syn::visit::visit_expr_for_loop(self, expression);
        self.local_bindings = outer_bindings;
    }

    fn visit_expr_if(&mut self, expression: &'ast syn::ExprIf) {
        let outer_bindings = self.local_bindings.clone();
        if let syn::Expr::Let(condition) = &*expression.cond {
            collect_pattern_bindings(&condition.pat, &mut self.local_bindings);
        }
        syn::visit::visit_expr_if(self, expression);
        self.local_bindings = outer_bindings;
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        let outer_bindings = self.local_bindings.clone();
        collect_pattern_bindings(&arm.pat, &mut self.local_bindings);
        syn::visit::visit_arm(self, arm);
        self.local_bindings = outer_bindings;
    }

    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        let outer_cfg_guarded_macro = self.cfg_guarded_macro;
        if let Stmt::Macro(statement_macro) = statement {
            self.cfg_guarded_macro = has_cfg_attribute(&statement_macro.attrs);
            match self.cfg_context.evaluate_attributes(&statement_macro.attrs) {
                CfgDecision::Inactive => {
                    self.cfg_guarded_macro = outer_cfg_guarded_macro;
                    return;
                }
                CfgDecision::Unknown => self.cfg_spans.push(statement_macro.span().start()),
                CfgDecision::Active => {}
            }
        }
        syn::visit::visit_stmt(self, statement);
        self.cfg_guarded_macro = outer_cfg_guarded_macro;
    }

    fn visit_expr_macro(&mut self, expression: &'ast syn::ExprMacro) {
        let outer_cfg_guarded_macro = self.cfg_guarded_macro;
        self.cfg_guarded_macro = has_cfg_attribute(&expression.attrs);
        syn::visit::visit_expr_macro(self, expression);
        self.cfg_guarded_macro = outer_cfg_guarded_macro;
    }

    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        let outer_bindings = self.local_bindings.clone();
        collect_generic_bindings(&function.sig.generics, &mut self.local_bindings);
        for input in &function.sig.inputs {
            if let syn::FnArg::Typed(argument) = input {
                collect_pattern_bindings(&argument.pat, &mut self.local_bindings);
            }
        }
        for attribute in &function.attrs {
            self.visit_attribute(attribute);
        }
        self.visit_signature(&function.sig);
        self.visit_block(&function.block);
        self.local_bindings = outer_bindings;
    }

    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        match self.cfg_context.evaluate_attributes(&function.attrs) {
            CfgDecision::Inactive => return,
            CfgDecision::Unknown => self.cfg_spans.push(function.span().start()),
            CfgDecision::Active => {}
        }
        let outer_bindings = self.local_bindings.clone();
        collect_generic_bindings(&function.sig.generics, &mut self.local_bindings);
        for input in &function.sig.inputs {
            if let syn::FnArg::Typed(argument) = input {
                collect_pattern_bindings(&argument.pat, &mut self.local_bindings);
            }
        }
        syn::visit::visit_impl_item_fn(self, function);
        self.local_bindings = outer_bindings;
    }

    fn visit_trait_item_fn(&mut self, function: &'ast syn::TraitItemFn) {
        match self.cfg_context.evaluate_attributes(&function.attrs) {
            CfgDecision::Inactive => return,
            CfgDecision::Unknown => self.cfg_spans.push(function.span().start()),
            CfgDecision::Active => {}
        }
        let outer_bindings = self.local_bindings.clone();
        collect_generic_bindings(&function.sig.generics, &mut self.local_bindings);
        for input in &function.sig.inputs {
            if let syn::FnArg::Typed(argument) = input {
                collect_pattern_bindings(&argument.pat, &mut self.local_bindings);
            }
        }
        syn::visit::visit_trait_item_fn(self, function);
        self.local_bindings = outer_bindings;
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        // Inline module bodies are observed through their own ModuleRecord.
        for attribute in &module.attrs {
            self.visit_attribute(attribute);
        }
    }

    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
        self.strict_path_spans.push(path.path.span().start());
        syn::visit::visit_type_path(self, path);
    }

    fn visit_trait_bound(&mut self, bound: &'ast syn::TraitBound) {
        self.strict_path_spans.push(bound.path.span().start());
        syn::visit::visit_trait_bound(self, bound);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if let Some((_, path, _)) = &item.trait_ {
            self.strict_path_spans.push(path.span().start());
        }
        syn::visit::visit_item_impl(self, item);
    }

    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        syn::visit::visit_item_use(self, item);
    }

    fn visit_macro(&mut self, macro_item: &'ast syn::Macro) {
        if self.inactive_expr_cfg {
            return;
        }
        let identity = macro_item
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        if identity.len() == 1
            && identity
                .first()
                .is_some_and(|name| self.local_macro_bindings.contains(name))
        {
            let safe = identity
                .first()
                .is_some_and(|name| self.safe_local_macro_bindings.contains(name));
            let observes_arguments = identity
                .first()
                .is_some_and(|name| self.observing_local_macro_bindings.contains(name));
            if self.in_item_macro
                || self.cfg_guarded_macro
                || !safe
                || (observes_arguments && !visit_macro_expressions(self, macro_item.tokens.clone()))
            {
                self.macro_spans.push(macro_item.span().start());
            }
            return;
        }
        if let Some(declarations) = thread_local_declarations(macro_item)
            && resolves_to_standard_thread_local(
                &macro_item.path,
                self.glob_imports.is_empty() && !self.no_implicit_prelude,
                self.crate_macro_rules_names,
                |name| {
                    self.aliases.contains_key(name)
                        || self.macro_bindings.contains(name)
                        || self.declared_items.contains(name)
                        || self.local_item_bindings.contains(name)
                },
            )
        {
            for declaration in &declarations {
                visit_thread_local_declaration(self, declaration);
            }
            return;
        }
        if is_dependency_observing_macro(&identity) {
            let observed = if is_matches_macro(&identity) {
                visit_matches_macro(self, macro_item.tokens.clone())
            } else if is_json_macro(&identity) {
                visit_json_macro(self, macro_item.tokens.clone())
            } else {
                visit_macro_expressions(self, macro_item.tokens.clone())
            };
            if !observed {
                self.macro_spans.push(macro_item.span().start());
            }
            return;
        }
        if is_dependency_neutral_token_macro(&identity) {
            return;
        }
        if known_dependency_neutral_external_macro(
            &identity,
            &self.aliases,
            self.external_crates,
            self.declared_items,
            &self.local_item_bindings,
            macro_item.path.leading_colon.is_some(),
            0,
        ) {
            return;
        }
        if is_dependency_neutral_expression_macro(&identity) {
            if !visit_macro_expressions(self, macro_item.tokens.clone()) {
                self.macro_spans.push(macro_item.span().start());
            }
            return;
        }
        self.macro_spans.push(macro_item.span().start());
    }

    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr") {
            match self
                .cfg_context
                .evaluate_attributes(std::slice::from_ref(attribute))
            {
                CfgDecision::Inactive if self.in_expr => self.inactive_expr_cfg = true,
                CfgDecision::Unknown => self.cfg_spans.push(attribute.span().start()),
                CfgDecision::Active | CfgDecision::Inactive => {}
            }
        } else if attribute.path().is_ident("derive") {
            if !known_builtin_derive(attribute, &self.aliases, self.external_crates) {
                self.macro_spans.push(attribute.span().start());
            }
        } else if let Some(paths) = dependency_neutral_attribute_paths(attribute) {
            if self.observe_attribute_paths {
                for (path, evidence_span) in paths {
                    self.paths.push(ObservedPath {
                        path,
                        evidence_span,
                        requires_complete_resolution: true,
                        local_bindings: self.local_bindings.clone(),
                        aliases: self.aliases.clone(),
                        glob_imports: self.glob_imports.clone(),
                    });
                }
            }
            return;
        } else if !known_builtin_attribute(attribute.path()) {
            self.macro_spans.push(attribute.span().start());
        }
        syn::visit::visit_attribute(self, attribute);
    }
}

fn known_dependency_neutral_external_macro(
    path: &[String],
    aliases: &BTreeMap<String, Vec<UseAlias>>,
    external_crates: &BTreeSet<String>,
    declared_items: &BTreeSet<String>,
    local_item_bindings: &BTreeSet<String>,
    external_root_proven: bool,
    alias_depth: usize,
) -> bool {
    const MAX_ALIAS_DEPTH: usize = 16;
    if alias_depth > MAX_ALIAS_DEPTH {
        return false;
    }
    let Some(first) = path.first() else {
        return false;
    };
    if !external_root_proven && let Some(candidates) = aliases.get(first) {
        return !candidates.is_empty()
            && candidates.iter().all(|alias| {
                let Some(root) = alias.path.first() else {
                    return false;
                };
                let external_root_proven = external_root_proven || alias.absolute_external;
                if !external_root_proven
                    && (declared_items.contains(root) || local_item_bindings.contains(root))
                {
                    return false;
                }
                let mut expanded = alias.path.clone();
                expanded.extend_from_slice(&path[1..]);
                known_dependency_neutral_external_macro(
                    &expanded,
                    aliases,
                    external_crates,
                    declared_items,
                    local_item_bindings,
                    external_root_proven,
                    alias_depth + 1,
                )
            });
    }
    path == ["syn", "Token"]
        && external_crates.contains("syn")
        && (external_root_proven
            || (!declared_items.contains("syn") && !local_item_bindings.contains("syn")))
}

fn dependency_neutral_attribute_paths(
    attribute: &Attribute,
) -> Option<Vec<(syn::Path, LineColumn)>> {
    if is_rustfmt_skip_attribute(attribute) {
        return Some(Vec::new());
    }
    if !attribute.path().is_ident("serde") {
        return None;
    }
    let syn::Meta::List(list) = &attribute.meta else {
        return None;
    };
    let parser = Punctuated::<syn::Meta, Token![,]>::parse_terminated;
    let metas = parser.parse2(list.tokens.clone()).ok()?;
    let mut paths = Vec::new();
    for meta in metas {
        match meta {
            syn::Meta::Path(path)
                if path.get_ident().is_some_and(|name| {
                    matches!(
                        name.to_string().as_str(),
                        "alias"
                            | "content"
                            | "default"
                            | "deny_unknown_fields"
                            | "flatten"
                            | "rename_all"
                            | "skip"
                            | "skip_deserializing"
                            | "skip_serializing"
                            | "untagged"
                    )
                }) => {}
            syn::Meta::NameValue(value)
                if value.path.get_ident().is_some_and(|name| {
                    matches!(
                        name.to_string().as_str(),
                        "alias" | "content" | "rename" | "rename_all"
                    )
                }) && matches!(&value.value, syn::Expr::Lit(_)) => {}
            syn::Meta::NameValue(value)
                if value.path.get_ident().is_some_and(|name| {
                    matches!(
                        name.to_string().as_str(),
                        "default"
                            | "deserialize_with"
                            | "serialize_with"
                            | "skip_serializing_if"
                            | "with"
                    )
                }) =>
            {
                let syn::Expr::Lit(expression) = value.value else {
                    return None;
                };
                let syn::Lit::Str(literal) = expression.lit else {
                    return None;
                };
                let expression_path = syn::parse_str::<syn::ExprPath>(&literal.value()).ok()?;
                if expression_path.qself.is_some()
                    || expression_path
                        .path
                        .segments
                        .iter()
                        .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))
                {
                    return None;
                }
                paths.push((expression_path.path, literal.span().start()));
            }
            _ => return None,
        }
    }
    Some(paths)
}

fn is_rustfmt_skip_attribute(attribute: &Attribute) -> bool {
    attribute.path().segments.len() == 2
        && attribute.path().segments[0].ident == "rustfmt"
        && attribute.path().segments[1].ident == "skip"
}

fn known_builtin_derive(
    attribute: &Attribute,
    aliases: &BTreeMap<String, Vec<UseAlias>>,
    external_crates: &BTreeSet<String>,
) -> bool {
    let parser = Punctuated::<syn::Path, Token![,]>::parse_terminated;
    attribute.parse_args_with(parser).is_ok_and(|paths| {
        paths.iter().all(|path| {
            if known_external_derive_path(path, aliases, external_crates, 0) {
                return true;
            }
            path.get_ident().is_some_and(|name| {
                !aliases.contains_key(&name.to_string())
                    && matches!(
                        name.to_string().as_str(),
                        "Clone"
                            | "Copy"
                            | "Debug"
                            | "Default"
                            | "Eq"
                            | "Hash"
                            | "Ord"
                            | "PartialEq"
                            | "PartialOrd"
                    )
            })
        })
    })
}

fn known_external_derive_path(
    path: &syn::Path,
    aliases: &BTreeMap<String, Vec<UseAlias>>,
    external_crates: &BTreeSet<String>,
    alias_depth: usize,
) -> bool {
    const MAX_ALIAS_DEPTH: usize = 16;
    if alias_depth > MAX_ALIAS_DEPTH {
        return false;
    }
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    let Some(first) = segments.first() else {
        return false;
    };
    if segments.len() == 1
        && let Some(candidates) = aliases.get(first)
    {
        return !candidates.is_empty()
            && candidates.iter().all(|alias| {
                known_external_derive_alias(
                    alias,
                    &segments[1..],
                    aliases,
                    external_crates,
                    alias_depth,
                )
            });
    }
    let Some(last) = segments.last() else {
        return false;
    };
    if !matches!(last.as_str(), "Deserialize" | "Serialize") {
        return false;
    }
    if external_crates.contains(first) && KNOWN_EXTERNAL_DERIVE_CRATES.contains(&first.as_str()) {
        return true;
    }
    let Some(candidates) = aliases.get(first) else {
        return false;
    };
    !candidates.is_empty()
        && candidates.iter().all(|alias| {
            known_external_derive_alias(
                alias,
                &segments[1..],
                aliases,
                external_crates,
                alias_depth,
            )
        })
}

fn known_external_derive_alias(
    alias: &UseAlias,
    suffix: &[String],
    aliases: &BTreeMap<String, Vec<UseAlias>>,
    external_crates: &BTreeSet<String>,
    alias_depth: usize,
) -> bool {
    let Some(root) = alias.path.first() else {
        return false;
    };
    if alias.absolute_external
        && !external_crates.contains(root)
        && !matches!(
            root.as_str(),
            "alloc" | "core" | "proc_macro" | "std" | "test"
        )
    {
        return false;
    }
    let mut expanded = alias.path.clone();
    expanded.extend_from_slice(suffix);
    if expanded.is_empty() {
        return false;
    }
    let mut expanded_path = syn::Path {
        leading_colon: None,
        segments: Punctuated::new(),
    };
    for segment in expanded {
        expanded_path
            .segments
            .push(syn::PathSegment::from(syn::Ident::new(
                &segment,
                proc_macro2::Span::call_site(),
            )));
    }
    known_external_derive_path(&expanded_path, aliases, external_crates, alias_depth + 1)
}

const KNOWN_EXTERNAL_DERIVE_CRATES: &[&str] = &["serde"];

/// Declarations parsed from macro tokens do not live for the visited `'ast`
/// borrow, so they are visited through a lifetime-independent receiver.
fn visit_thread_local_declaration(visitor: &mut PathVisitor<'_>, declaration: &syn::ItemStatic) {
    visitor.visit_item_static(declaration);
}

fn visit_macro_expressions(visitor: &mut PathVisitor<'_>, tokens: TokenStream) -> bool {
    let parser = Punctuated::<syn::Expr, Token![,]>::parse_terminated;
    if let Ok(expressions) = parser.parse2(tokens.clone()) {
        for expression in &expressions {
            visitor.visit_expr(expression);
        }
        return true;
    }
    let wrapped = TokenStream::from(TokenTree::Group(Group::new(Delimiter::Bracket, tokens)));
    if let Ok(expression) = syn::parse2::<syn::Expr>(wrapped) {
        visitor.visit_expr(&expression);
        true
    } else {
        false
    }
}

fn visit_matches_macro(visitor: &mut PathVisitor<'_>, tokens: TokenStream) -> bool {
    let arguments = split_macro_tokens(tokens, ',');
    if arguments.len() != 2 {
        return false;
    }
    let Ok(expression) = syn::parse2::<syn::Expr>(arguments[0].clone()) else {
        return false;
    };
    let (pattern, guard) = if let Ok(pattern) = syn::Pat::parse_multi.parse2(arguments[1].clone()) {
        (pattern, None)
    } else {
        let Some((pattern_tokens, guard_tokens)) = split_macro_keyword(arguments[1].clone(), "if")
        else {
            return false;
        };
        let Ok(pattern) = syn::Pat::parse_multi.parse2(pattern_tokens) else {
            return false;
        };
        let Ok(guard) = syn::parse2::<syn::Expr>(guard_tokens) else {
            return false;
        };
        (pattern, Some(guard))
    };
    visitor.visit_expr(&expression);
    visitor.visit_pat(&pattern);
    if let Some(guard) = guard {
        visitor.visit_expr(&guard);
    }
    true
}

fn visit_json_macro(visitor: &mut PathVisitor<'_>, tokens: TokenStream) -> bool {
    visit_json_fragment(visitor, tokens)
}

fn visit_json_fragment(visitor: &mut PathVisitor<'_>, tokens: TokenStream) -> bool {
    if tokens.is_empty() {
        return true;
    }
    let mut trees = tokens.clone().into_iter();
    if let (Some(TokenTree::Ident(identifier)), None) = (trees.next(), trees.next())
        && matches!(identifier.to_string().as_str(), "null" | "true" | "false")
    {
        return true;
    }
    if let Ok(expression) = syn::parse2::<syn::Expr>(tokens.clone()) {
        visitor.visit_expr(&expression);
        return true;
    }
    let groups = tokens.clone().into_iter().collect::<Vec<_>>();
    if groups.len() == 1
        && let TokenTree::Group(group) = &groups[0]
    {
        return visit_json_fragment(visitor, group.stream());
    }
    let entries = split_macro_tokens(tokens.clone(), ',');
    if entries.len() > 1 {
        return entries
            .into_iter()
            .filter(|entry| !entry.is_empty())
            .all(|entry| visit_json_entry(visitor, entry));
    }
    visit_json_entry(visitor, tokens)
}

fn visit_json_entry(visitor: &mut PathVisitor<'_>, entry: TokenStream) -> bool {
    if let Some((_, value)) = split_macro_tokens_once(entry.clone(), ':') {
        return visit_json_fragment(visitor, value);
    }
    if let Ok(expression) = syn::parse2::<syn::Expr>(entry.clone()) {
        visitor.visit_expr(&expression);
        return true;
    }
    let groups = entry.into_iter().collect::<Vec<_>>();
    groups.len() == 1
        && matches!(&groups[0], TokenTree::Group(_))
        && visit_json_fragment(
            visitor,
            match &groups[0] {
                TokenTree::Group(group) => group.stream(),
                _ => TokenStream::new(),
            },
        )
}

fn split_macro_tokens(tokens: TokenStream, separator: char) -> Vec<TokenStream> {
    let mut parts = Vec::new();
    let mut current = TokenStream::new();
    for token in tokens {
        if matches!(&token, TokenTree::Punct(punct) if punct.as_char() == separator) {
            parts.push(current);
            current = TokenStream::new();
        } else {
            current.extend(std::iter::once(token));
        }
    }
    parts.push(current);
    parts
}

fn split_macro_tokens_once(
    tokens: TokenStream,
    separator: char,
) -> Option<(TokenStream, TokenStream)> {
    let mut before = TokenStream::new();
    let mut found = false;
    let mut after = TokenStream::new();
    for token in tokens {
        if !found && matches!(&token, TokenTree::Punct(punct) if punct.as_char() == separator) {
            found = true;
        } else if found {
            after.extend(std::iter::once(token));
        } else {
            before.extend(std::iter::once(token));
        }
    }
    found.then_some((before, after))
}

fn split_macro_keyword(tokens: TokenStream, keyword: &str) -> Option<(TokenStream, TokenStream)> {
    let mut before = TokenStream::new();
    let mut found = false;
    let mut after = TokenStream::new();
    for token in tokens {
        if !found && matches!(&token, TokenTree::Ident(identifier) if identifier == keyword) {
            found = true;
        } else if found {
            after.extend(std::iter::once(token));
        } else {
            before.extend(std::iter::once(token));
        }
    }
    found.then_some((before, after))
}

fn collect_pattern_bindings(pattern: &syn::Pat, bindings: &mut BTreeSet<String>) {
    struct BindingVisitor<'a>(&'a mut BTreeSet<String>);
    impl<'ast> Visit<'ast> for BindingVisitor<'_> {
        fn visit_pat_ident(&mut self, pattern: &'ast syn::PatIdent) {
            self.0.insert(pattern.ident.to_string());
            syn::visit::visit_pat_ident(self, pattern);
        }
    }
    BindingVisitor(bindings).visit_pat(pattern);
}

fn has_cfg_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr"))
}
