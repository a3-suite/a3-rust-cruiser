use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::LineColumn;
use syn::Item;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::visit_mut::VisitMut;

use super::{ModuleRecord, ModuleVariant};
use crate::rust_dependency_observation::{
    cfg_context::{CfgContext, CfgDecision},
    rust_syntax::{
        SupportedItemMacroDefinition, declared_external_crate_aliases, declared_item_names,
        declared_self_crate_aliases, expand_supported_expression_macro,
        expand_supported_item_macro, glob_imports, has_no_implicit_prelude, is_crate_visible,
        local_macro_bindings, public_glob_imports, public_item_names, public_use_bindings,
        resolves_to_standard_thread_local, supported_item_macro_definition, syn_path_segments,
        thread_local_declarations, use_aliases,
    },
};

/// Matches rustc's default `recursion_limit`, which also bounds recursive
/// internal rules such as `@helper` arms.
const RECURSION_LIMIT: usize = 128;

#[derive(Clone)]
struct RegisteredItemMacro {
    module_id: String,
    definition_start: LineColumn,
    definition: SupportedItemMacroDefinition,
}

struct ItemMacroRegistry {
    /// Unsupported definitions are kept as `None` so that a later same-name
    /// definition still shadows a supported one.
    definitions: BTreeMap<(String, String), Vec<Option<RegisteredItemMacro>>>,
    /// Maps an exported name to the definition it re-exports, which may be
    /// renamed (`pub(crate) use actual as exposed;`).
    exports: BTreeMap<(String, String), BTreeSet<String>>,
}

pub(super) fn expand_supported_item_macros(
    records: &mut BTreeMap<String, ModuleRecord>,
    cfg_context: &CfgContext,
    external_crates: &BTreeSet<String>,
) {
    let registry = ItemMacroRegistry::collect(records);
    for (module_id, record) in records {
        for variant in &mut record.variants {
            let items = std::mem::take(&mut variant.ast.items);
            variant.ast.items =
                expand_module_items(module_id, variant, items, &registry, external_crates);
            refresh_variant_indexes(variant, cfg_context);
        }
    }
}

impl ItemMacroRegistry {
    fn collect(records: &BTreeMap<String, ModuleRecord>) -> Self {
        let mut definitions: BTreeMap<(String, String), Vec<Option<RegisteredItemMacro>>> =
            BTreeMap::new();
        for (module_id, record) in records {
            for variant in &record.variants {
                for item in &variant.ast.items {
                    let Item::Macro(item_macro) = item else {
                        continue;
                    };
                    let Some(name) = item_macro.ident.as_ref().map(ToString::to_string) else {
                        continue;
                    };
                    if !item_macro.mac.path.is_ident("macro_rules") {
                        continue;
                    }
                    definitions
                        .entry((module_id.clone(), name))
                        .or_default()
                        .push(
                            supported_item_macro_definition(item_macro).map(|definition| {
                                RegisteredItemMacro {
                                    module_id: module_id.clone(),
                                    definition_start: item_macro.span().start(),
                                    definition,
                                }
                            }),
                        );
                }
            }
        }
        let mut exports: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
        for (module_id, record) in records {
            for variant in &record.variants {
                for binding in &variant.public_use_bindings {
                    if binding.path.len() == 1
                        && definitions.contains_key(&(module_id.clone(), binding.path[0].clone()))
                    {
                        exports
                            .entry((module_id.clone(), binding.name.clone()))
                            .or_default()
                            .insert(binding.path[0].clone());
                    }
                }
            }
        }
        Self {
            definitions,
            exports,
        }
    }

    fn exported_definition(&self, module_id: &str, name: &str) -> Option<String> {
        let names = self.exports.get(&(module_id.to_owned(), name.to_owned()))?;
        let [definition] = names.iter().collect::<Vec<_>>()[..] else {
            return None;
        };
        Some(definition.clone())
    }

    fn is_defined_outside(&self, name: &str, module_id: &str) -> bool {
        self.definitions
            .keys()
            .any(|(module, defined)| defined == name && module != module_id)
    }

    fn resolve(
        &self,
        current_module: &str,
        variant: &ModuleVariant,
        path: &syn::Path,
    ) -> Option<&RegisteredItemMacro> {
        if path.leading_colon.is_some() {
            return None;
        }
        let segments = syn_path_segments(path);
        let name = segments.last()?.clone();
        let mut candidates = Vec::new();
        if segments.len() == 1 {
            if self
                .definitions
                .contains_key(&(current_module.to_owned(), name.clone()))
            {
                candidates.push((current_module.to_owned(), name.clone()));
            }
            for alias in variant.use_aliases.get(&name).into_iter().flatten() {
                if alias.absolute_external || alias.path.len() < 2 {
                    continue;
                }
                let exported_name = alias.path.last()?;
                let target_module =
                    resolve_macro_module(current_module, &alias.path[..alias.path.len() - 1])?;
                if let Some(definition) = self.exported_definition(&target_module, exported_name) {
                    candidates.push((target_module, definition));
                }
            }
        } else {
            let target_module =
                resolve_macro_module(current_module, &segments[..segments.len() - 1])?;
            // A qualified path reaches a macro_rules only through a use binding,
            // even within the defining module.
            let definition = self.exported_definition(&target_module, &name)?;
            candidates.push((target_module, definition));
        }
        let [candidate] = candidates.as_slice() else {
            return None;
        };
        match self.definitions.get(candidate)?.as_slice() {
            [Some(definition)] => Some(definition),
            _ => None,
        }
    }
}

fn resolve_macro_module(current_module: &str, path: &[String]) -> Option<String> {
    if path.is_empty() {
        return Some(current_module.to_owned());
    }
    let mut module = current_module
        .split("::")
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut index = 0;
    match path.first().map(String::as_str) {
        Some("crate") => {
            module.truncate(1);
            index = 1;
        }
        Some("self") => index = 1,
        Some("super") => {
            while path.get(index).is_some_and(|segment| segment == "super") {
                if module.len() == 1 {
                    return None;
                }
                module.pop();
                index += 1;
            }
        }
        _ => return None,
    }
    module.extend(path[index..].iter().cloned());
    Some(module.join("::"))
}

fn expand_module_items(
    module_id: &str,
    variant: &ModuleVariant,
    items: Vec<Item>,
    registry: &ItemMacroRegistry,
    external_crates: &BTreeSet<String>,
) -> Vec<Item> {
    let mut expanded = Vec::new();
    for item in items {
        let Item::Macro(invocation) = &item else {
            expanded.push(item);
            continue;
        };
        if invocation.mac.path.is_ident("macro_rules")
            || !invocation.attrs.is_empty()
            || invocation.ident.is_some()
        {
            expanded.push(item);
            continue;
        }
        let mut depth = 0;
        if let Some(generated) = expand_item_macro_invocation(
            module_id,
            variant,
            invocation,
            registry,
            external_crates,
            invocation.span().start(),
            &mut depth,
        ) {
            expanded.extend(generated);
        } else {
            expanded.push(item);
        }
    }
    expanded
}

/// Resolves an invocation the way rustc would choose its macro_rules
/// definition, and returns the span that generated tokens should carry.
fn resolve_invocation<'a>(
    module_id: &str,
    variant: &ModuleVariant,
    path: &syn::Path,
    invocation_span: proc_macro2::Span,
    registry: &'a ItemMacroRegistry,
    outer_span: LineColumn,
) -> Option<(&'a RegisteredItemMacro, Option<proc_macro2::Span>)> {
    let resolved = registry.resolve(module_id, variant, path)?;
    if let Some(name) = path.get_ident() {
        let name = name.to_string();
        if resolved.module_id == module_id {
            // A local definition is in textual scope only after its position.
            let start = resolved.definition_start;
            if (start.line, start.column) >= (outer_span.line, outer_span.column) {
                return None;
            }
        } else if registry.is_defined_outside(&name, &resolved.module_id) {
            // Another macro_rules of this name could shadow the import through
            // textual scope from an ancestor module.
            return None;
        }
    }
    // Paths in a macro_rules transcriber resolve at the call site, so the
    // invoking module is the observation scope in both cases. Spans stay real
    // only when the definition is in the same file as the invocation.
    let call_site = (resolved.module_id != module_id).then_some(invocation_span);
    Some((resolved, call_site))
}

fn expand_item_macro_invocation(
    module_id: &str,
    variant: &ModuleVariant,
    invocation: &syn::ItemMacro,
    registry: &ItemMacroRegistry,
    external_crates: &BTreeSet<String>,
    outer_span: LineColumn,
    depth: &mut usize,
) -> Option<Vec<Item>> {
    let (resolved, call_site) = resolve_invocation(
        module_id,
        variant,
        &invocation.mac.path,
        invocation.span(),
        registry,
        outer_span,
    )?;
    if *depth >= RECURSION_LIMIT {
        return None;
    }
    *depth += 1;
    let mut generated = expand_supported_item_macro(
        &resolved.definition,
        invocation.mac.tokens.clone(),
        call_site,
    )?;
    let imports = generated_scope(&generated).imports;
    let mut expressions = ExpressionMacroExpander {
        module_id,
        variant,
        registry,
        outer_span,
        depth,
        bare_names_resolvable: !imports,
        failed: false,
    };
    for item in &mut generated {
        expressions.visit_item_mut(item);
    }
    if expressions.failed {
        return None;
    }
    // Module records are discovered before expansion, so a generated module
    // body at any depth would otherwise escape observation. Generated
    // macro_rules definitions would shadow registry definitions in their own
    // lexical scope, which the registry does not model.
    let scope = generated_scope(&generated);
    if scope.defines_module || scope.defines_macro {
        return None;
    }
    let generated_names = generated
        .iter()
        .filter_map(generated_item_name)
        .collect::<BTreeSet<_>>();
    let mut fully_expanded = Vec::new();
    for item in generated {
        if let Item::Macro(nested) = &item {
            let path = syn_path_segments(&nested.mac.path);
            if path == ["inventory", "submit"] {
                let submitted = nested.mac.tokens.clone().into_iter().collect::<Vec<_>>();
                let [proc_macro2::TokenTree::Ident(slot)] = submitted.as_slice() else {
                    return None;
                };
                if !external_crates.contains("inventory")
                    || !generated_names.contains(&slot.to_string())
                {
                    return None;
                }
                fully_expanded.push(item);
                continue;
            }
            // Bare names would resolve through generated imports, which the
            // pre-expansion alias index used by the registry does not contain.
            if scope.imports && nested.mac.path.get_ident().is_some() {
                return None;
            }
            let nested_items = expand_item_macro_invocation(
                module_id,
                variant,
                nested,
                registry,
                external_crates,
                outer_span,
                depth,
            )?;
            fully_expanded.extend(nested_items);
        } else {
            fully_expanded.push(item);
        }
    }
    *depth -= 1;
    Some(fully_expanded)
}

/// Expands expression-position invocations inside generated items. Macros
/// that do not resolve to a registered definition stay for the visitor, which
/// reports them unless they are known; a registered one that cannot expand
/// rejects the whole expansion.
struct ExpressionMacroExpander<'a> {
    module_id: &'a str,
    variant: &'a ModuleVariant,
    registry: &'a ItemMacroRegistry,
    outer_span: LineColumn,
    depth: &'a mut usize,
    bare_names_resolvable: bool,
    failed: bool,
}

impl VisitMut for ExpressionMacroExpander<'_> {
    fn visit_expr_mut(&mut self, expr: &mut syn::Expr) {
        if self.failed {
            return;
        }
        if let syn::Expr::Macro(invocation) = expr
            && invocation.mac.path.get_ident().is_some()
            && !self.bare_names_resolvable
        {
            self.failed = true;
            return;
        }
        if let syn::Expr::Macro(invocation) = expr
            && let Some((resolved, call_site)) = resolve_invocation(
                self.module_id,
                self.variant,
                &invocation.mac.path,
                invocation.span(),
                self.registry,
                self.outer_span,
            )
        {
            let expanded = (*self.depth < RECURSION_LIMIT)
                .then(|| {
                    expand_supported_expression_macro(
                        &resolved.definition,
                        invocation.mac.tokens.clone(),
                        call_site,
                    )
                })
                .flatten();
            let Some(expanded) = expanded else {
                self.failed = true;
                return;
            };
            *expr = expanded;
            let mut scope = GeneratedScope::default();
            scope.visit_expr(expr);
            self.bare_names_resolvable &= !scope.imports;
            *self.depth += 1;
            self.visit_expr_mut(expr);
            *self.depth -= 1;
            return;
        }
        syn::visit_mut::visit_expr_mut(self, expr);
    }
}

/// Names are registered only for invocations proven to be the std macro, so a
/// same-named custom macro cannot make other modules' paths resolve here.
pub(super) fn register_thread_local_names(
    records: &mut BTreeMap<String, ModuleRecord>,
    cfg_context: &CfgContext,
    crate_macro_rules_names: &BTreeSet<String>,
) {
    for record in records.values_mut() {
        for variant in &mut record.variants {
            let prelude_visible = variant.glob_imports.is_empty() && !variant.no_implicit_prelude;
            let mut declared = Vec::new();
            let mut public = Vec::new();
            for item in &variant.ast.items {
                let Item::Macro(item) = item else {
                    continue;
                };
                if cfg_context.evaluate_attributes(&item.attrs) == CfgDecision::Inactive {
                    continue;
                }
                let Some(declarations) = thread_local_declarations(&item.mac) else {
                    continue;
                };
                if !resolves_to_standard_thread_local(
                    &item.mac.path,
                    prelude_visible,
                    crate_macro_rules_names,
                    |name| {
                        variant.use_aliases.contains_key(name)
                            || variant.declared_items.contains(name)
                    },
                ) {
                    continue;
                }
                for declaration in declarations {
                    let name = declaration.ident.to_string();
                    if is_crate_visible(&declaration.vis) {
                        public.push(name.clone());
                    }
                    declared.push(name);
                }
            }
            variant.declared_items.extend(declared);
            variant.public_items.extend(public);
        }
    }
}

#[derive(Default)]
struct GeneratedScope {
    defines_module: bool,
    defines_macro: bool,
    imports: bool,
}

impl<'ast> Visit<'ast> for GeneratedScope {
    fn visit_item_mod(&mut self, _: &'ast syn::ItemMod) {
        self.defines_module = true;
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        self.defines_macro |= item.mac.path.is_ident("macro_rules");
        syn::visit::visit_item_macro(self, item);
    }

    fn visit_item_use(&mut self, _: &'ast syn::ItemUse) {
        self.imports = true;
    }
}

fn generated_scope(items: &[Item]) -> GeneratedScope {
    let mut scope = GeneratedScope::default();
    for item in items {
        scope.visit_item(item);
    }
    scope
}

fn generated_item_name(item: &Item) -> Option<String> {
    match item {
        Item::Const(item) => Some(item.ident.to_string()),
        Item::Fn(item) => Some(item.sig.ident.to_string()),
        _ => None,
    }
}

fn refresh_variant_indexes(variant: &mut ModuleVariant, cfg_context: &CfgContext) {
    variant.declared_items = declared_item_names(&variant.ast, cfg_context);
    variant.public_items = public_item_names(&variant.ast, cfg_context);
    variant.public_use_bindings = public_use_bindings(&variant.ast, cfg_context);
    variant.use_aliases = use_aliases(&variant.ast, cfg_context);
    variant.glob_imports = glob_imports(&variant.ast, cfg_context);
    variant.public_glob_imports = public_glob_imports(&variant.ast, cfg_context);
    variant.local_macro_bindings = local_macro_bindings(&variant.ast, cfg_context);
    variant.external_crate_aliases = declared_external_crate_aliases(&variant.ast, cfg_context);
    variant.self_crate_aliases = declared_self_crate_aliases(&variant.ast, cfg_context);
    variant.no_implicit_prelude = has_no_implicit_prelude(&variant.ast);
}
