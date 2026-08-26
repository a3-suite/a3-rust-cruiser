use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{Delimiter, Group, LineColumn, Span, TokenStream, TokenTree};
use syn::parse::Parser;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Attribute, File, Item, ItemMod, ItemUse, Stmt, Token, UseTree, punctuated::Punctuated};

use super::cfg_context::{CfgContext, CfgDecision};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UseAlias {
    pub(super) path: Vec<String>,
    pub(super) absolute_external: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UseBinding {
    pub(super) name: String,
    pub(super) path: Vec<String>,
    pub(super) absolute_external: bool,
}

#[derive(Clone)]
pub(super) struct ObservedPath {
    pub(super) path: syn::Path,
    pub(super) evidence_span: LineColumn,
    pub(super) requires_complete_resolution: bool,
    pub(super) local_bindings: BTreeSet<String>,
    pub(super) aliases: BTreeMap<String, Vec<UseAlias>>,
    pub(super) glob_imports: Vec<UseAlias>,
}

#[derive(Clone)]
pub(super) struct ScopedUse {
    pub(super) item: ItemUse,
    pub(super) aliases: BTreeMap<String, Vec<UseAlias>>,
    pub(super) glob_imports: Vec<UseAlias>,
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

pub(super) struct PathVisitor<'a> {
    pub(super) cfg_context: &'a CfgContext,
    pub(super) external_crates: &'a BTreeSet<String>,
    pub(super) observe_attribute_paths: bool,
    pub(super) paths: Vec<ObservedPath>,
    pub(super) scoped_uses: Vec<ScopedUse>,
    pub(super) local_bindings: BTreeSet<String>,
    pub(super) aliases: BTreeMap<String, Vec<UseAlias>>,
    pub(super) glob_imports: Vec<UseAlias>,
    pub(super) macro_bindings: BTreeSet<String>,
    pub(super) local_macro_bindings: BTreeSet<String>,
    pub(super) safe_local_macro_bindings: BTreeSet<String>,
    pub(super) observing_local_macro_bindings: BTreeSet<String>,
    pub(super) inactive_expr_cfg: bool,
    pub(super) in_expr: bool,
    pub(super) in_item_macro: bool,
    pub(super) cfg_guarded_macro: bool,
    pub(super) no_implicit_prelude: bool,
    pub(super) macro_spans: Vec<LineColumn>,
    pub(super) cfg_spans: Vec<LineColumn>,
    pub(super) strict_path_spans: Vec<LineColumn>,
    pub(super) use_spans: Vec<LineColumn>,
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
                        self.cfg_context,
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
        self.use_spans.push(item.span().start());
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

fn is_dependency_neutral_token_macro(path: &[String]) -> bool {
    macro_identity(path).is_some_and(|name| {
        matches!(
            name,
            "cfg"
                | "column"
                | "concat"
                | "env"
                | "file"
                | "format_description"
                | "include_bytes"
                | "include_str"
                | "line"
                | "macro_rules"
                | "module_path"
                | "option_env"
                | "stringify"
        )
    })
}

fn is_dependency_observing_macro(path: &[String]) -> bool {
    [
        &["matches"][..],
        &["json"][..],
        &["serde_json", "json"][..],
        &["inventory", "submit"][..],
        &["inventory", "collect"][..],
        &["require_contract_value"][..],
        &["require_shape"][..],
    ]
    .iter()
    .any(|expected| macro_path_is(path, expected))
}

fn is_matches_macro(path: &[String]) -> bool {
    macro_path_is(path, &["matches"])
}

fn is_json_macro(path: &[String]) -> bool {
    macro_path_is(path, &["json"]) || macro_path_is(path, &["serde_json", "json"])
}

fn macro_path_is(path: &[String], expected: &[&str]) -> bool {
    path.len() == expected.len()
        && path
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual == expected)
}

fn is_dependency_neutral_expression_macro(path: &[String]) -> bool {
    macro_identity(path).is_some_and(|name| {
        matches!(
            name,
            "assert"
                | "assert_eq"
                | "assert_ne"
                | "dbg"
                | "debug_assert"
                | "debug_assert_eq"
                | "debug_assert_ne"
                | "eprint"
                | "eprintln"
                | "format"
                | "format_args"
                | "panic"
                | "print"
                | "println"
                | "todo"
                | "unimplemented"
                | "unreachable"
                | "vec"
                | "write"
                | "writeln"
        )
    })
}

fn macro_identity(path: &[String]) -> Option<&str> {
    match path {
        [name] => Some(name),
        [root, name] if matches!(root.as_str(), "alloc" | "core" | "std") => Some(name),
        _ => None,
    }
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

pub(super) fn item_attributes(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        Item::Verbatim(_) => &[],
        _ => &[],
    }
}

pub(super) fn flatten_use_tree(tree: &UseTree, prefix: Vec<String>, paths: &mut Vec<Vec<String>>) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            flatten_use_tree(&path.tree, next, paths);
        }
        UseTree::Name(name) => {
            let mut path = prefix;
            path.push(name.ident.to_string());
            paths.push(path);
        }
        UseTree::Rename(rename) => {
            let mut path = prefix;
            path.push(rename.ident.to_string());
            paths.push(path);
        }
        UseTree::Glob(_) => paths.push(prefix),
        UseTree::Group(group) => {
            for item in &group.items {
                flatten_use_tree(item, prefix.clone(), paths);
            }
        }
    }
}

pub(super) fn declared_item_names(file: &File, cfg_context: &CfgContext) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for item in &file.items {
        if cfg_context.evaluate_attributes(item_attributes(item)) == CfgDecision::Inactive {
            continue;
        }
        let name = match item {
            Item::Use(item) => {
                collect_use_bindings(&item.tree, Vec::new(), &mut names);
                None
            }
            _ => declared_item_name(item),
        };
        if let Some(name) = name {
            names.insert(name);
        }
    }
    names
}

pub(super) fn public_item_names(file: &File, cfg_context: &CfgContext) -> BTreeSet<String> {
    file.items
        .iter()
        .filter(|item| {
            cfg_context.evaluate_attributes(item_attributes(item)) != CfgDecision::Inactive
        })
        .filter(|item| !matches!(item, Item::Use(_)))
        .filter(|item| item_visibility(item).is_some_and(is_crate_visible))
        .filter_map(declared_item_name)
        .collect()
}

pub(super) fn public_use_bindings(file: &File, cfg_context: &CfgContext) -> Vec<UseBinding> {
    let mut bindings = Vec::new();
    for item in &file.items {
        if cfg_context.evaluate_attributes(item_attributes(item)) == CfgDecision::Inactive {
            continue;
        }
        if let Item::Use(item) = item
            && is_crate_visible(&item.vis)
        {
            collect_use_bindings_with_paths(
                &item.tree,
                Vec::new(),
                item.leading_colon.is_some(),
                &mut bindings,
            );
        }
    }
    bindings
}

pub(super) fn glob_imports(file: &File, cfg_context: &CfgContext) -> Vec<UseAlias> {
    let mut imports = Vec::new();
    for item in &file.items {
        if cfg_context.evaluate_attributes(item_attributes(item)) == CfgDecision::Inactive {
            continue;
        }
        if let Item::Use(item) = item {
            collect_use_glob_imports(
                &item.tree,
                Vec::new(),
                item.leading_colon.is_some(),
                &mut imports,
            );
        }
    }
    imports
}

pub(super) fn public_glob_imports(file: &File, cfg_context: &CfgContext) -> Vec<UseAlias> {
    let mut imports = Vec::new();
    for item in &file.items {
        if cfg_context.evaluate_attributes(item_attributes(item)) == CfgDecision::Inactive {
            continue;
        }
        if let Item::Use(item) = item
            && is_crate_visible(&item.vis)
        {
            collect_use_glob_imports(
                &item.tree,
                Vec::new(),
                item.leading_colon.is_some(),
                &mut imports,
            );
        }
    }
    imports
}

pub(super) fn is_crate_visible(visibility: &syn::Visibility) -> bool {
    match visibility {
        syn::Visibility::Public(_) => true,
        syn::Visibility::Restricted(restricted) => restricted.path.is_ident("crate"),
        syn::Visibility::Inherited => false,
    }
}

fn item_visibility(item: &Item) -> Option<&syn::Visibility> {
    match item {
        Item::Const(item) => Some(&item.vis),
        Item::Enum(item) => Some(&item.vis),
        Item::ExternCrate(item) => Some(&item.vis),
        Item::Fn(item) => Some(&item.vis),
        Item::Macro(_) => None,
        Item::Mod(item) => Some(&item.vis),
        Item::Static(item) => Some(&item.vis),
        Item::Struct(item) => Some(&item.vis),
        Item::Trait(item) => Some(&item.vis),
        Item::TraitAlias(item) => Some(&item.vis),
        Item::Type(item) => Some(&item.vis),
        Item::Union(item) => Some(&item.vis),
        Item::Use(item) => Some(&item.vis),
        Item::ForeignMod(_) | Item::Impl(_) | Item::Verbatim(_) => None,
        _ => None,
    }
}

fn declared_item_name(item: &Item) -> Option<String> {
    match item {
        Item::Const(item) => Some(item.ident.to_string()),
        Item::Enum(item) => Some(item.ident.to_string()),
        Item::ExternCrate(item) => Some(
            item.rename
                .as_ref()
                .map_or(&item.ident, |(_, rename)| rename)
                .to_string(),
        ),
        Item::Fn(item) => Some(item.sig.ident.to_string()),
        Item::Macro(item) => item.ident.as_ref().map(ToString::to_string),
        Item::Mod(item) => Some(item.ident.to_string()),
        Item::Static(item) => Some(item.ident.to_string()),
        Item::Struct(item) => Some(item.ident.to_string()),
        Item::Trait(item) => Some(item.ident.to_string()),
        Item::TraitAlias(item) => Some(item.ident.to_string()),
        Item::Type(item) => Some(item.ident.to_string()),
        Item::Union(item) => Some(item.ident.to_string()),
        Item::Use(_) | Item::ForeignMod(_) | Item::Impl(_) | Item::Verbatim(_) => None,
        _ => None,
    }
}

pub(super) fn declared_macro_bindings(file: &File, cfg_context: &CfgContext) -> BTreeSet<String> {
    let mut bindings = BTreeSet::new();
    for item in &file.items {
        if cfg_context.evaluate_attributes(item_attributes(item)) == CfgDecision::Inactive {
            continue;
        }
        if let Item::Macro(item) = item
            && let Some(ident) = &item.ident
        {
            bindings.insert(ident.to_string());
        }
        if let Item::Use(item) = item {
            collect_use_bindings(&item.tree, Vec::new(), &mut bindings);
        }
    }
    bindings
}

pub(super) fn local_macro_bindings(file: &File, cfg_context: &CfgContext) -> BTreeSet<String> {
    file.items
        .iter()
        .filter(|item| {
            cfg_context.evaluate_attributes(item_attributes(item)) != CfgDecision::Inactive
        })
        .filter_map(|item| match item {
            Item::Macro(item) => item.ident.as_ref().map(ToString::to_string),
            _ => None,
        })
        .collect()
}

#[derive(Clone)]
pub(super) struct SupportedItemMacroDefinition {
    parameters: Vec<MacroParameter>,
    transcriber: TokenStream,
}

#[derive(Clone)]
struct MacroParameter {
    key: &'static str,
    binding: String,
    fragment: &'static str,
}

const BUILTIN_RUNTIME_PARAMETERS: &[(&str, &str)] = &[
    ("slot", "ident"),
    ("agnostic_fn", "ident"),
    ("contextual_fn", "ident"),
    ("agnostic_for_fn", "ident"),
    ("contextual_for_fn", "ident"),
    ("source_view_resolver", "path"),
    ("language", "literal"),
    ("provider", "literal"),
    ("agnostic_provider", "path"),
    ("contextual_provider", "path"),
];

const POLICY_RUNTIME_PARAMETERS: &[(&str, &str)] = &[
    ("slot", "ident"),
    ("agnostic_fn", "ident"),
    ("contextual_fn", "ident"),
    ("agnostic_for_fn", "ident"),
    ("contextual_for_fn", "ident"),
    ("source_view_resolver", "path"),
    ("language", "literal"),
    ("provider", "literal"),
];

pub(super) fn supported_item_macro_definition(
    item: &syn::ItemMacro,
    cfg_context: &CfgContext,
) -> Option<SupportedItemMacroDefinition> {
    if !item.mac.path.is_ident("macro_rules")
        || !item.attrs.is_empty()
        || cfg_context.evaluate_attributes(&item.attrs) != CfgDecision::Active
    {
        return None;
    }
    let tokens = item.mac.tokens.clone().into_iter().collect::<Vec<_>>();
    let (matcher, transcriber) = split_single_macro_rule(&tokens)?;
    let [TokenTree::Group(matcher)] = matcher.as_slice() else {
        return None;
    };
    let [TokenTree::Group(transcriber)] = transcriber.as_slice() else {
        return None;
    };
    for (expected, expected_transcriber) in [
        (BUILTIN_RUNTIME_PARAMETERS, builtin_runtime_transcriber()),
        (POLICY_RUNTIME_PARAMETERS, policy_runtime_transcriber()),
    ] {
        let Some(parameters) =
            parse_named_macro_matcher(&matcher.stream().into_iter().collect::<Vec<_>>(), expected)
        else {
            continue;
        };
        let canonical_names = parameters
            .iter()
            .zip(expected)
            .map(|(parameter, (key, _))| (parameter.binding.clone(), *key))
            .collect::<BTreeMap<_, _>>();
        let normalized = normalize_metavariables(transcriber.stream(), &canonical_names)?;
        if normalized.to_string() == expected_transcriber.to_string() {
            return Some(SupportedItemMacroDefinition {
                parameters,
                transcriber: transcriber.stream(),
            });
        }
    }
    None
}

pub(super) fn expand_supported_item_macro(
    definition: &SupportedItemMacroDefinition,
    arguments: TokenStream,
    invocation_span: Span,
) -> Option<Vec<Item>> {
    let values = parse_named_macro_arguments(arguments, &definition.parameters)?;
    let expanded = substitute_metavariables(definition.transcriber.clone(), &values)?;
    let expanded = with_token_stream_span(expanded, invocation_span);
    syn::parse2::<File>(expanded).ok().map(|file| file.items)
}

fn parse_named_macro_matcher(
    tokens: &[TokenTree],
    expected: &[(&'static str, &'static str)],
) -> Option<Vec<MacroParameter>> {
    let mut cursor = TokenCursor::new(tokens);
    let mut parameters = Vec::with_capacity(expected.len());
    let mut bindings = BTreeSet::new();
    for (index, (key, fragment)) in expected.iter().enumerate() {
        if index > 0 && !cursor.punct(',') {
            return None;
        }
        if !cursor.ident(key) || !cursor.punct('=') || !cursor.punct('$') {
            return None;
        }
        let binding = cursor.ident_any()?;
        if !bindings.insert(binding.clone()) || !cursor.punct(':') || !cursor.ident(fragment) {
            return None;
        }
        parameters.push(MacroParameter {
            key,
            binding,
            fragment,
        });
    }
    if !cursor.end() && !(cursor.punct(',') && cursor.end()) {
        return None;
    }
    Some(parameters)
}

fn parse_named_macro_arguments(
    tokens: TokenStream,
    parameters: &[MacroParameter],
) -> Option<BTreeMap<String, TokenStream>> {
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    let mut values = BTreeMap::new();
    let mut start = 0;
    for parameter in parameters {
        let end = tokens[start..]
            .iter()
            .position(|token| is_punct(token, ','))
            .map_or(tokens.len(), |offset| start + offset);
        let assignment = &tokens[start..end];
        let [TokenTree::Ident(key), TokenTree::Punct(equals), value @ ..] = assignment else {
            return None;
        };
        if key != parameter.key || equals.as_char() != '=' || value.is_empty() {
            return None;
        }
        let value = value.iter().cloned().collect::<TokenStream>();
        let valid = match parameter.fragment {
            "ident" => matches!(assignment, [_, _, TokenTree::Ident(_)]),
            "literal" => matches!(assignment, [_, _, TokenTree::Literal(_)]),
            "path" => syn::parse2::<syn::Path>(value.clone()).is_ok(),
            _ => false,
        };
        if !valid || values.insert(parameter.binding.clone(), value).is_some() {
            return None;
        }
        start = end.saturating_add(1);
    }
    if start < tokens.len() {
        return None;
    }
    Some(values)
}

fn normalize_metavariables(
    tokens: TokenStream,
    names: &BTreeMap<String, &'static str>,
) -> Option<TokenStream> {
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    let mut output = TokenStream::new();
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            TokenTree::Punct(punct) if punct.as_char() == '$' => {
                let TokenTree::Ident(binding) = tokens.get(index + 1)? else {
                    return None;
                };
                let canonical = names.get(&binding.to_string())?;
                output.extend([
                    tokens[index].clone(),
                    TokenTree::Ident(syn::Ident::new(canonical, Span::call_site())),
                ]);
                index += 2;
            }
            TokenTree::Group(group) => {
                let stream = normalize_metavariables(group.stream(), names)?;
                let mut normalized = Group::new(group.delimiter(), stream);
                normalized.set_span(group.span());
                output.extend([TokenTree::Group(normalized)]);
                index += 1;
            }
            token => {
                output.extend([token.clone()]);
                index += 1;
            }
        }
    }
    Some(output)
}

fn substitute_metavariables(
    tokens: TokenStream,
    values: &BTreeMap<String, TokenStream>,
) -> Option<TokenStream> {
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    let mut output = TokenStream::new();
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            TokenTree::Punct(punct) if punct.as_char() == '$' => {
                let TokenTree::Ident(binding) = tokens.get(index + 1)? else {
                    return None;
                };
                output.extend(values.get(&binding.to_string())?.clone());
                index += 2;
            }
            TokenTree::Group(group) => {
                let stream = substitute_metavariables(group.stream(), values)?;
                let mut substituted = Group::new(group.delimiter(), stream);
                substituted.set_span(group.span());
                output.extend([TokenTree::Group(substituted)]);
                index += 1;
            }
            token => {
                output.extend([token.clone()]);
                index += 1;
            }
        }
    }
    Some(output)
}

fn with_token_stream_span(tokens: TokenStream, span: Span) -> TokenStream {
    tokens
        .into_iter()
        .map(|mut token| {
            if let TokenTree::Group(group) = token {
                let mut group = Group::new(
                    group.delimiter(),
                    with_token_stream_span(group.stream(), span),
                );
                group.set_span(span);
                token = TokenTree::Group(group);
            } else {
                token.set_span(span);
            }
            token
        })
        .collect()
}

fn builtin_runtime_transcriber() -> TokenStream {
    r#"{
        pub(crate) const LANGUAGE: &str = $language;
        pub(crate) const PROVIDER_NAME: &str = $provider;
        pub(crate) const $slot: crate::foundation::plugin::provider::LanguagePluginProviderSlot =
            crate::foundation::plugin::provider::LanguagePluginProviderSlot {
                name: PROVIDER_NAME,
                language: LANGUAGE,
                language_agnostic_snapshot_extractor_for: $agnostic_for_fn,
                contextual_snapshot_materializer_for: $contextual_for_fn,
                source_view_resolver: $source_view_resolver,
            };
        inventory::submit! { $slot }
        pub(crate) fn $agnostic_fn() -> Option<crate::foundation::plugin::language_agnostic_snapshot_extractor::LanguageAgnosticSnapshotExtractor> { $agnostic_provider() }
        pub(crate) fn $contextual_fn() -> Option<crate::foundation::plugin::contextual_snapshot_materializer::ContextualSnapshotMaterializer> { $contextual_provider() }
        fn $agnostic_for_fn(language: &str,) -> Option<crate::foundation::plugin::language_agnostic_snapshot_extractor::LanguageAgnosticSnapshotExtractor> { if language == LANGUAGE { $agnostic_fn() } else { None } }
        fn $contextual_for_fn(language: &str,) -> Option<crate::foundation::plugin::contextual_snapshot_materializer::ContextualSnapshotMaterializer> { if language == LANGUAGE { $contextual_fn() } else { None } }
    }"#
        .parse::<TokenStream>()
        .expect("built-in runtime macro template must parse")
        .into_iter()
        .next()
        .and_then(|token| match token {
            TokenTree::Group(group) => Some(group.stream()),
            _ => None,
        })
        .expect("built-in runtime macro template must be a group")
}

fn policy_runtime_transcriber() -> TokenStream {
    r#"{
        crate::foundation::plugin::policy_only::declare_builtin_language_runtime! {
            slot = $slot,
            agnostic_fn = $agnostic_fn,
            contextual_fn = $contextual_fn,
            agnostic_for_fn = $agnostic_for_fn,
            contextual_for_fn = $contextual_for_fn,
            source_view_resolver = $source_view_resolver,
            language = $language,
            provider = $provider,
            agnostic_provider = crate::foundation::plugin::policy_only::no_language_agnostic_snapshot_extractor,
            contextual_provider = crate::foundation::plugin::policy_only::no_contextual_snapshot_materializer
        }
    }"#
        .parse::<TokenStream>()
        .expect("policy runtime macro template must parse")
        .into_iter()
        .next()
        .and_then(|token| match token {
            TokenTree::Group(group) => Some(group.stream()),
            _ => None,
        })
        .expect("policy runtime macro template must be a group")
}

pub(super) fn local_macro_safety_bindings(
    file: &File,
    cfg_context: &CfgContext,
    declared_items: &BTreeSet<String>,
    no_implicit_prelude: bool,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let macro_bindings = declared_macro_bindings(file, cfg_context);
    let scope = MacroDefinitionScope {
        declared_items,
        macro_bindings: &macro_bindings,
        no_implicit_prelude,
    };
    let mut safe = BTreeSet::new();
    let mut observing = BTreeSet::new();
    for item in &file.items {
        let Item::Macro(item_macro) = item else {
            continue;
        };
        let Some(name) = item_macro.ident.as_ref().map(ToString::to_string) else {
            continue;
        };
        let Some(observes_arguments) = local_macro_safety(item_macro, cfg_context, &scope) else {
            continue;
        };
        safe.insert(name.clone());
        if observes_arguments {
            observing.insert(name);
        }
    }
    (safe, observing)
}

struct MacroDefinitionScope<'a> {
    declared_items: &'a BTreeSet<String>,
    macro_bindings: &'a BTreeSet<String>,
    no_implicit_prelude: bool,
}

fn local_macro_safety(
    item: &syn::ItemMacro,
    cfg_context: &CfgContext,
    scope: &MacroDefinitionScope<'_>,
) -> Option<bool> {
    if !item.mac.path.is_ident("macro_rules")
        || !item.attrs.is_empty()
        || cfg_context.evaluate_attributes(&item.attrs) != CfgDecision::Active
    {
        return None;
    }
    let tokens = item.mac.tokens.clone().into_iter().collect::<Vec<_>>();
    let (matcher, transcriber) = split_single_macro_rule(&tokens)?;
    let [TokenTree::Group(matcher_group)] = matcher.as_slice() else {
        return None;
    };
    let [TokenTree::Group(transcriber_group)] = transcriber.as_slice() else {
        return None;
    };
    if let Some(observes_arguments) = exact_validation_macro_safety(
        &matcher_group.stream().into_iter().collect::<Vec<_>>(),
        &transcriber_group.stream().into_iter().collect::<Vec<_>>(),
        scope,
    ) {
        return Some(observes_arguments);
    }
    let mut bindings = BTreeSet::new();
    if !validate_macro_matcher(
        &matcher_group.stream().into_iter().collect::<Vec<_>>(),
        &mut bindings,
    ) {
        return None;
    }
    let mut observes_arguments = false;
    if !validate_macro_transcriber(
        &transcriber_group.stream().into_iter().collect::<Vec<_>>(),
        &bindings,
        scope.macro_bindings,
        &mut observes_arguments,
        true,
    ) {
        return None;
    }
    Some(observes_arguments)
}

fn exact_validation_macro_safety(
    matcher: &[TokenTree],
    transcriber: &[TokenTree],
    scope: &MacroDefinitionScope<'_>,
) -> Option<bool> {
    if !scope_allows_validation_symbols(scope) {
        return None;
    }
    let matcher = exact_macro_matcher(matcher, &["expr", "expr", "literal"])
        .or_else(|| exact_macro_matcher(matcher, &["expr", "expr", "expr", "literal"]))?;
    if matcher.len() == 3 && exact_contract_value_transcriber(transcriber, &matcher) {
        return Some(true);
    }
    if matcher.len() == 4 && exact_shape_transcriber(transcriber, &matcher) {
        return Some(true);
    }
    None
}

fn exact_macro_matcher(tokens: &[TokenTree], fragments: &[&str]) -> Option<Vec<String>> {
    let mut cursor = TokenCursor::new(tokens);
    let mut names = Vec::with_capacity(fragments.len());
    let mut seen = BTreeSet::new();
    for (index, fragment) in fragments.iter().enumerate() {
        if index > 0 && !cursor.punct(',') {
            return None;
        }
        if !cursor.punct('$') {
            return None;
        }
        let name = cursor.ident_any()?;
        if !seen.insert(name.clone()) || !cursor.punct(':') || !cursor.ident(fragment) {
            return None;
        }
        names.push(name);
    }
    cursor.end().then_some(names)
}

fn exact_contract_value_transcriber(tokens: &[TokenTree], matcher: &[String]) -> bool {
    let [actual, expected, label] = matcher else {
        return false;
    };
    let mut cursor = TokenCursor::new(tokens);
    if !cursor.ident("if")
        || !cursor.metavar(actual)
        || !cursor.punct('!')
        || !cursor.punct('=')
        || !cursor.metavar(expected)
    {
        return false;
    }
    let Some(body) = cursor.group(Delimiter::Brace) else {
        return false;
    };
    if !cursor.end() {
        return false;
    }
    let mut body_cursor = TokenCursor::new(&body);
    if !body_cursor.ident("return") || !body_cursor.ident("Err") {
        return false;
    }
    let Some(err_arguments) = body_cursor.group(Delimiter::Parenthesis) else {
        return false;
    };
    if !body_cursor.punct(';') || !body_cursor.end() {
        return false;
    }
    exact_contract_format_arguments(&err_arguments, expected, label)
}

fn exact_contract_format_arguments(tokens: &[TokenTree], expected: &str, label: &str) -> bool {
    let mut cursor = TokenCursor::new(tokens);
    if !cursor.ident("format") || !cursor.punct('!') {
        return false;
    }
    let Some(arguments) = cursor.group(Delimiter::Parenthesis) else {
        return false;
    };
    if !cursor.end() {
        return false;
    }
    let mut arguments_cursor = TokenCursor::new(&arguments);
    arguments_cursor.literal("\"{} must be {}\"")
        && arguments_cursor.punct(',')
        && arguments_cursor.metavar(label)
        && arguments_cursor.punct(',')
        && arguments_cursor.metavar(expected)
        && arguments_cursor.end()
}

fn exact_shape_transcriber(tokens: &[TokenTree], matcher: &[String]) -> bool {
    let [path, actual, expected, field] = matcher else {
        return false;
    };
    let mut cursor = TokenCursor::new(tokens);
    if !cursor.ident("if")
        || !cursor.metavar(actual)
        || !cursor.punct('!')
        || !cursor.punct('=')
        || !cursor.metavar(expected)
    {
        return false;
    }
    let Some(body) = cursor.group(Delimiter::Brace) else {
        return false;
    };
    if !cursor.end() {
        return false;
    }
    let mut body_cursor = TokenCursor::new(&body);
    if !body_cursor.ident("return") || !body_cursor.ident("Err") {
        return false;
    }
    let Some(err_arguments) = body_cursor.group(Delimiter::Parenthesis) else {
        return false;
    };
    body_cursor.punct(';')
        && body_cursor.end()
        && exact_shape_format_arguments(&err_arguments, path, field)
}

fn exact_shape_format_arguments(tokens: &[TokenTree], path: &str, field: &str) -> bool {
    let mut cursor = TokenCursor::new(tokens);
    if !cursor.ident("format") || !cursor.punct('!') {
        return false;
    }
    let Some(arguments) = cursor.group(Delimiter::Parenthesis) else {
        return false;
    };
    if !cursor.end() {
        return false;
    }
    let mut arguments_cursor = TokenCursor::new(&arguments);
    if !arguments_cursor.literal("\"{} {} does not match the authoritative schema\"")
        || !arguments_cursor.punct(',')
        || !arguments_cursor.metavar(path)
        || !arguments_cursor.punct('.')
        || !arguments_cursor.ident("display")
    {
        return false;
    }
    let Some(display_arguments) = arguments_cursor.group(Delimiter::Parenthesis) else {
        return false;
    };
    display_arguments.is_empty()
        && arguments_cursor.punct(',')
        && arguments_cursor.metavar(field)
        && arguments_cursor.end()
}

fn scope_allows_validation_symbols(scope: &MacroDefinitionScope<'_>) -> bool {
    !scope.no_implicit_prelude
        && !scope.declared_items.contains("Err")
        && !scope.macro_bindings.contains("Err")
        && !scope.declared_items.contains("format")
        && !scope.macro_bindings.contains("format")
}

struct TokenCursor<'a> {
    tokens: &'a [TokenTree],
    index: usize,
}

impl<'a> TokenCursor<'a> {
    fn new(tokens: &'a [TokenTree]) -> Self {
        Self { tokens, index: 0 }
    }

    fn punct(&mut self, expected: char) -> bool {
        let Some(TokenTree::Punct(punct)) = self.tokens.get(self.index) else {
            return false;
        };
        if punct.as_char() != expected {
            return false;
        }
        self.index += 1;
        true
    }

    fn ident(&mut self, expected: &str) -> bool {
        let Some(TokenTree::Ident(identifier)) = self.tokens.get(self.index) else {
            return false;
        };
        if identifier != expected {
            return false;
        }
        self.index += 1;
        true
    }

    fn ident_any(&mut self) -> Option<String> {
        let TokenTree::Ident(identifier) = self.tokens.get(self.index)? else {
            return None;
        };
        self.index += 1;
        Some(identifier.to_string())
    }

    fn literal(&mut self, expected: &str) -> bool {
        let Some(TokenTree::Literal(literal)) = self.tokens.get(self.index) else {
            return false;
        };
        if literal.to_string() != expected {
            return false;
        }
        self.index += 1;
        true
    }

    fn metavar(&mut self, expected: &str) -> bool {
        self.punct('$') && self.ident(expected)
    }

    fn group(&mut self, expected: Delimiter) -> Option<Vec<TokenTree>> {
        let TokenTree::Group(group) = self.tokens.get(self.index)? else {
            return None;
        };
        if group.delimiter() != expected {
            return None;
        }
        self.index += 1;
        Some(group.stream().into_iter().collect())
    }

    fn end(&self) -> bool {
        self.index == self.tokens.len()
    }
}

fn split_single_macro_rule(tokens: &[TokenTree]) -> Option<(Vec<TokenTree>, Vec<TokenTree>)> {
    let mut arrow_index = None;
    let mut index = 0;
    while index + 1 < tokens.len() {
        if is_punct(&tokens[index], '=') && is_punct(&tokens[index + 1], '>') {
            if arrow_index.replace(index).is_some() {
                return None;
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    let arrow_index = arrow_index?;
    let matcher = tokens[..arrow_index].to_vec();
    let mut transcriber = tokens[arrow_index + 2..].to_vec();
    if matches!(transcriber.last(), Some(TokenTree::Punct(punct)) if punct.as_char() == ';') {
        transcriber.pop();
    }
    if transcriber
        .iter()
        .any(|token| matches!(token, TokenTree::Punct(punct) if punct.as_char() == ';'))
    {
        return None;
    }
    Some((matcher, transcriber))
}

fn validate_macro_matcher(tokens: &[TokenTree], bindings: &mut BTreeSet<String>) -> bool {
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            TokenTree::Group(group) => {
                if !validate_macro_matcher(
                    &group.stream().into_iter().collect::<Vec<_>>(),
                    bindings,
                ) {
                    return false;
                }
            }
            TokenTree::Punct(punct) if punct.as_char() == '$' => {
                let Some(TokenTree::Ident(name)) = tokens.get(index + 1) else {
                    return false;
                };
                let Some(TokenTree::Punct(colon)) = tokens.get(index + 2) else {
                    return false;
                };
                let Some(TokenTree::Ident(fragment)) = tokens.get(index + 3) else {
                    return false;
                };
                if colon.as_char() != ':'
                    || !matches!(fragment.to_string().as_str(), "expr" | "literal")
                    || !bindings.insert(name.to_string())
                {
                    return false;
                }
                index += 3;
            }
            TokenTree::Punct(punct) if matches!(punct.as_char(), '*' | '+' | '?') => {
                return false;
            }
            _ => {}
        }
        index += 1;
    }
    true
}

fn validate_macro_transcriber(
    tokens: &[TokenTree],
    bindings: &BTreeSet<String>,
    macro_bindings: &BTreeSet<String>,
    observes_arguments: &mut bool,
    observe_metavariables: bool,
) -> bool {
    let mut index = 0;
    while index < tokens.len() {
        if let Some((next_index, path, nested_tokens)) =
            known_nested_macro(tokens, index, macro_bindings)
        {
            let valid = if macro_path_is(&path, &["cfg"]) {
                validate_cfg_macro_tokens(&nested_tokens)
            } else {
                validate_macro_transcriber(
                    &nested_tokens,
                    bindings,
                    macro_bindings,
                    observes_arguments,
                    observe_metavariables && !is_dependency_neutral_token_macro(&path),
                )
            };
            if !valid {
                return false;
            }
            index = next_index;
            continue;
        }
        match &tokens[index] {
            TokenTree::Group(group) => {
                if !validate_macro_transcriber(
                    &group.stream().into_iter().collect::<Vec<_>>(),
                    bindings,
                    macro_bindings,
                    observes_arguments,
                    observe_metavariables,
                ) {
                    return false;
                }
            }
            TokenTree::Punct(punct) if punct.as_char() == '$' => {
                let Some(TokenTree::Ident(name)) = tokens.get(index + 1) else {
                    return false;
                };
                if !bindings.contains(&name.to_string()) {
                    return false;
                }
                if observe_metavariables {
                    *observes_arguments = true;
                }
                index += 1;
            }
            TokenTree::Ident(identifier) if !is_safe_macro_keyword(identifier) => {
                return false;
            }
            TokenTree::Punct(punct) if matches!(punct.as_char(), '#' | ':' | '\'') => {
                return false;
            }
            _ => {}
        }
        index += 1;
    }
    true
}

fn known_nested_macro(
    tokens: &[TokenTree],
    start: usize,
    macro_bindings: &BTreeSet<String>,
) -> Option<(usize, Vec<String>, Vec<TokenTree>)> {
    let TokenTree::Ident(first) = tokens.get(start)? else {
        return None;
    };
    if macro_bindings.contains(&first.to_string()) {
        return None;
    }
    let mut path = vec![first.to_string()];
    let mut index = start + 1;
    while index + 2 < tokens.len()
        && is_punct(&tokens[index], ':')
        && is_punct(&tokens[index + 1], ':')
    {
        let TokenTree::Ident(segment) = &tokens[index + 2] else {
            return None;
        };
        path.push(segment.to_string());
        index += 3;
    }
    if !is_punct(tokens.get(index)?, '!') {
        return None;
    }
    let TokenTree::Group(arguments) = tokens.get(index + 1)? else {
        return None;
    };
    if !(is_dependency_neutral_token_macro(&path)
        || is_dependency_neutral_expression_macro(&path)
        || is_dependency_observing_macro(&path))
    {
        return None;
    }
    Some((index + 2, path, arguments.stream().into_iter().collect()))
}

fn validate_cfg_macro_tokens(tokens: &[TokenTree]) -> bool {
    tokens.iter().all(|token| match token {
        TokenTree::Group(group) => {
            validate_cfg_macro_tokens(&group.stream().into_iter().collect::<Vec<_>>())
        }
        TokenTree::Punct(punct) => !matches!(punct.as_char(), '#' | ':' | '\'' | '$'),
        TokenTree::Ident(_) | TokenTree::Literal(_) => true,
    })
}

fn is_safe_macro_keyword(identifier: &syn::Ident) -> bool {
    matches!(
        identifier.to_string().as_str(),
        "_" | "as"
            | "async"
            | "await"
            | "break"
            | "continue"
            | "else"
            | "false"
            | "for"
            | "if"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "move"
            | "mut"
            | "return"
            | "true"
            | "unsafe"
            | "while"
    )
}

fn is_punct(token: &TokenTree, expected: char) -> bool {
    matches!(token, TokenTree::Punct(punct) if punct.as_char() == expected)
}

fn has_cfg_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr"))
}

pub(super) fn declared_external_crate_aliases(
    file: &File,
    cfg_context: &CfgContext,
) -> BTreeSet<String> {
    file.items
        .iter()
        .filter_map(|item| {
            if cfg_context.evaluate_attributes(item_attributes(item)) == CfgDecision::Inactive {
                return None;
            }
            match item {
                Item::ExternCrate(item) => Some(
                    item.rename
                        .as_ref()
                        .map_or(&item.ident, |(_, rename)| rename)
                        .to_string(),
                ),
                _ => None,
            }
        })
        .collect()
}

pub(super) fn declared_self_crate_aliases(
    file: &File,
    cfg_context: &CfgContext,
) -> BTreeSet<String> {
    file.items
        .iter()
        .filter_map(|item| {
            if cfg_context.evaluate_attributes(item_attributes(item)) == CfgDecision::Inactive {
                return None;
            }
            match item {
                Item::ExternCrate(item) if item.ident == "self" => {
                    item.rename.as_ref().map(|(_, rename)| rename.to_string())
                }
                _ => None,
            }
        })
        .collect()
}

pub(super) fn has_no_implicit_prelude(file: &File) -> bool {
    file.attrs
        .iter()
        .any(|attribute| attribute.path().is_ident("no_implicit_prelude"))
}

fn collect_use_bindings(tree: &UseTree, prefix: Vec<String>, names: &mut BTreeSet<String>) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            collect_use_bindings(&path.tree, next, names);
        }
        UseTree::Name(name) if name.ident == "self" => {
            if let Some(binding) = prefix.last() {
                names.insert(binding.clone());
            }
        }
        UseTree::Name(name) => {
            names.insert(name.ident.to_string());
        }
        UseTree::Rename(rename) => {
            names.insert(rename.rename.to_string());
        }
        UseTree::Glob(_) => {}
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_bindings(item, prefix.clone(), names);
            }
        }
    }
}

fn collect_use_bindings_with_paths(
    tree: &UseTree,
    prefix: Vec<String>,
    absolute_external: bool,
    bindings: &mut Vec<UseBinding>,
) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            collect_use_bindings_with_paths(&path.tree, next, absolute_external, bindings);
        }
        UseTree::Name(name) if name.ident == "self" => {
            if let Some(binding) = prefix.last() {
                bindings.push(UseBinding {
                    name: binding.clone(),
                    path: prefix,
                    absolute_external,
                });
            }
        }
        UseTree::Name(name) => {
            let mut path = prefix;
            let name = name.ident.to_string();
            path.push(name.clone());
            bindings.push(UseBinding {
                name,
                path,
                absolute_external,
            });
        }
        UseTree::Rename(rename) => {
            let mut path = prefix;
            path.push(rename.ident.to_string());
            bindings.push(UseBinding {
                name: rename.rename.to_string(),
                path,
                absolute_external,
            });
        }
        UseTree::Glob(_) => {}
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_bindings_with_paths(item, prefix.clone(), absolute_external, bindings);
            }
        }
    }
}

fn collect_use_glob_imports(
    tree: &UseTree,
    prefix: Vec<String>,
    absolute_external: bool,
    imports: &mut Vec<UseAlias>,
) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            collect_use_glob_imports(&path.tree, next, absolute_external, imports);
        }
        UseTree::Glob(_) => {
            let alias = UseAlias {
                path: prefix,
                absolute_external,
            };
            if !imports.contains(&alias) {
                imports.push(alias);
            }
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_glob_imports(item, prefix.clone(), absolute_external, imports);
            }
        }
        UseTree::Name(_) | UseTree::Rename(_) => {}
    }
}

fn collect_item_generic_bindings(item: &Item, bindings: &mut BTreeSet<String>) {
    let generics = match item {
        Item::Enum(item) => Some(&item.generics),
        Item::Fn(item) => Some(&item.sig.generics),
        Item::Impl(item) => Some(&item.generics),
        Item::Struct(item) => Some(&item.generics),
        Item::Trait(item) => Some(&item.generics),
        Item::TraitAlias(item) => Some(&item.generics),
        Item::Type(item) => Some(&item.generics),
        Item::Union(item) => Some(&item.generics),
        _ => None,
    };
    if let Some(generics) = generics {
        collect_generic_bindings(generics, bindings);
    }
}

fn collect_generic_bindings(generics: &syn::Generics, bindings: &mut BTreeSet<String>) {
    for parameter in &generics.params {
        match parameter {
            syn::GenericParam::Type(parameter) => {
                bindings.insert(parameter.ident.to_string());
            }
            syn::GenericParam::Const(parameter) => {
                bindings.insert(parameter.ident.to_string());
            }
            syn::GenericParam::Lifetime(_) => {}
        }
    }
}

pub(super) fn use_aliases(
    file: &File,
    cfg_context: &CfgContext,
) -> BTreeMap<String, Vec<UseAlias>> {
    let mut aliases = BTreeMap::new();
    for item in &file.items {
        if let Item::Use(item) = item
            && cfg_context.evaluate_attributes(&item.attrs) != CfgDecision::Inactive
        {
            collect_use_aliases(
                &item.tree,
                Vec::new(),
                item.leading_colon.is_some(),
                &mut aliases,
            );
        }
    }
    aliases
}

fn collect_use_aliases(
    tree: &UseTree,
    prefix: Vec<String>,
    absolute_external: bool,
    aliases: &mut BTreeMap<String, Vec<UseAlias>>,
) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            collect_use_aliases(&path.tree, next, absolute_external, aliases);
        }
        UseTree::Name(name) if name.ident == "self" => {
            if let Some(binding) = prefix.last() {
                insert_use_alias(
                    aliases,
                    binding.clone(),
                    UseAlias {
                        path: prefix,
                        absolute_external,
                    },
                );
            }
        }
        UseTree::Name(name) => {
            let mut path = prefix;
            path.push(name.ident.to_string());
            insert_use_alias(
                aliases,
                name.ident.to_string(),
                UseAlias {
                    path,
                    absolute_external,
                },
            );
        }
        UseTree::Rename(rename) => {
            let mut path = prefix;
            path.push(rename.ident.to_string());
            insert_use_alias(
                aliases,
                rename.rename.to_string(),
                UseAlias {
                    path,
                    absolute_external,
                },
            );
        }
        UseTree::Glob(_) => {}
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_aliases(item, prefix.clone(), absolute_external, aliases);
            }
        }
    }
}

fn insert_use_alias(
    aliases: &mut BTreeMap<String, Vec<UseAlias>>,
    binding: String,
    alias: UseAlias,
) {
    let candidates = aliases.entry(binding).or_default();
    if !candidates.contains(&alias) {
        candidates.push(alias);
    }
}

pub(super) fn has_path_attribute(attributes: &[Attribute], cfg_context: &CfgContext) -> bool {
    attributes
        .iter()
        .any(|attribute| cfg_context.has_active_path_attribute(attribute))
}

pub(super) fn path_to_string(path: &[String]) -> String {
    path.join("::")
}

pub(super) fn syn_path_segments(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::path_to_string;

    #[test]
    fn should_join_path_segments_with_the_rust_separator() {
        // Arrange
        let path = vec!["crate".to_owned(), "application".to_owned()];

        // Act
        let value = path_to_string(&path);

        // Assert
        assert_eq!(value, "crate::application");
    }
}
