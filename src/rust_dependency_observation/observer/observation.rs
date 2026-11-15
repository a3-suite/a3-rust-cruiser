use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use super::super::{cargo_project, cfg_context::CfgContext};
use super::{Observer, RustDependencyObserver, macro_expansion, path_resolution};
use crate::boundary_validation::{
    AnalysisContext, AnalysisIssue, AnalysisProfile, CruiserError, DependencyObserver,
    LogicalModule, ObservedDependencyGraph, Result, matches_pattern,
};

impl DependencyObserver for RustDependencyObserver {
    fn observe(
        &self,
        project_root: &Path,
        context: AnalysisContext,
    ) -> Result<(ObservedDependencyGraph, Vec<AnalysisIssue>)> {
        observe(project_root, context)
    }
}

fn observe(
    project_root: &Path,
    mut context: AnalysisContext,
) -> Result<(ObservedDependencyGraph, Vec<AnalysisIssue>)> {
    let project_root = &project_root
        .canonicalize()
        .map_err(|error| CruiserError::io("Cargo project root", error))?;
    context.features.sort();
    context.features.dedup();
    context.cfg.sort();
    context.cfg.dedup();
    let (package_root, manifest) = cargo_project::selected_manifest(project_root, &context)?;
    let cfg_context = CfgContext::resolve(project_root, &context);
    let (entry, target_issues) = cargo_project::target_entry(&package_root, &manifest, &context)?;
    let (mut external_crates, context_dependent_crates) =
        cargo_project::declared_external_crates(&manifest, &context, &cfg_context)?;
    let target_kind = context
        .compilation_target
        .split_once(':')
        .map_or(context.compilation_target.as_str(), |(kind, _)| kind);
    if target_kind != "lib" && cargo_project::has_library_target(&package_root, &manifest)? {
        // A binary/example/test target in the same package links the package
        // library under its crate name. The selected target remains the
        // observation boundary, so treat that library crate as external to
        // this graph rather than inventing a second source root.
        external_crates.insert(context.package.replace('-', "_"));
    }
    let mut observer = Observer {
        project_root: project_root.to_path_buf(),
        cfg_context,
        source_lines: BTreeMap::new(),
        external_crates,
        context_dependent_crates,
        self_crate_aliases: BTreeSet::new(),
        crate_macro_rules_names: BTreeSet::new(),
        records: BTreeMap::new(),
        exported_items: BTreeMap::new(),
        declarations: Vec::new(),
        issues: target_issues,
    };
    observer.discover_module("crate", &entry)?;
    if context.analysis_profile == AnalysisProfile::Compilation {
        macro_expansion::expand_supported_item_macros(
            &mut observer.records,
            &observer.cfg_context,
            &observer.external_crates,
        );
    }
    observer.self_crate_aliases = observer
        .records
        .get("crate")
        .into_iter()
        .flat_map(|record| record.variants.iter())
        .flat_map(|variant| variant.self_crate_aliases.iter().cloned())
        .collect();
    observer.crate_macro_rules_names = observer
        .records
        .values()
        .flat_map(|record| record.variants.iter())
        .flat_map(|variant| variant.local_macro_bindings.iter().cloned())
        .collect();
    macro_expansion::register_thread_local_names(
        &mut observer.records,
        &observer.cfg_context,
        &observer.crate_macro_rules_names,
    );

    let all_module_ids: BTreeSet<String> = observer.records.keys().cloned().collect();
    let exported_items = path_resolution::build_export_index(
        &observer.records,
        &observer.external_crates,
        &observer.self_crate_aliases,
        &all_module_ids,
    );
    observer.exported_items = exported_items;
    let excluded_scopes: Vec<String> = all_module_ids
        .iter()
        .filter(|id| {
            context
                .exclude
                .iter()
                .any(|pattern| matches_pattern(pattern, id))
        })
        .cloned()
        .collect();
    let selected: BTreeSet<String> = all_module_ids
        .iter()
        .filter(|id| {
            let included = context.include.is_empty()
                || context
                    .include
                    .iter()
                    .any(|pattern| matches_pattern(pattern, id));
            included && !excluded_scopes.contains(id)
        })
        .cloned()
        .collect();

    let mut dependencies = Vec::new();
    let record_ids: Vec<String> = observer.records.keys().cloned().collect();
    for record_id in record_ids {
        if !selected.contains(&record_id) {
            continue;
        }
        let (record_dependencies, record_issues) = observer.collect_dependencies(
            &record_id,
            &selected,
            &all_module_ids,
            context.analysis_profile,
        );
        dependencies.extend(record_dependencies);
        observer.issues.extend(record_issues);
    }
    dependencies.sort_by(|left, right| {
        (
            left.source.as_str(),
            left.target.as_str(),
            left.kind,
            left.evidence.path.as_str(),
            left.evidence.line,
            left.evidence.column,
        )
            .cmp(&(
                right.source.as_str(),
                right.target.as_str(),
                right.kind,
                right.evidence.path.as_str(),
                right.evidence.line,
                right.evidence.column,
            ))
    });
    dependencies.dedup_by(|left, right| {
        left.source == right.source
            && left.target == right.target
            && left.kind == right.kind
            && left.evidence == right.evidence
    });

    let modules = selected
        .iter()
        .map(|id| {
            let mut source_paths = observer
                .records
                .get(id)
                .into_iter()
                .flat_map(|record| record.variants.iter())
                .map(|variant| observer.relative_path(&variant.path))
                .collect::<Vec<_>>();
            source_paths.sort();
            source_paths.dedup();
            LogicalModule {
                id: id.clone(),
                source_paths,
            }
        })
        .collect();
    observer.declarations.sort_by(|left, right| {
        (left.parent.as_str(), left.child.as_str())
            .cmp(&(right.parent.as_str(), right.child.as_str()))
    });
    observer
        .declarations
        .dedup_by(|left, right| left.declaration_id == right.declaration_id);
    observer.issues.sort_by(AnalysisIssue::canonical_cmp);
    observer.issues.dedup();

    let graph = ObservedDependencyGraph {
        analysis_context: context,
        modules,
        module_declarations: observer.declarations,
        dependencies,
        excluded_scopes,
    };
    Ok((graph, observer.issues))
}
