use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::LineColumn;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Attribute, File, Item, ItemMod, ItemUse, UseTree};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct UseAlias {
    pub(super) path: Vec<String>,
    pub(super) absolute_external: bool,
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

pub(super) struct PathVisitor {
    pub(super) paths: Vec<syn::Path>,
    pub(super) macro_spans: Vec<LineColumn>,
    pub(super) cfg_spans: Vec<LineColumn>,
    pub(super) strict_path_spans: Vec<LineColumn>,
    pub(super) use_spans: Vec<LineColumn>,
}

impl<'ast> Visit<'ast> for PathVisitor {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.paths.push(path.clone());
        syn::visit::visit_path(self, path);
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
        self.macro_spans.push(macro_item.span().start());
        syn::visit::visit_macro(self, macro_item);
    }

    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr") {
            self.cfg_spans.push(attribute.span().start());
        } else if attribute.path().is_ident("derive") || !known_builtin_attribute(attribute.path())
        {
            self.macro_spans.push(attribute.span().start());
        }
        syn::visit::visit_attribute(self, attribute);
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

pub(super) fn declared_item_names(file: &File) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for item in &file.items {
        let name = match item {
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
            Item::Use(item) => {
                collect_use_bindings(&item.tree, Vec::new(), &mut names);
                None
            }
            _ => None,
        };
        if let Some(name) = name {
            names.insert(name);
        }
    }
    names
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

pub(super) fn use_aliases(file: &File) -> BTreeMap<String, Vec<UseAlias>> {
    let mut aliases = BTreeMap::new();
    for item in &file.items {
        if let Item::Use(item) = item {
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

pub(super) fn has_path_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("path"))
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
