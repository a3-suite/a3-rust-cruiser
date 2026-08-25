use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use proc_macro2::LineColumn;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{File, Item, ItemUse};

use super::{
    cargo_project,
    cfg_context::{CfgContext, CfgDecision},
    rust_syntax::{
        PathVisitor, UseAlias, declared_external_crate_aliases, declared_item_names,
        declared_macro_bindings, flatten_use_tree, has_no_implicit_prelude, has_path_attribute,
        item_attributes, path_to_string, syn_path_segments, use_aliases,
    },
};

use crate::boundary_validation::{
    AnalysisContext, AnalysisIssue, AnalysisProfile, CruiserError, Dependency, DependencyKind,
    DependencyObserver, LogicalModule, ModuleDeclaration, ObservedDependencyGraph, Result,
    SourceEvidence, TargetScope, matches_pattern,
};

pub struct RustDependencyObserver;

impl DependencyObserver for RustDependencyObserver {
    fn target_source(&self, project_root: &Path, context: &AnalysisContext) -> Result<PathBuf> {
        let manifest = cargo_project::load_manifest(project_root)?;
        cargo_project::target_entry(project_root, &manifest, context).map(|(path, _)| path)
    }

    fn observe(
        &self,
        project_root: &Path,
        context: AnalysisContext,
    ) -> Result<(ObservedDependencyGraph, Vec<AnalysisIssue>)> {
        observe(project_root, context)
    }
}

struct ModuleRecord {
    id: String,
    variants: Vec<ModuleVariant>,
}

struct ModuleVariant {
    path: PathBuf,
    module_directory: PathBuf,
    ast: File,
    declared_items: BTreeSet<String>,
    use_aliases: BTreeMap<String, Vec<UseAlias>>,
    external_crate_aliases: BTreeSet<String>,
    no_implicit_prelude: bool,
}

struct PathResolutionContext<'a> {
    source_module: &'a str,
    source_variant: &'a ModuleVariant,
    aliases: &'a BTreeMap<String, Vec<UseAlias>>,
    all_modules: &'a BTreeSet<String>,
}

struct Observer {
    project_root: PathBuf,
    cfg_context: CfgContext,
    external_crates: BTreeSet<String>,
    context_dependent_crates: BTreeSet<String>,
    records: BTreeMap<String, ModuleRecord>,
    declarations: Vec<ModuleDeclaration>,
    issues: Vec<AnalysisIssue>,
}

fn observe(
    project_root: &Path,
    mut context: AnalysisContext,
) -> Result<(ObservedDependencyGraph, Vec<AnalysisIssue>)> {
    context.features.sort();
    context.features.dedup();
    context.cfg.sort();
    context.cfg.dedup();
    let manifest = cargo_project::load_manifest(project_root)?;
    let cfg_context = CfgContext::resolve(&context);
    let (entry, target_issues) = cargo_project::target_entry(project_root, &manifest, &context)?;
    let (external_crates, context_dependent_crates) =
        cargo_project::declared_external_crates(&manifest, &context, &cfg_context)?;
    let mut observer = Observer {
        project_root: project_root.to_path_buf(),
        cfg_context,
        external_crates,
        context_dependent_crates,
        records: BTreeMap::new(),
        declarations: Vec::new(),
        issues: target_issues,
    };
    observer.discover_module("crate", &entry)?;

    let all_module_ids: BTreeSet<String> = observer.records.keys().cloned().collect();
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
        .cloned()
        .map(|id| LogicalModule { id })
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

impl Observer {
    fn discover_module(&mut self, id: &str, path: &Path) -> Result<()> {
        if self.records.contains_key(id) {
            return Ok(());
        }
        let source = fs::read_to_string(path).map_err(|error| {
            CruiserError::Io(std::io::Error::new(
                error.kind(),
                format!("{}: {error}", path.display()),
            ))
        })?;
        let ast = match syn::parse_file(&source) {
            Ok(ast) => ast,
            Err(error) => {
                let evidence = self.evidence(path, error.span().start());
                self.issues.push(AnalysisIssue::analysis_context_unresolved(
                    "analysis.unsupported-syntax",
                    format!("unable to parse {}: {error}", path.display()),
                    id,
                    Some(id.to_owned()),
                    Some(evidence),
                    all_dependency_kinds(),
                ));
                return Ok(());
            }
        };
        let variant = ModuleVariant {
            path: path.to_path_buf(),
            module_directory: module_directory(path),
            declared_items: declared_item_names(&ast, &self.cfg_context),
            use_aliases: use_aliases(&ast, &self.cfg_context),
            external_crate_aliases: declared_external_crate_aliases(&ast, &self.cfg_context),
            no_implicit_prelude: has_no_implicit_prelude(&ast),
            ast,
        };
        let parent_path = variant.path.clone();
        let parent_module_directory = variant.module_directory.clone();
        let items = variant.ast.items.clone();
        self.records.insert(
            id.to_owned(),
            ModuleRecord {
                id: id.to_owned(),
                variants: vec![variant],
            },
        );
        self.discover_child_items(id, &parent_path, &parent_module_directory, items)?;
        Ok(())
    }

    fn discover_child_items(
        &mut self,
        parent_id: &str,
        parent_path: &Path,
        parent_module_directory: &Path,
        items: Vec<Item>,
    ) -> Result<()> {
        for item in items {
            let cfg_decision = self.cfg_context.evaluate_attributes(item_attributes(&item));
            if cfg_decision == CfgDecision::Inactive {
                continue;
            }
            let Item::Mod(module) = item else {
                if cfg_decision == CfgDecision::Unknown {
                    self.issues.push(AnalysisIssue::analysis_context_unresolved(
                        "analysis.unsupported-analysis-context",
                        "unable to evaluate cfg condition for module declaration",
                        parent_id,
                        Some(parent_id.to_owned()),
                        Some(self.evidence(parent_path, item.span().start())),
                        all_dependency_kinds(),
                    ));
                }
                continue;
            };
            let child_id = format!("{parent_id}::{}", module.ident);
            let evidence = self.evidence(parent_path, module.span().start());
            if cfg_decision == CfgDecision::Unknown {
                self.issues.push(AnalysisIssue::analysis_context_unresolved(
                    "analysis.unsupported-analysis-context",
                    "unable to evaluate cfg condition for module declaration",
                    child_id.clone(),
                    Some(child_id.clone()),
                    Some(evidence.clone()),
                    all_dependency_kinds(),
                ));
            }
            self.declarations.push(ModuleDeclaration {
                declaration_id: format!("decl:{parent_id}:{}", module.ident),
                parent: parent_id.to_owned(),
                child: child_id.clone(),
                evidence: evidence.clone(),
            });
            if cfg_decision != CfgDecision::Unknown
                && has_path_attribute(&module.attrs, &self.cfg_context)
            {
                self.issues.push(AnalysisIssue::analysis_context_unresolved(
                    "analysis.unsupported-syntax",
                    "#[path] module declarations are not supported in the MVP",
                    child_id.clone(),
                    Some(child_id.clone()),
                    Some(evidence),
                    all_dependency_kinds(),
                ));
                continue;
            }
            if let Some((_, inline_items)) = module.content {
                let inline_file = File {
                    shebang: None,
                    attrs: module
                        .attrs
                        .iter()
                        .filter(|attribute| attribute.path().is_ident("no_implicit_prelude"))
                        .cloned()
                        .collect(),
                    items: inline_items,
                };
                let child_items = inline_file.items.clone();
                let child_module_directory = parent_module_directory.join(module.ident.to_string());
                let variant = ModuleVariant {
                    path: parent_path.to_path_buf(),
                    module_directory: child_module_directory.clone(),
                    declared_items: declared_item_names(&inline_file, &self.cfg_context),
                    use_aliases: use_aliases(&inline_file, &self.cfg_context),
                    external_crate_aliases: declared_external_crate_aliases(
                        &inline_file,
                        &self.cfg_context,
                    ),
                    no_implicit_prelude: has_no_implicit_prelude(&inline_file),
                    ast: inline_file,
                };
                if let Some(existing) = self.records.get_mut(&child_id) {
                    existing.variants.push(variant);
                } else {
                    self.records.insert(
                        child_id.clone(),
                        ModuleRecord {
                            id: child_id.clone(),
                            variants: vec![variant],
                        },
                    );
                }
                self.discover_child_items(
                    &child_id,
                    parent_path,
                    &child_module_directory,
                    child_items,
                )?;
            } else {
                let child_path =
                    resolve_module_file(parent_module_directory, &module.ident.to_string())
                        .ok_or_else(|| {
                            CruiserError::Io(std::io::Error::new(
                                std::io::ErrorKind::NotFound,
                                format!("module file not found for {child_id}"),
                            ))
                        })?;
                self.discover_module(&child_id, &child_path)?;
            }
        }
        Ok(())
    }

    fn collect_dependencies(
        &self,
        record_id: &str,
        selected: &BTreeSet<String>,
        all_modules: &BTreeSet<String>,
        analysis_profile: AnalysisProfile,
    ) -> (Vec<Dependency>, Vec<AnalysisIssue>) {
        let Some(record) = self.records.get(record_id) else {
            return (Vec::new(), Vec::new());
        };
        let mut dependencies = Vec::new();
        let mut issues = Vec::new();
        for variant in &record.variants {
            self.collect_variant_dependencies(
                &record.id,
                variant,
                selected,
                all_modules,
                analysis_profile,
                &mut dependencies,
                &mut issues,
            );
        }
        (dependencies, issues)
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_variant_dependencies(
        &self,
        source_id: &str,
        variant: &ModuleVariant,
        selected: &BTreeSet<String>,
        all_modules: &BTreeSet<String>,
        analysis_profile: AnalysisProfile,
        dependencies: &mut Vec<Dependency>,
        issues: &mut Vec<AnalysisIssue>,
    ) {
        for item in &variant.ast.items {
            if let Item::Use(use_item) = item
                && self.cfg_context.evaluate_attributes(&use_item.attrs) != CfgDecision::Inactive
            {
                let (use_dependencies, use_issues) = self.collect_use(
                    source_id,
                    variant,
                    use_item,
                    &variant.use_aliases,
                    selected,
                    all_modules,
                );
                dependencies.extend(use_dependencies);
                issues.extend(use_issues);
            }
        }
        let mut visitor = PathVisitor {
            cfg_context: &self.cfg_context,
            external_crates: &self.external_crates,
            paths: Vec::new(),
            scoped_uses: Vec::new(),
            local_bindings: BTreeSet::new(),
            aliases: variant.use_aliases.clone(),
            macro_bindings: declared_macro_bindings(&variant.ast, &self.cfg_context),
            inactive_expr_cfg: false,
            in_expr: false,
            macro_spans: Vec::new(),
            cfg_spans: Vec::new(),
            strict_path_spans: Vec::new(),
            use_spans: Vec::new(),
        };
        visitor.visit_file(&variant.ast);
        for scoped_use in &visitor.scoped_uses {
            let (use_dependencies, use_issues) = self.collect_use(
                source_id,
                variant,
                &scoped_use.item,
                &scoped_use.aliases,
                selected,
                all_modules,
            );
            dependencies.extend(use_dependencies);
            issues.extend(use_issues);
        }
        let strict_path_spans = visitor.strict_path_spans.clone();
        let top_level_use_spans: Vec<LineColumn> = variant
            .ast
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Use(use_item) => Some(use_item.span().start()),
                _ => None,
            })
            .collect();
        let _block_scoped_use_spans = visitor
            .use_spans
            .iter()
            .filter(|span| !top_level_use_spans.contains(span))
            .collect::<Vec<_>>();
        if analysis_profile == AnalysisProfile::Compilation {
            for macro_span in visitor.macro_spans {
                issues.push(AnalysisIssue::unresolved_dependency_candidate(
                    "analysis.unsupported-macro-expansion",
                    "macro expansion is outside the MVP observer boundary",
                    format!("{source_id}:{}", macro_span.line),
                    source_id.to_owned(),
                    self.evidence(&variant.path, macro_span),
                    Vec::new(),
                    all_dependency_kinds(),
                ));
            }
            for cfg_span in visitor.cfg_spans {
                issues.push(AnalysisIssue::analysis_context_unresolved(
                    "analysis.unsupported-analysis-context",
                    "cfg and feature conditions are not evaluated by the MVP observer",
                    format!("{source_id}:{}", cfg_span.line),
                    Some(source_id.to_owned()),
                    Some(self.evidence(&variant.path, cfg_span)),
                    all_dependency_kinds(),
                ));
            }
        }
        for observed_path in visitor.paths {
            let path = observed_path.path;
            let strict_single_segment = strict_path_spans.contains(&path.span().start());
            let path_segments = syn_path_segments(&path);
            if path_segments
                .first()
                .is_some_and(|first| observed_path.local_bindings.contains(first))
            {
                continue;
            }
            let resolved = self.resolve_path(
                source_id,
                variant,
                &observed_path.aliases,
                &path_segments,
                path.leading_colon.is_some(),
                all_modules,
            );
            if resolved.is_empty()
                && path_segments.first().is_some_and(|first| {
                    known_language_name(first)
                        || (!variant.no_implicit_prelude && known_prelude_name(first))
                })
            {
                continue;
            }
            if resolved.is_empty() {
                let explicit_relative_path = matches!(
                    path_segments.first().map(String::as_str),
                    Some("crate" | "self" | "super")
                );
                let requires_complete_resolution = analysis_profile == AnalysisProfile::Compilation
                    && (path_segments.len() > 1 || strict_single_segment);
                let may_reference_internal = self.may_reference_internal_module(
                    source_id,
                    &observed_path.aliases,
                    &path_segments,
                    all_modules,
                    0,
                );
                if explicit_relative_path || requires_complete_resolution || may_reference_internal
                {
                    let code = if path_segments
                        .first()
                        .is_some_and(|first| self.context_dependent_crates.contains(first))
                    {
                        "analysis.unsupported-analysis-context"
                    } else if explicit_relative_path {
                        "analysis.unresolved-reference"
                    } else {
                        "analysis.unsupported-name-resolution"
                    };
                    let reason =
                        format!("unable to resolve path {}", path_to_string(&path_segments));
                    let evidence = self.evidence(&variant.path, path.span().start());
                    let candidate_targets = self.candidate_targets_for_path(
                        source_id,
                        variant,
                        &observed_path.aliases,
                        &path_segments,
                        path.leading_colon.is_some(),
                        all_modules,
                    );
                    if code == "analysis.unsupported-analysis-context" {
                        issues.push(AnalysisIssue::analysis_context_unresolved(
                            code,
                            reason,
                            source_id,
                            Some(source_id.to_owned()),
                            Some(evidence),
                            vec![DependencyKind::PathReference],
                        ));
                    } else {
                        issues.push(AnalysisIssue::unresolved_dependency_candidate(
                            code,
                            reason,
                            source_id,
                            source_id.to_owned(),
                            evidence,
                            candidate_targets,
                            vec![DependencyKind::PathReference],
                        ));
                    }
                }
                continue;
            }
            let kind = DependencyKind::PathReference;
            let evidence = self.evidence(&variant.path, path.span().start());
            for (target, scope) in resolved {
                if scope == TargetScope::InternalModule && target == source_id {
                    continue;
                }
                if scope == TargetScope::InternalModule && !selected.contains(&target) {
                    continue;
                }
                dependencies.push(Dependency {
                    candidate_id: candidate_id(source_id, &target, kind, &evidence),
                    source: source_id.to_owned(),
                    target,
                    kind,
                    target_scope: scope,
                    evidence: evidence.clone(),
                });
            }
        }
    }

    fn may_reference_internal_module(
        &self,
        source_module: &str,
        aliases: &BTreeMap<String, Vec<UseAlias>>,
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
                        &expanded,
                        all_modules,
                        alias_depth + 1,
                    )
            });
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

    fn collect_use(
        &self,
        source_id: &str,
        source_variant: &ModuleVariant,
        use_item: &ItemUse,
        aliases: &BTreeMap<String, Vec<UseAlias>>,
        selected: &BTreeSet<String>,
        all_modules: &BTreeSet<String>,
    ) -> (Vec<Dependency>, Vec<AnalysisIssue>) {
        let mut dependencies = Vec::new();
        let mut issues = Vec::new();
        let kind = if matches!(use_item.vis, syn::Visibility::Public(_)) {
            DependencyKind::ReExport
        } else {
            DependencyKind::Import
        };
        let mut paths = Vec::new();
        flatten_use_tree(&use_item.tree, Vec::new(), &mut paths);
        for path in paths {
            let mut resolved = self.resolve_path(
                source_id,
                source_variant,
                aliases,
                &path,
                use_item.leading_colon.is_some(),
                all_modules,
            );
            if resolved.is_empty()
                && let Some(first) = path.first().filter(|first| {
                    known_external_crate(first) || self.external_crates.contains(*first)
                })
            {
                resolved.push((first.clone(), TargetScope::ExternalCrate));
            }
            if resolved.is_empty() {
                let explicit_relative_path = matches!(
                    path.first().map(String::as_str),
                    Some("crate" | "self" | "super")
                );
                let code = if path
                    .first()
                    .is_some_and(|first| self.context_dependent_crates.contains(first))
                {
                    "analysis.unsupported-analysis-context"
                } else if explicit_relative_path {
                    "analysis.unresolved-reference"
                } else {
                    "analysis.unsupported-name-resolution"
                };
                let reason = format!("unable to resolve use path {}", path_to_string(&path));
                let evidence = self.evidence(&source_variant.path, use_item.span().start());
                let candidate_targets = self.candidate_targets_for_path(
                    source_id,
                    source_variant,
                    aliases,
                    &path,
                    use_item.leading_colon.is_some(),
                    all_modules,
                );
                if code == "analysis.unsupported-analysis-context" {
                    issues.push(AnalysisIssue::analysis_context_unresolved(
                        code,
                        reason,
                        source_id,
                        Some(source_id.to_owned()),
                        Some(evidence),
                        vec![kind],
                    ));
                } else {
                    issues.push(AnalysisIssue::unresolved_dependency_candidate(
                        code,
                        reason,
                        source_id,
                        source_id.to_owned(),
                        evidence,
                        candidate_targets,
                        vec![kind],
                    ));
                }
                continue;
            }
            let evidence = self.evidence(&source_variant.path, use_item.span().start());
            for (target, scope) in resolved {
                if scope == TargetScope::InternalModule && target == source_id {
                    continue;
                }
                if scope == TargetScope::InternalModule && !selected.contains(&target) {
                    continue;
                }
                dependencies.push(Dependency {
                    candidate_id: candidate_id(source_id, &target, kind, &evidence),
                    source: source_id.to_owned(),
                    target,
                    kind,
                    target_scope: scope,
                    evidence: evidence.clone(),
                });
            }
        }
        (dependencies, issues)
    }

    fn candidate_targets_for_path(
        &self,
        source_id: &str,
        source_variant: &ModuleVariant,
        aliases: &BTreeMap<String, Vec<UseAlias>>,
        path: &[String],
        leading_colon: bool,
        all_modules: &BTreeSet<String>,
    ) -> Vec<String> {
        let mut candidates = BTreeSet::new();
        for end in (1..path.len()).rev() {
            for (target, scope) in self.resolve_path(
                source_id,
                source_variant,
                aliases,
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
        candidates.into_iter().collect()
    }

    fn resolve_path(
        &self,
        source_module: &str,
        source_variant: &ModuleVariant,
        aliases: &BTreeMap<String, Vec<UseAlias>>,
        path: &[String],
        absolute_external: bool,
        all_modules: &BTreeSet<String>,
    ) -> Vec<(String, TargetScope)> {
        let context = PathResolutionContext {
            source_module,
            source_variant,
            aliases,
            all_modules,
        };
        self.resolve_path_inner(&context, path, absolute_external, 0)
    }

    fn resolve_path_inner(
        &self,
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
            return if known_external_crate(first) || self.external_crates.contains(first) {
                vec![(first.clone(), TargetScope::ExternalCrate)]
            } else {
                Vec::new()
            };
        }
        if alias_depth > self.records.len() {
            return Vec::new();
        }
        if let Some(candidates) = context.aliases.get(&path[0]) {
            return candidates
                .iter()
                .flat_map(|alias| {
                    let mut expanded = alias.path.clone();
                    expanded.extend_from_slice(&path[1..]);
                    self.resolve_path_inner(
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
                } else {
                    let is_external_crate = known_external_crate(first)
                        || self.external_crates.contains(first)
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
                    let declared_item = self.records.get(&candidate).is_some_and(|record| {
                        record
                            .variants
                            .iter()
                            .any(|variant| variant.declared_items.contains(segment))
                    });
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

    fn evidence(&self, path: &Path, start: LineColumn) -> SourceEvidence {
        let relative = path
            .strip_prefix(&self.project_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let column = fs::read_to_string(path)
            .ok()
            .and_then(|source| {
                source
                    .lines()
                    .nth(start.line.saturating_sub(1))
                    .map(str::to_owned)
            })
            .map(|line| {
                line.char_indices()
                    .take_while(|(offset, _)| *offset < start.column)
                    .count()
                    + 1
            })
            .unwrap_or(start.column + 1);
        SourceEvidence {
            path: relative,
            line: start.line,
            column,
        }
    }
}

fn all_dependency_kinds() -> Vec<DependencyKind> {
    vec![
        DependencyKind::Import,
        DependencyKind::ReExport,
        DependencyKind::PathReference,
    ]
}

fn known_external_crate(name: &str) -> bool {
    matches!(name, "alloc" | "core" | "proc_macro" | "std" | "test")
}

fn known_language_name(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "char"
            | "f32"
            | "f64"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "str"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
    )
}

fn known_prelude_name(name: &str) -> bool {
    matches!(
        name,
        "Box"
            | "AsRef"
            | "Clone"
            | "Copy"
            | "Default"
            | "DoubleEndedIterator"
            | "Drop"
            | "Eq"
            | "ExactSizeIterator"
            | "Fn"
            | "FnMut"
            | "FnOnce"
            | "From"
            | "Into"
            | "IntoIterator"
            | "Iterator"
            | "None"
            | "Ok"
            | "Option"
            | "Ord"
            | "PartialEq"
            | "PartialOrd"
            | "Result"
            | "Send"
            | "Sized"
            | "Some"
            | "String"
            | "Sync"
            | "ToOwned"
            | "ToString"
            | "TryFrom"
            | "TryInto"
            | "Unpin"
            | "Vec"
    )
}

fn candidate_id(
    source: &str,
    target: &str,
    kind: DependencyKind,
    evidence: &SourceEvidence,
) -> String {
    format!(
        "dep:{source}:{target}:{}:{}:{}:{}",
        kind.as_str(),
        evidence.path,
        evidence.line,
        evidence.column
    )
}

fn resolve_module_file(directory: &Path, name: &str) -> Option<PathBuf> {
    let flat = directory.join(format!("{name}.rs"));
    if flat.is_file() {
        return Some(flat);
    }
    let nested = directory.join(name).join("mod.rs");
    nested.is_file().then_some(nested)
}

fn module_directory(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or_else(|| Path::new("."));
    let stem = file.file_stem().and_then(|stem| stem.to_str());
    if matches!(stem, Some("mod" | "main" | "lib")) {
        parent.to_path_buf()
    } else {
        parent.join(file.file_stem().unwrap_or_default())
    }
}
