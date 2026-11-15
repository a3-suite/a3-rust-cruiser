use std::{collections::BTreeSet, path::Path};

use proc_macro2::LineColumn;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Item, ItemUse};

use super::super::{
    cfg_context::CfgDecision,
    rust_syntax::{
        ObservedPath, PathVisitor, ScopedUse, declared_macro_bindings, flatten_use_tree,
        is_crate_visible, known_external_crate, known_language_name, known_prelude_name,
        local_macro_safety_bindings, path_to_string, syn_path_segments,
    },
};
use super::{
    ModuleVariant, Observer,
    path_resolution::{PathResolver, ResolutionScope},
};
use crate::boundary_validation::{
    AnalysisIssue, AnalysisProfile, Dependency, DependencyKind, SourceEvidence, TargetScope,
};

impl Observer {
    pub(super) fn collect_dependencies(
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
        self.collect_top_level_use_dependencies(
            source_id,
            variant,
            selected,
            all_modules,
            dependencies,
            issues,
        );
        let visitor = self.observe_variant(variant, analysis_profile);
        let PathVisitor {
            paths,
            scoped_uses,
            strict_path_spans,
            macro_spans,
            cfg_spans,
            ..
        } = visitor;
        self.collect_scoped_use_dependencies(
            source_id,
            variant,
            &scoped_uses,
            selected,
            all_modules,
            dependencies,
            issues,
        );
        if analysis_profile == AnalysisProfile::Compilation {
            self.append_visitor_issues(source_id, &variant.path, macro_spans, cfg_spans, issues);
        }
        self.collect_observed_path_dependencies(
            source_id,
            variant,
            paths,
            strict_path_spans,
            selected,
            all_modules,
            analysis_profile,
            dependencies,
            issues,
        );
    }

    fn collect_top_level_use_dependencies(
        &self,
        source_id: &str,
        variant: &ModuleVariant,
        selected: &BTreeSet<String>,
        all_modules: &BTreeSet<String>,
        dependencies: &mut Vec<Dependency>,
        issues: &mut Vec<AnalysisIssue>,
    ) {
        let top_level_scope = ResolutionScope {
            aliases: &variant.use_aliases,
            glob_imports: &variant.glob_imports,
        };
        for item in &variant.ast.items {
            if let Item::Use(use_item) = item
                && self.cfg_context.evaluate_attributes(&use_item.attrs) != CfgDecision::Inactive
            {
                let (use_dependencies, use_issues) = self.collect_use(
                    source_id,
                    variant,
                    use_item,
                    &top_level_scope,
                    selected,
                    all_modules,
                );
                dependencies.extend(use_dependencies);
                issues.extend(use_issues);
            }
        }
    }

    fn observe_variant<'a>(
        &'a self,
        variant: &'a ModuleVariant,
        analysis_profile: AnalysisProfile,
    ) -> PathVisitor<'a> {
        let macro_bindings = declared_macro_bindings(&variant.ast, &self.cfg_context);
        let (safe_local_macro_bindings, observing_local_macro_bindings) =
            local_macro_safety_bindings(
                &variant.ast,
                &self.cfg_context,
                &variant.declared_items,
                variant.no_implicit_prelude,
            );
        let mut visitor = PathVisitor {
            cfg_context: &self.cfg_context,
            external_crates: &self.external_crates,
            declared_items: &variant.declared_items,
            crate_macro_rules_names: &self.crate_macro_rules_names,
            observe_attribute_paths: analysis_profile == AnalysisProfile::Compilation,
            paths: Vec::new(),
            scoped_uses: Vec::new(),
            local_bindings: BTreeSet::new(),
            local_item_bindings: BTreeSet::new(),
            aliases: variant.use_aliases.clone(),
            glob_imports: variant.glob_imports.clone(),
            macro_bindings,
            local_macro_bindings: variant.local_macro_bindings.clone(),
            safe_local_macro_bindings,
            observing_local_macro_bindings,
            inactive_expr_cfg: false,
            in_expr: false,
            in_item_macro: false,
            cfg_guarded_macro: false,
            no_implicit_prelude: variant.no_implicit_prelude,
            macro_spans: Vec::new(),
            cfg_spans: Vec::new(),
            strict_path_spans: Vec::new(),
        };
        visitor.visit_file(&variant.ast);
        visitor
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_scoped_use_dependencies(
        &self,
        source_id: &str,
        variant: &ModuleVariant,
        scoped_uses: &[ScopedUse],
        selected: &BTreeSet<String>,
        all_modules: &BTreeSet<String>,
        dependencies: &mut Vec<Dependency>,
        issues: &mut Vec<AnalysisIssue>,
    ) {
        for scoped_use in scoped_uses {
            let scoped_scope = ResolutionScope {
                aliases: &scoped_use.aliases,
                glob_imports: &scoped_use.glob_imports,
            };
            let (use_dependencies, use_issues) = self.collect_use(
                source_id,
                variant,
                &scoped_use.item,
                &scoped_scope,
                selected,
                all_modules,
            );
            dependencies.extend(use_dependencies);
            issues.extend(use_issues);
        }
    }

    fn append_visitor_issues(
        &self,
        source_id: &str,
        path: &Path,
        macro_spans: Vec<LineColumn>,
        cfg_spans: Vec<LineColumn>,
        issues: &mut Vec<AnalysisIssue>,
    ) {
        for macro_span in macro_spans {
            issues.push(AnalysisIssue::unresolved_dependency_candidate(
                "analysis.unsupported-macro-expansion",
                "macro expansion is outside the MVP observer boundary",
                format!("{source_id}:{}", macro_span.line),
                source_id.to_owned(),
                self.evidence(path, macro_span),
                Vec::new(),
                DependencyKind::ALL.to_vec(),
            ));
        }
        for cfg_span in cfg_spans {
            issues.push(AnalysisIssue::analysis_context_unresolved(
                "analysis.unsupported-analysis-context",
                "cfg and feature conditions are not evaluated by the MVP observer",
                format!("{source_id}:{}", cfg_span.line),
                Some(source_id.to_owned()),
                Some(self.evidence(path, cfg_span)),
                DependencyKind::ALL.to_vec(),
            ));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_observed_path_dependencies(
        &self,
        source_id: &str,
        variant: &ModuleVariant,
        paths: Vec<ObservedPath>,
        strict_path_spans: Vec<LineColumn>,
        selected: &BTreeSet<String>,
        all_modules: &BTreeSet<String>,
        analysis_profile: AnalysisProfile,
        dependencies: &mut Vec<Dependency>,
        issues: &mut Vec<AnalysisIssue>,
    ) {
        for observed_path in paths {
            let evidence_span = observed_path.evidence_span;
            let requires_complete_resolution = observed_path.requires_complete_resolution;
            let path = observed_path.path;
            let strict_single_segment = strict_path_spans.contains(&path.span().start());
            let path_segments = syn_path_segments(&path);
            if path_segments
                .first()
                .is_some_and(|first| observed_path.local_bindings.contains(first))
            {
                if requires_complete_resolution && analysis_profile == AnalysisProfile::Compilation
                {
                    issues.push(AnalysisIssue::unresolved_dependency_candidate(
                        "analysis.unsupported-name-resolution",
                        format!(
                            "unable to resolve attribute path {} because its root is shadowed",
                            path_to_string(&path_segments)
                        ),
                        source_id,
                        source_id.to_owned(),
                        self.evidence(&variant.path, evidence_span),
                        Vec::new(),
                        vec![DependencyKind::PathReference],
                    ));
                }
                continue;
            }
            let observed_scope = ResolutionScope {
                aliases: &observed_path.aliases,
                glob_imports: &observed_path.glob_imports,
            };
            let resolver = PathResolver::new(
                &self.records,
                &self.external_crates,
                &self.self_crate_aliases,
                &self.exported_items,
            );
            let resolved = resolver.resolve_path(
                source_id,
                variant,
                &observed_scope,
                &path_segments,
                path.leading_colon.is_some(),
                all_modules,
            );
            let may_reference_internal = resolver.may_reference_internal_module(
                source_id,
                &observed_path.aliases,
                &observed_path.glob_imports,
                &path_segments,
                all_modules,
                0,
            );
            if resolved.is_empty()
                && path_segments.first().is_some_and(|first| {
                    known_language_name(first)
                        || (!variant.no_implicit_prelude && known_prelude_name(first))
                })
                && (!requires_complete_resolution || !may_reference_internal)
            {
                continue;
            }
            if resolved.is_empty() {
                let explicit_relative_path = matches!(
                    path_segments.first().map(String::as_str),
                    Some("crate" | "self" | "super")
                );
                let requires_complete_resolution = analysis_profile == AnalysisProfile::Compilation
                    && (requires_complete_resolution
                        || path_segments.len() > 1
                        || strict_single_segment);
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
                    let evidence = self.evidence(&variant.path, evidence_span);
                    let candidate_targets = resolver.candidate_targets_for_path(
                        source_id,
                        variant,
                        &observed_scope,
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
            let declared_item_shadow = requires_complete_resolution
                && path_segments.len() > 1
                && path.leading_colon.is_none()
                && path_segments.first().is_some_and(|first| {
                    !matches!(first.as_str(), "crate" | "self" | "super")
                        && !observed_path.aliases.contains_key(first)
                        && variant.declared_items.contains(first)
                })
                && resolved.iter().all(|(target, scope)| {
                    *scope == TargetScope::InternalModule && target == source_id
                });
            let mut distinct_resolutions = resolved.clone();
            distinct_resolutions.sort_by(|left, right| {
                left.0.cmp(&right.0).then_with(|| {
                    let rank = |scope| match scope {
                        TargetScope::InternalModule => 0,
                        TargetScope::ExternalCrate => 1,
                    };
                    rank(left.1).cmp(&rank(right.1))
                })
            });
            distinct_resolutions.dedup();
            if requires_complete_resolution
                && analysis_profile == AnalysisProfile::Compilation
                && (declared_item_shadow || distinct_resolutions.len() > 1)
            {
                let candidate_targets = distinct_resolutions
                    .iter()
                    .filter_map(|(target, scope)| {
                        (*scope == TargetScope::InternalModule && target != source_id)
                            .then_some(target.clone())
                    })
                    .collect();
                issues.push(AnalysisIssue::unresolved_dependency_candidate(
                    "analysis.unsupported-name-resolution",
                    format!(
                        "unable to prove a unique resolution for attribute path {}",
                        path_to_string(&path_segments)
                    ),
                    source_id,
                    source_id.to_owned(),
                    self.evidence(&variant.path, evidence_span),
                    candidate_targets,
                    vec![DependencyKind::PathReference],
                ));
                continue;
            }
            let kind = DependencyKind::PathReference;
            let evidence = self.evidence(&variant.path, evidence_span);
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

    fn collect_use(
        &self,
        source_id: &str,
        source_variant: &ModuleVariant,
        use_item: &ItemUse,
        resolution_scope: &ResolutionScope<'_>,
        selected: &BTreeSet<String>,
        all_modules: &BTreeSet<String>,
    ) -> (Vec<Dependency>, Vec<AnalysisIssue>) {
        let mut dependencies = Vec::new();
        let mut issues = Vec::new();
        let kind = if is_crate_visible(&use_item.vis) {
            DependencyKind::ReExport
        } else {
            DependencyKind::Import
        };
        let mut paths = Vec::new();
        flatten_use_tree(&use_item.tree, Vec::new(), &mut paths);
        for path in paths {
            if path.len() == 1 && source_variant.local_macro_bindings.contains(&path[0]) {
                continue;
            }
            let resolver = PathResolver::new(
                &self.records,
                &self.external_crates,
                &self.self_crate_aliases,
                &self.exported_items,
            );
            let mut resolved = resolver.resolve_path(
                source_id,
                source_variant,
                resolution_scope,
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
                let candidate_targets = resolver.candidate_targets_for_path(
                    source_id,
                    source_variant,
                    resolution_scope,
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
