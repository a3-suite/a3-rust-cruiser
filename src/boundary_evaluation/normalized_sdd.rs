use std::collections::{BTreeMap, BTreeSet};

use super::model::{
    AnalysisContext, AnalysisIssue, AnalysisObservationOutcome, BoundaryRule, LogicalModule,
    NormalizedSddCoverage, NormalizedSddPolicy, ObservedDependencyGraph, RuleKind,
    ScopedComponentAttribution, Severity, TargetScope, UnresolvedModuleAttribution,
};
use super::pattern::matches_pattern;

pub fn assess_normalized_sdd(
    issues: &mut Vec<AnalysisIssue>,
    graph: &ObservedDependencyGraph,
    model: &NormalizedSddPolicy,
) -> (Vec<BoundaryRule>, NormalizedSddCoverage) {
    let mut module_attributions = BTreeMap::new();
    let mut covered_modules = Vec::new();
    let mut unresolved_modules = Vec::new();
    for module in &graph.modules {
        match resolve_module_attribution(module, &model.file_attributions) {
            Ok(attribution) => {
                covered_modules.push(module.id.clone());
                module_attributions.insert(module.id.clone(), attribution);
            }
            Err(unresolved) => unresolved_modules.push(unresolved),
        }
    }
    covered_modules.sort();
    unresolved_modules.sort();
    let unresolved_module_ids = unresolved_modules
        .iter()
        .map(|unresolved| unresolved.module_id.as_str())
        .collect::<BTreeSet<_>>();
    for unresolved in &unresolved_modules {
        issues.push(AnalysisIssue::component_attribution_unresolved(
            "analysis.normalized-attribution-unresolved",
            format!(
                "Rust module {} has unresolved normalized attribution for [{}]: {}",
                unresolved.module_id,
                unresolved.observed_file_paths.join(", "),
                unresolved.reason
            ),
            unresolved.module_id.clone(),
        ));
    }

    for dependency in graph
        .dependencies
        .iter()
        .filter(|dependency| dependency.target_scope == TargetScope::InternalModule)
    {
        let missing = if !module_attributions.contains_key(&dependency.source) {
            dependency.source.as_str()
        } else if !module_attributions.contains_key(&dependency.target) {
            dependency.target.as_str()
        } else {
            continue;
        };
        if !unresolved_module_ids.contains(missing) {
            issues.push(AnalysisIssue::component_attribution_unresolved(
                "analysis.normalized-attribution-missing",
                format!("Observed Rust module has no normalized attribution: {missing}"),
                missing.to_owned(),
            ));
        }
    }

    // Unobserved dependencies are judged against the same component model as observed ones,
    // so rules are derived per component pair instead of per observed edge.
    let mut component_modules: BTreeMap<(&str, &str), Vec<String>> = BTreeMap::new();
    for (module_id, attribution) in &module_attributions {
        component_modules
            .entry((
                attribution.canonical_scope.as_str(),
                attribution.component_id.as_str(),
            ))
            .or_default()
            .push(module_id.clone());
    }
    let mut rules = Vec::new();
    for ((source_scope, source_component), source_modules) in &component_modules {
        for ((target_scope, target_component), target_modules) in &component_modules {
            let allowed = if source_scope == target_scope && source_component == target_component {
                true
            } else if source_scope == target_scope {
                model.declared_component_dependencies.contains(&(
                    (*source_scope).to_owned(),
                    (*source_component).to_owned(),
                    (*target_component).to_owned(),
                ))
            } else {
                model
                    .declared_scope_dependencies
                    .contains(&((*source_scope).to_owned(), (*target_scope).to_owned()))
            };
            if !allowed {
                rules.push(BoundaryRule {
                    id: format!("sdd-v2-forbidden-{:04}", rules.len() + 1),
                    kind: RuleKind::Forbidden,
                    severity: Severity::Error,
                    from: source_modules.clone(),
                    to: target_modules.clone(),
                    modules: Vec::new(),
                    dependency_kinds: Vec::new(),
                });
            }
        }
    }

    let observed_module_ids = graph
        .modules
        .iter()
        .map(|module| module.id.as_str())
        .collect::<BTreeSet<_>>();
    let unobserved_sources = issues
        .iter()
        .filter(|issue| {
            matches!(
                issue.observation.outcome,
                AnalysisObservationOutcome::UnresolvedDependencyCandidate
                    | AnalysisObservationOutcome::AnalysisContextUnresolved
            )
        })
        .filter_map(|issue| issue.observation.source_module.clone())
        .filter(|module_id| !observed_module_ids.contains(module_id.as_str()))
        .filter(|module_id| is_selected(&graph.analysis_context, module_id))
        .collect::<BTreeSet<_>>();
    for module_id in unobserved_sources {
        let unresolved = UnresolvedModuleAttribution {
            module_id,
            observed_file_paths: Vec::new(),
            reason: "observer did not read the module source".to_owned(),
        };
        issues.push(AnalysisIssue::component_attribution_unresolved(
            "analysis.normalized-attribution-unresolved",
            format!(
                "Rust module {} has unresolved normalized attribution: {}",
                unresolved.module_id, unresolved.reason
            ),
            unresolved.module_id.clone(),
        ));
        unresolved_modules.push(unresolved);
    }
    unresolved_modules.sort();

    let coverage = NormalizedSddCoverage {
        source_provenance: model.provenance.clone(),
        covered_modules,
        unresolved_modules,
    };
    (rules, coverage)
}

fn is_selected(context: &AnalysisContext, module_id: &str) -> bool {
    (context.include.is_empty()
        || context
            .include
            .iter()
            .any(|pattern| matches_pattern(pattern, module_id)))
        && !context
            .exclude
            .iter()
            .any(|pattern| matches_pattern(pattern, module_id))
}

fn resolve_module_attribution(
    module: &LogicalModule,
    file_attributions: &BTreeMap<String, ScopedComponentAttribution>,
) -> Result<ScopedComponentAttribution, UnresolvedModuleAttribution> {
    let mut source_paths = module.source_paths.clone();
    source_paths.sort();
    source_paths.dedup();
    if source_paths.is_empty() {
        return Err(UnresolvedModuleAttribution {
            module_id: module.id.clone(),
            observed_file_paths: source_paths,
            reason: "observer reported no source file".to_owned(),
        });
    }

    let missing_paths = source_paths
        .iter()
        .filter(|path| !file_attributions.contains_key(*path))
        .cloned()
        .collect::<Vec<_>>();
    if !missing_paths.is_empty() {
        return Err(UnresolvedModuleAttribution {
            module_id: module.id.clone(),
            observed_file_paths: source_paths,
            reason: format!(
                "no normalized file attribution for: {}",
                missing_paths.join(", ")
            ),
        });
    }

    let Some(first) = source_paths
        .first()
        .and_then(|path| file_attributions.get(path))
    else {
        return Err(UnresolvedModuleAttribution {
            module_id: module.id.clone(),
            observed_file_paths: source_paths,
            reason: "normalized file attribution became unavailable".to_owned(),
        });
    };
    if source_paths
        .iter()
        .skip(1)
        .any(|path| file_attributions.get(path) != Some(first))
    {
        return Err(UnresolvedModuleAttribution {
            module_id: module.id.clone(),
            observed_file_paths: source_paths,
            reason: "source files resolve to different owner tuples".to_owned(),
        });
    }

    Ok(first.clone())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::super::evaluation::{decide, evaluate};
    use super::super::model::{
        AnalysisContext, AnalysisIssue, AnalysisProfile, BoundaryPolicy, Dependency,
        DependencyKind, LogicalModule, NormalizedSddPolicy, NormalizedSddProvenance,
        ObservedDependencyGraph, ScopedComponentAttribution, SourceEvidence, TargetScope,
        ValidationDecision,
    };
    use super::assess_normalized_sdd;

    #[test]
    fn should_apply_normalized_same_scope_and_cross_scope_permissions() {
        // Arrange
        let graph = graph_with_dependency("crate::application", "crate::provider");
        let same_scope = normalized_policy("scope-a", "application", "scope-a", "provider");
        let mut declared_same_scope = same_scope.clone();
        declared_same_scope.declared_component_dependencies.insert((
            "scope-a".to_owned(),
            "application".to_owned(),
            "provider".to_owned(),
        ));
        let cross_scope = normalized_policy("scope-a", "application", "scope-b", "provider");
        let mut declared_cross_scope = cross_scope.clone();
        declared_cross_scope
            .declared_scope_dependencies
            .insert(("scope-a".to_owned(), "scope-b".to_owned()));

        // Act
        let (missing_same_scope, _) = assess_normalized_sdd(&mut Vec::new(), &graph, &same_scope);
        let (allowed_same_scope, _) =
            assess_normalized_sdd(&mut Vec::new(), &graph, &declared_same_scope);
        let (missing_cross_scope, _) = assess_normalized_sdd(&mut Vec::new(), &graph, &cross_scope);
        let (allowed_cross_scope, _) =
            assess_normalized_sdd(&mut Vec::new(), &graph, &declared_cross_scope);

        // Assert
        let violation_count = |rules| evaluate(&graph, &BoundaryPolicy { rules }).0.len();
        assert_eq!(violation_count(missing_same_scope), 1);
        assert_eq!(violation_count(allowed_same_scope), 0);
        let cross_scope_violations = violation_count(missing_cross_scope);
        assert_eq!(cross_scope_violations, 1);
        assert_eq!(
            decide(0, cross_scope_violations),
            ValidationDecision::NonConformant
        );
        assert_eq!(violation_count(allowed_cross_scope), 0);
    }

    #[test]
    fn should_allow_normalized_dependencies_inside_one_component() {
        // Arrange
        let graph = graph_with_dependency("crate::application", "crate::provider");
        let policy = normalized_policy("scope-a", "component", "scope-a", "component");

        // Act
        let (rules, _) = assess_normalized_sdd(&mut Vec::new(), &graph, &policy);

        // Assert
        assert!(rules.is_empty());
    }

    #[test]
    fn should_mark_missing_or_unresolved_normalized_attribution_as_indeterminate() {
        // Arrange
        let graph = graph_with_dependency("crate::application", "crate::provider");
        let mut missing = normalized_policy("scope-a", "application", "scope-b", "provider");
        missing.file_attributions.remove("src/provider.rs");
        let mut issues = Vec::new();

        // Act
        let (rules, coverage) = assess_normalized_sdd(&mut issues, &graph, &missing);

        // Assert
        assert!(rules.is_empty());
        assert_eq!(issues.len(), 1);
        assert_eq!(coverage.covered_modules, vec!["crate::application"]);
        assert_eq!(coverage.unresolved_modules.len(), 1);
        assert_eq!(decide(issues.len(), 0), ValidationDecision::Indeterminate);
    }

    #[test]
    fn should_derive_coverage_for_selected_modules_without_dependencies() {
        // Arrange
        let mut graph = graph_with_dependency("crate::application", "crate::provider");
        graph.modules.push(LogicalModule {
            id: "crate::isolated".to_owned(),
            source_paths: vec!["src/isolated.rs".to_owned()],
        });
        let mut policy = normalized_policy("scope-a", "application", "scope-b", "provider");
        policy.file_attributions.insert(
            "src/isolated.rs".to_owned(),
            ScopedComponentAttribution {
                canonical_scope: "scope-a".to_owned(),
                component_id: "isolated".to_owned(),
                element_id: "isolated-element".to_owned(),
            },
        );
        policy.file_attributions.insert(
            "src/unused.rs".to_owned(),
            ScopedComponentAttribution {
                canonical_scope: "scope-a".to_owned(),
                component_id: "unused".to_owned(),
                element_id: "unused-element".to_owned(),
            },
        );

        // Act
        let (_, coverage) = assess_normalized_sdd(&mut Vec::new(), &graph, &policy);

        // Assert
        assert!(
            coverage
                .covered_modules
                .contains(&"crate::isolated".to_owned())
        );
        assert!(coverage.unresolved_modules.is_empty());
        assert_eq!(coverage.covered_modules.len(), 3);
    }

    #[test]
    fn should_mark_conflicting_source_variant_owners_as_unresolved() {
        // Arrange
        let mut graph = graph_with_dependency("crate::application", "crate::provider");
        graph.modules[1]
            .source_paths
            .push("src/provider_alternative.rs".to_owned());
        let mut policy = normalized_policy("scope-a", "application", "scope-b", "provider");
        policy.file_attributions.insert(
            "src/provider_alternative.rs".to_owned(),
            ScopedComponentAttribution {
                canonical_scope: "scope-c".to_owned(),
                component_id: "alternative-provider".to_owned(),
                element_id: "alternative-provider-element".to_owned(),
            },
        );
        let mut issues = Vec::new();

        // Act
        let (rules, coverage) = assess_normalized_sdd(&mut issues, &graph, &policy);

        // Assert
        assert!(rules.is_empty());
        assert_eq!(issues.len(), 1);
        assert_eq!(coverage.unresolved_modules.len(), 1);
        assert_eq!(
            coverage.unresolved_modules[0].observed_file_paths,
            vec!["src/provider.rs", "src/provider_alternative.rs"]
        );
    }

    #[test]
    fn should_mark_only_selected_unobserved_sources_as_unresolved() {
        // Arrange
        let mut graph = graph_with_dependency("crate::application", "crate::provider");
        graph.analysis_context.exclude = vec!["crate::excluded".to_owned()];
        let policy = normalized_policy("scope-a", "application", "scope-a", "provider");
        let unobserved = |module: &str| {
            AnalysisIssue::analysis_context_unresolved(
                "analysis.unsupported-syntax",
                "#[path] module declarations are not supported in the MVP",
                module,
                Some(module.to_owned()),
                None,
                Vec::new(),
            )
        };
        let mut issues = vec![unobserved("crate::selected"), unobserved("crate::excluded")];

        // Act
        let (_, coverage) = assess_normalized_sdd(&mut issues, &graph, &policy);

        // Assert
        let unresolved = coverage
            .unresolved_modules
            .iter()
            .map(|module| module.module_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(unresolved, vec!["crate::selected"]);
    }

    fn graph_with_dependency(source: &str, target: &str) -> ObservedDependencyGraph {
        ObservedDependencyGraph {
            analysis_context: AnalysisContext {
                analysis_profile: AnalysisProfile::Compilation,
                package: "fixture".to_owned(),
                compilation_target: "bin:fixture".to_owned(),
                features: Vec::new(),
                cfg: Vec::new(),
                target_triple: "x86_64-unknown-linux-gnu".to_owned(),
                include: Vec::new(),
                exclude: Vec::new(),
            },
            modules: vec![
                LogicalModule {
                    id: source.to_owned(),
                    source_paths: vec!["src/application.rs".to_owned()],
                },
                LogicalModule {
                    id: target.to_owned(),
                    source_paths: vec!["src/provider.rs".to_owned()],
                },
            ],
            module_declarations: Vec::new(),
            dependencies: vec![Dependency {
                candidate_id: "candidate".to_owned(),
                source: source.to_owned(),
                target: target.to_owned(),
                kind: DependencyKind::Import,
                target_scope: TargetScope::InternalModule,
                evidence: SourceEvidence {
                    path: "src/application.rs".to_owned(),
                    line: 1,
                    column: 1,
                },
            }],
            excluded_scopes: Vec::new(),
        }
    }

    fn normalized_policy(
        source_scope: &str,
        source_component: &str,
        target_scope: &str,
        target_component: &str,
    ) -> NormalizedSddPolicy {
        NormalizedSddPolicy {
            provenance: NormalizedSddProvenance {
                producer: "owner-resolved-current-family".to_owned(),
                artifacts: Vec::new(),
            },
            file_attributions: BTreeMap::from([
                (
                    "src/application.rs".to_owned(),
                    ScopedComponentAttribution {
                        canonical_scope: source_scope.to_owned(),
                        component_id: source_component.to_owned(),
                        element_id: "source-element".to_owned(),
                    },
                ),
                (
                    "src/provider.rs".to_owned(),
                    ScopedComponentAttribution {
                        canonical_scope: target_scope.to_owned(),
                        component_id: target_component.to_owned(),
                        element_id: "target-element".to_owned(),
                    },
                ),
            ]),
            declared_component_dependencies: BTreeSet::new(),
            declared_scope_dependencies: BTreeSet::new(),
        }
    }
}
