use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::LineColumn;
use syn::Item;
use syn::spanned::Spanned;

use super::{ModuleRecord, ModuleVariant};
use crate::rust_dependency_observation::{
    cfg_context::CfgContext,
    rust_syntax::{
        SupportedItemMacroDefinition, declared_external_crate_aliases, declared_item_names,
        declared_self_crate_aliases, expand_supported_item_macro, glob_imports,
        has_no_implicit_prelude, local_macro_bindings, public_glob_imports, public_item_names,
        public_use_bindings, supported_item_macro_definition, syn_path_segments, use_aliases,
    },
};

#[derive(Clone)]
struct RegisteredItemMacro {
    module_id: String,
    definition: SupportedItemMacroDefinition,
}

struct ItemMacroRegistry {
    definitions: BTreeMap<(String, String), Vec<RegisteredItemMacro>>,
    exports: BTreeSet<(String, String)>,
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
        let mut definitions: BTreeMap<(String, String), Vec<RegisteredItemMacro>> = BTreeMap::new();
        for (module_id, record) in records {
            for variant in &record.variants {
                for item in &variant.ast.items {
                    let Item::Macro(item_macro) = item else {
                        continue;
                    };
                    let Some(name) = item_macro.ident.as_ref().map(ToString::to_string) else {
                        continue;
                    };
                    let Some(definition) = supported_item_macro_definition(item_macro) else {
                        continue;
                    };
                    definitions
                        .entry((module_id.clone(), name))
                        .or_default()
                        .push(RegisteredItemMacro {
                            module_id: module_id.clone(),
                            definition,
                        });
                }
            }
        }
        let mut exports = BTreeSet::new();
        for (module_id, record) in records {
            for variant in &record.variants {
                for binding in &variant.public_use_bindings {
                    if binding.path.len() == 1
                        && definitions.contains_key(&(module_id.clone(), binding.path[0].clone()))
                    {
                        exports.insert((module_id.clone(), binding.name.clone()));
                    }
                }
            }
        }
        Self {
            definitions,
            exports,
        }
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
                let macro_name = alias.path.last()?.clone();
                let target_module =
                    resolve_macro_module(current_module, &alias.path[..alias.path.len() - 1])?;
                if self
                    .exports
                    .contains(&(target_module.clone(), macro_name.clone()))
                    && self
                        .definitions
                        .contains_key(&(target_module.clone(), macro_name.clone()))
                {
                    candidates.push((target_module, macro_name));
                }
            }
        } else {
            let target_module =
                resolve_macro_module(current_module, &segments[..segments.len() - 1])?;
            if target_module != current_module
                && !self
                    .exports
                    .contains(&(target_module.clone(), name.clone()))
            {
                return None;
            }
            candidates.push((target_module, name));
        }
        let [candidate] = candidates.as_slice() else {
            return None;
        };
        let definitions = self.definitions.get(candidate)?;
        (definitions.len() == 1).then_some(&definitions[0])
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
        let mut stack = Vec::new();
        if let Some(generated) = expand_item_macro_invocation(
            module_id,
            variant,
            invocation,
            registry,
            external_crates,
            invocation.span().start(),
            &mut stack,
        ) {
            expanded.extend(generated);
        } else {
            expanded.push(item);
        }
    }
    expanded
}

fn expand_item_macro_invocation(
    module_id: &str,
    variant: &ModuleVariant,
    invocation: &syn::ItemMacro,
    registry: &ItemMacroRegistry,
    external_crates: &BTreeSet<String>,
    _outer_span: LineColumn,
    stack: &mut Vec<(String, String)>,
) -> Option<Vec<Item>> {
    let resolved = registry.resolve(module_id, variant, &invocation.mac.path)?;
    let name = invocation.mac.path.segments.last()?.ident.to_string();
    let identity = (resolved.module_id.clone(), name);
    if stack.contains(&identity) {
        return None;
    }
    stack.push(identity);
    let generated = expand_supported_item_macro(
        &resolved.definition,
        invocation.mac.tokens.clone(),
        invocation.span(),
    )?;
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
            let nested_items = expand_item_macro_invocation(
                module_id,
                variant,
                nested,
                registry,
                external_crates,
                _outer_span,
                stack,
            )?;
            fully_expanded.extend(nested_items);
        } else {
            fully_expanded.push(item);
        }
    }
    stack.pop();
    Some(fully_expanded)
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
