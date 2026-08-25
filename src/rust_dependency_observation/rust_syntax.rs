use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{Delimiter, Group, LineColumn, TokenStream, TokenTree};
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

#[derive(Clone)]
pub(super) struct ObservedPath {
    pub(super) path: syn::Path,
    pub(super) local_bindings: BTreeSet<String>,
    pub(super) aliases: BTreeMap<String, Vec<UseAlias>>,
}

#[derive(Clone)]
pub(super) struct ScopedUse {
    pub(super) item: ItemUse,
    pub(super) aliases: BTreeMap<String, Vec<UseAlias>>,
}

fn known_builtin_attribute(path: &syn::Path) -> bool {
    path.get_ident().is_some_and(|name| {
        matches!(
            name.to_string().as_str(),
            "allow"
                | "cold"
                | "deprecated"
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
    pub(super) paths: Vec<ObservedPath>,
    pub(super) scoped_uses: Vec<ScopedUse>,
    pub(super) local_bindings: BTreeSet<String>,
    pub(super) aliases: BTreeMap<String, Vec<UseAlias>>,
    pub(super) macro_bindings: BTreeSet<String>,
    pub(super) inactive_expr_cfg: bool,
    pub(super) in_expr: bool,
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
        collect_item_generic_bindings(item, &mut self.local_bindings);
        syn::visit::visit_item(self, item);
        self.local_bindings = outer_bindings;
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        match self.cfg_context.evaluate_attributes(&local.attrs) {
            CfgDecision::Inactive => return,
            CfgDecision::Unknown => self.cfg_spans.push(local.span().start()),
            CfgDecision::Active => {}
        }
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
        self.paths.push(ObservedPath {
            path: path.clone(),
            local_bindings: self.local_bindings.clone(),
            aliases: self.aliases.clone(),
        });
        syn::visit::visit_path(self, path);
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        let outer_bindings = self.local_bindings.clone();
        let outer_aliases = self.aliases.clone();
        let outer_macro_bindings = self.macro_bindings.clone();
        for statement in &block.stmts {
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
                    collect_use_bindings(&item.tree, Vec::new(), &mut self.macro_bindings);
                }
                if let Item::Macro(item) = item
                    && let Some(ident) = &item.ident
                {
                    self.macro_bindings.insert(ident.to_string());
                }
                if let Some(name) = declared_item_name(item) {
                    self.local_bindings.insert(name);
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
        self.macro_bindings = outer_macro_bindings;
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

    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        if let Stmt::Macro(statement_macro) = statement {
            match self.cfg_context.evaluate_attributes(&statement_macro.attrs) {
                CfgDecision::Inactive => return,
                CfgDecision::Unknown => self.cfg_spans.push(statement_macro.span().start()),
                CfgDecision::Active => {}
            }
        }
        syn::visit::visit_stmt(self, statement);
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
                .is_some_and(|name| self.macro_bindings.contains(name))
        {
            self.macro_spans.push(macro_item.span().start());
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
        } else if !known_builtin_attribute(attribute.path()) {
            self.macro_spans.push(attribute.span().start());
        }
        syn::visit::visit_attribute(self, attribute);
    }
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
