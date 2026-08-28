use std::collections::{BTreeMap, BTreeSet};

use super::{ModuleRecord, ModuleVariant, UseAlias, UseBinding};
use crate::boundary_validation::TargetScope;

pub(super) struct PathResolver<'a> {
    records: &'a BTreeMap<String, ModuleRecord>,
    external_crates: &'a BTreeSet<String>,
    self_crate_aliases: &'a BTreeSet<String>,
    exported_items: &'a BTreeMap<String, BTreeSet<String>>,
}

pub(super) struct ResolutionScope<'a> {
    pub(super) aliases: &'a BTreeMap<String, Vec<UseAlias>>,
    pub(super) glob_imports: &'a [UseAlias],
}

struct PathResolutionContext<'a> {
    source_module: &'a str,
    source_variant: &'a ModuleVariant,
    aliases: &'a BTreeMap<String, Vec<UseAlias>>,
    glob_imports: &'a [UseAlias],
    all_modules: &'a BTreeSet<String>,
    exported_items: &'a BTreeMap<String, BTreeSet<String>>,
}

impl<'a> PathResolver<'a> {
    pub(super) fn new(
        records: &'a BTreeMap<String, ModuleRecord>,
        external_crates: &'a BTreeSet<String>,
        self_crate_aliases: &'a BTreeSet<String>,
        exported_items: &'a BTreeMap<String, BTreeSet<String>>,
    ) -> Self {
        Self {
            records,
            external_crates,
            self_crate_aliases,
            exported_items,
        }
    }

    pub(super) fn may_reference_internal_module(
        &self,
        source_module: &str,
        aliases: &BTreeMap<String, Vec<UseAlias>>,
        glob_imports: &[UseAlias],
        path: &[String],
        all_modules: &BTreeSet<String>,
        alias_depth: usize,
    ) -> bool {
        let Some(first) = path.first() else {
            return false;
        };
        if matches!(first.as_str(), "crate" | "self" | "super") {
            return true;
        }
        if alias_depth <= self.records.len()
            && let Some(candidates) = aliases.get(first)
        {
            return candidates.iter().any(|alias| {
                let mut expanded = alias.path.clone();
                expanded.extend_from_slice(&path[1..]);
                !alias.absolute_external
                    && self.may_reference_internal_module(
                        source_module,
                        aliases,
                        glob_imports,
                        &expanded,
                        all_modules,
                        alias_depth + 1,
                    )
            });
        }
        if !glob_imports.is_empty() {
            return true;
        }
        let local = format!("{source_module}::{first}");
        let crate_level = format!("crate::{first}");
        all_modules.iter().any(|module| {
            module == &local
                || module.starts_with(&(local.clone() + "::"))
                || module == &crate_level
                || module.starts_with(&(crate_level.clone() + "::"))
        })
    }

    pub(super) fn candidate_targets_for_path(
        &self,
        source_id: &str,
        source_variant: &ModuleVariant,
        resolution_scope: &ResolutionScope<'_>,
        path: &[String],
        leading_colon: bool,
        all_modules: &BTreeSet<String>,
    ) -> Vec<String> {
        let mut candidates = BTreeSet::new();
        for end in (1..path.len()).rev() {
            for (target, scope) in self.resolve_path(
                source_id,
                source_variant,
                resolution_scope,
                &path[..end],
                leading_colon,
                all_modules,
            ) {
                if scope == TargetScope::InternalModule && target != source_id {
                    candidates.insert(target);
                }
            }
            if !candidates.is_empty() {
                break;
            }
        }
        if candidates.is_empty() {
            for glob in resolution_scope.glob_imports {
                for (target, target_scope) in self.resolve_path(
                    source_id,
                    source_variant,
                    &ResolutionScope {
                        aliases: resolution_scope.aliases,
                        glob_imports: &[],
                    },
                    &glob.path,
                    glob.absolute_external,
                    all_modules,
                ) {
                    if target_scope == TargetScope::InternalModule && target != source_id {
                        candidates.insert(target);
                    }
                }
            }
        }
        candidates.into_iter().collect()
    }

    pub(super) fn resolve_path(
        &self,
        source_module: &str,
        source_variant: &ModuleVariant,
        scope: &ResolutionScope<'_>,
        path: &[String],
        absolute_external: bool,
        all_modules: &BTreeSet<String>,
    ) -> Vec<(String, TargetScope)> {
        let context = PathResolutionContext {
            source_module,
            source_variant,
            aliases: scope.aliases,
            glob_imports: scope.glob_imports,
            all_modules,
            exported_items: self.exported_items,
        };
        self.resolve_path_with_exports(&context, path, absolute_external)
    }

    fn resolve_path_with_exports(
        &self,
        context: &PathResolutionContext<'_>,
        path: &[String],
        absolute_external: bool,
    ) -> Vec<(String, TargetScope)> {
        resolve_path_inner(self, context, path, absolute_external, 0)
    }
}

fn resolve_path_inner(
    resolver: &PathResolver<'_>,
    context: &PathResolutionContext<'_>,
    path: &[String],
    absolute_external: bool,
    alias_depth: usize,
) -> Vec<(String, TargetScope)> {
    if path.is_empty() {
        return Vec::new();
    }
    if absolute_external {
        let first = &path[0];
        return if super::known_external_crate(first) || resolver.external_crates.contains(first) {
            vec![(first.clone(), TargetScope::ExternalCrate)]
        } else {
            Vec::new()
        };
    }
    if alias_depth > resolver.records.len() {
        return Vec::new();
    }
    if resolver.self_crate_aliases.contains(&path[0])
        || context.source_variant.self_crate_aliases.contains(&path[0])
    {
        let mut crate_path = vec!["crate".to_owned()];
        crate_path.extend_from_slice(&path[1..]);
        return resolve_path_inner(resolver, context, &crate_path, false, alias_depth + 1);
    }
    if let Some(candidates) = context.aliases.get(&path[0]) {
        return candidates
            .iter()
            .flat_map(|alias| {
                let mut expanded = alias.path.clone();
                expanded.extend_from_slice(&path[1..]);
                resolve_path_inner(
                    resolver,
                    context,
                    &expanded,
                    alias.absolute_external,
                    alias_depth + 1,
                )
            })
            .collect();
    }
    let (base, remainder) = match path[0].as_str() {
        "crate" => ("crate".to_owned(), &path[1..]),
        "self" => (context.source_module.to_owned(), &path[1..]),
        "super" => {
            let mut current = context.source_module.split("::").collect::<Vec<_>>();
            let super_count = path
                .iter()
                .take_while(|segment| segment.as_str() == "super")
                .count();
            if current.len() <= super_count {
                return Vec::new();
            }
            current.truncate(current.len() - super_count);
            (current.join("::"), &path[super_count..])
        }
        first => {
            let source_parts = context.source_module.split("::").collect::<Vec<_>>();
            let candidates = if source_parts.len() > 1 {
                vec![
                    format!("{}::{first}", context.source_module),
                    format!("crate::{first}"),
                ]
            } else {
                vec![format!("crate::{first}")]
            };
            if let Some(local) = candidates.into_iter().find(|candidate| {
                context.all_modules.iter().any(|module| {
                    module == candidate || module.starts_with(&(candidate.clone() + "::"))
                })
            }) {
                (local, &path[1..])
            } else if path.len() > 1
                && context
                    .source_variant
                    .external_crate_aliases
                    .contains(first)
            {
                return vec![(first.to_owned(), TargetScope::ExternalCrate)];
            } else if first == "Self" || context.source_variant.declared_items.contains(first) {
                return vec![(
                    context.source_module.to_owned(),
                    TargetScope::InternalModule,
                )];
            } else if !context.glob_imports.is_empty()
                && let Some(resolved) = resolve_glob_path(resolver, context, path, alias_depth)
            {
                return resolved;
            } else {
                let is_external_crate = super::known_external_crate(first)
                    || resolver.external_crates.contains(first)
                    || context
                        .source_variant
                        .external_crate_aliases
                        .contains(first);
                if path.len() == 1 || !is_external_crate {
                    return Vec::new();
                }
                return vec![(first.to_owned(), TargetScope::ExternalCrate)];
            }
        }
    };
    let mut candidate = base;
    if context.all_modules.contains(&candidate) {
        for (index, segment) in remainder.iter().enumerate() {
            let next = format!("{candidate}::{segment}");
            if context.all_modules.contains(&next) {
                candidate = next;
            } else {
                if segment == "self" && index + 1 == remainder.len() {
                    break;
                }
                let declared_item = resolver.records.get(&candidate).is_some_and(|record| {
                    record
                        .variants
                        .iter()
                        .any(|variant| variant.declared_items.contains(segment))
                }) || context
                    .exported_items
                    .get(&candidate)
                    .is_some_and(|items| items.contains(segment));
                return if declared_item || alias_depth > 0 {
                    vec![(candidate, TargetScope::InternalModule)]
                } else {
                    Vec::new()
                };
            }
        }
        return vec![(candidate, TargetScope::InternalModule)];
    }
    let external = candidate
        .split("::")
        .next()
        .unwrap_or(&candidate)
        .to_owned();
    vec![(external, TargetScope::ExternalCrate)]
}

fn resolve_glob_path(
    resolver: &PathResolver<'_>,
    context: &PathResolutionContext<'_>,
    path: &[String],
    alias_depth: usize,
) -> Option<Vec<(String, TargetScope)>> {
    if alias_depth >= resolver.records.len() {
        return None;
    }
    let mut resolved = Vec::new();
    let direct_context = PathResolutionContext {
        source_module: context.source_module,
        source_variant: context.source_variant,
        aliases: context.aliases,
        glob_imports: &[],
        all_modules: context.all_modules,
        exported_items: context.exported_items,
    };
    for glob in context.glob_imports {
        let mut expanded = glob.path.clone();
        expanded.extend_from_slice(path);
        resolved.extend(resolve_path_inner(
            resolver,
            &direct_context,
            &expanded,
            glob.absolute_external,
            0,
        ));
    }
    resolved.sort_by(|left, right| {
        left.0.cmp(&right.0).then_with(|| {
            let rank = |scope| match scope {
                TargetScope::InternalModule => 0,
                TargetScope::ExternalCrate => 1,
            };
            rank(left.1).cmp(&rank(right.1))
        })
    });
    resolved.dedup();
    (!resolved.is_empty()).then_some(resolved)
}

pub(super) fn build_export_index(
    records: &BTreeMap<String, ModuleRecord>,
    external_crates: &BTreeSet<String>,
    self_crate_aliases: &BTreeSet<String>,
    all_modules: &BTreeSet<String>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut exported_items = BTreeMap::new();
    let mut visiting = BTreeSet::new();
    for module_id in all_modules {
        collect_exported_items(
            records,
            external_crates,
            self_crate_aliases,
            module_id,
            all_modules,
            &mut exported_items,
            &mut visiting,
        );
    }
    exported_items
}

fn collect_exported_items(
    records: &BTreeMap<String, ModuleRecord>,
    external_crates: &BTreeSet<String>,
    self_crate_aliases: &BTreeSet<String>,
    module_id: &str,
    all_modules: &BTreeSet<String>,
    exported_items: &mut BTreeMap<String, BTreeSet<String>>,
    visiting: &mut BTreeSet<String>,
) -> BTreeSet<String> {
    if let Some(items) = exported_items.get(module_id) {
        return items.clone();
    }
    if !visiting.insert(module_id.to_owned()) {
        return BTreeSet::new();
    }
    let mut items = BTreeSet::new();
    if let Some(record) = records.get(module_id) {
        for variant in &record.variants {
            items.extend(variant.public_items.iter().cloned());
            for binding in &variant.public_use_bindings {
                if public_use_binding_is_visible(
                    records,
                    external_crates,
                    self_crate_aliases,
                    module_id,
                    variant,
                    binding,
                    all_modules,
                    exported_items,
                    visiting,
                ) {
                    items.insert(binding.name.clone());
                }
            }
            let glob_imports = variant.public_glob_imports.clone();
            for glob in glob_imports {
                let resolver =
                    PathResolver::new(records, external_crates, self_crate_aliases, exported_items);
                let context = PathResolutionContext {
                    source_module: module_id,
                    source_variant: variant,
                    aliases: &variant.use_aliases,
                    glob_imports: &variant.glob_imports,
                    all_modules,
                    exported_items,
                };
                let targets = resolver.resolve_path_with_exports(
                    &context,
                    &glob.path,
                    glob.absolute_external,
                );
                for (target, scope) in targets {
                    if scope == TargetScope::InternalModule && target != module_id {
                        items.extend(collect_exported_items(
                            records,
                            external_crates,
                            self_crate_aliases,
                            &target,
                            all_modules,
                            exported_items,
                            visiting,
                        ));
                    }
                }
            }
        }
    }
    visiting.remove(module_id);
    exported_items.insert(module_id.to_owned(), items.clone());
    items
}

#[allow(clippy::too_many_arguments)]
fn public_use_binding_is_visible(
    records: &BTreeMap<String, ModuleRecord>,
    external_crates: &BTreeSet<String>,
    self_crate_aliases: &BTreeSet<String>,
    module_id: &str,
    variant: &ModuleVariant,
    binding: &UseBinding,
    all_modules: &BTreeSet<String>,
    exported_items: &mut BTreeMap<String, BTreeSet<String>>,
    visiting: &mut BTreeSet<String>,
) -> bool {
    let Some(name) = binding.path.last() else {
        return false;
    };
    if binding.path.len() == 1 {
        return variant.public_items.contains(name);
    }
    let resolver = PathResolver::new(records, external_crates, self_crate_aliases, exported_items);
    let context = PathResolutionContext {
        source_module: module_id,
        source_variant: variant,
        aliases: &variant.use_aliases,
        glob_imports: &variant.glob_imports,
        all_modules,
        exported_items,
    };
    let parent_path = &binding.path[..binding.path.len() - 1];
    let targets =
        resolver.resolve_path_with_exports(&context, parent_path, binding.absolute_external);
    targets.into_iter().any(|(target, scope)| {
        if scope != TargetScope::InternalModule {
            return true;
        }
        if target == module_id && variant.public_items.contains(name) {
            return true;
        }
        if all_modules.contains(&format!("{target}::{name}")) {
            return true;
        }
        collect_exported_items(
            records,
            external_crates,
            self_crate_aliases,
            &target,
            all_modules,
            exported_items,
            visiting,
        )
        .contains(name)
    })
}
