use std::collections::{BTreeMap, BTreeSet};

use syn::{Attribute, File, Item, UseTree};

use super::super::cfg_context::{CfgContext, CfgDecision};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UseAlias {
    pub(crate) path: Vec<String>,
    pub(crate) absolute_external: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UseBinding {
    pub(crate) name: String,
    pub(crate) path: Vec<String>,
    pub(crate) absolute_external: bool,
}

pub(crate) fn item_attributes(item: &Item) -> &[Attribute] {
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

pub(crate) fn flatten_use_tree(tree: &UseTree, prefix: Vec<String>, paths: &mut Vec<Vec<String>>) {
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

pub(crate) fn declared_item_names(file: &File, cfg_context: &CfgContext) -> BTreeSet<String> {
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

pub(crate) fn public_item_names(file: &File, cfg_context: &CfgContext) -> BTreeSet<String> {
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

pub(crate) fn public_use_bindings(file: &File, cfg_context: &CfgContext) -> Vec<UseBinding> {
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

pub(crate) fn glob_imports(file: &File, cfg_context: &CfgContext) -> Vec<UseAlias> {
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

pub(crate) fn public_glob_imports(file: &File, cfg_context: &CfgContext) -> Vec<UseAlias> {
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

pub(crate) fn is_crate_visible(visibility: &syn::Visibility) -> bool {
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

pub(super) fn declared_item_name(item: &Item) -> Option<String> {
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

pub(crate) fn declared_macro_bindings(file: &File, cfg_context: &CfgContext) -> BTreeSet<String> {
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

pub(crate) fn local_macro_bindings(file: &File, cfg_context: &CfgContext) -> BTreeSet<String> {
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

pub(crate) fn declared_external_crate_aliases(
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

pub(crate) fn declared_self_crate_aliases(
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

pub(crate) fn has_no_implicit_prelude(file: &File) -> bool {
    file.attrs
        .iter()
        .any(|attribute| attribute.path().is_ident("no_implicit_prelude"))
}

pub(super) fn collect_use_bindings(
    tree: &UseTree,
    prefix: Vec<String>,
    names: &mut BTreeSet<String>,
) {
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

pub(super) fn collect_use_bindings_with_paths(
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

pub(super) fn collect_use_glob_imports(
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

pub(super) fn collect_item_generic_bindings(item: &Item, bindings: &mut BTreeSet<String>) {
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

pub(super) fn collect_generic_bindings(generics: &syn::Generics, bindings: &mut BTreeSet<String>) {
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

pub(crate) fn use_aliases(
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

pub(super) fn collect_use_aliases(
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

pub(crate) fn has_path_attribute(attributes: &[Attribute], cfg_context: &CfgContext) -> bool {
    attributes
        .iter()
        .any(|attribute| cfg_context.has_active_path_attribute(attribute))
}

pub(crate) fn path_to_string(path: &[String]) -> String {
    path.join("::")
}

pub(crate) fn syn_path_segments(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::flatten_use_tree;

    #[test]
    fn should_flatten_nested_use_tree_paths_without_losing_prefix() {
        // Arrange
        let item: syn::ItemUse =
            syn::parse_str("use crate::api::{self, Client as ClientAlias, prelude::*};")
                .expect("the use tree fixture must parse");
        let mut paths = Vec::new();

        // Act
        flatten_use_tree(&item.tree, Vec::new(), &mut paths);

        // Assert
        assert_eq!(
            paths,
            vec![
                vec!["crate", "api", "self"],
                vec!["crate", "api", "Client"],
                vec!["crate", "api", "prelude"],
            ]
        );
    }
}
