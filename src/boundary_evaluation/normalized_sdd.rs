use std::collections::{BTreeMap, BTreeSet};

use super::model::{
    AnalysisIssue, BoundaryRule, LogicalModule, NormalizedSddCoverage, NormalizedSddPolicy,
    ObservedDependencyGraph, RuleKind, ScopedComponentAttribution, Severity, TargetScope,
    UnresolvedModuleAttribution,
};

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

    let mut forbidden = BTreeSet::new();
    for dependency in graph
        .dependencies
        .iter()
        .filter(|dependency| dependency.target_scope == TargetScope::InternalModule)
    {
        let source = module_attributions.get(&dependency.source);
        let target = module_attributions.get(&dependency.target);
        let (Some(source), Some(target)) = (source, target) else {
            let missing = if source.is_none() {
                dependency.source.as_str()
            } else {
                dependency.target.as_str()
            };
            if !unresolved_module_ids.contains(missing) {
                issues.push(AnalysisIssue::component_attribution_unresolved(
                    "analysis.normalized-attribution-missing",
                    format!("Observed Rust module has no normalized attribution: {missing}"),
                    missing.to_owned(),
                ));
            }
            continue;
        };

        let allowed = if source.canonical_scope == target.canonical_scope
            && source.component_id == target.component_id
        {
            true
        } else if source.canonical_scope == target.canonical_scope {
            model.declared_component_dependencies.contains(&(
                source.canonical_scope.clone(),
                source.component_id.clone(),
                target.component_id.clone(),
            ))
        } else {
            model.declared_scope_dependencies.contains(&(
                source.canonical_scope.clone(),
                target.canonical_scope.clone(),
            ))
        };
        if !allowed {
            forbidden.insert((
                dependency.source.clone(),
                dependency.target.clone(),
                dependency.kind,
            ));
        }
    }

    let rules = forbidden
        .into_iter()
        .enumerate()
        .map(|(index, (source, target, kind))| BoundaryRule {
            id: format!("sdd-v2-forbidden-{:04}", index + 1),
            kind: RuleKind::Forbidden,
            severity: Severity::Error,
            from: vec![source],
            to: vec![target],
            modules: Vec::new(),
            dependency_kinds: vec![kind],
        })
        .collect();
    let coverage = NormalizedSddCoverage {
        source_provenance: model.provenance.clone(),
        covered_modules,
        unresolved_modules,
    };
    (rules, coverage)
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
        AnalysisContext, AnalysisProfile, BoundaryPolicy, Dependency, DependencyKind,
        LogicalModule, NormalizedSddPolicy, NormalizedSddProvenance, ObservedDependencyGraph,
        ScopedComponentAttribution, SourceEvidence, TargetScope, ValidationDecision,
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
        assert_eq!(missing_same_scope.len(), 1);
        assert!(allowed_same_scope.is_empty());
        assert_eq!(missing_cross_scope.len(), 1);
        let policy = BoundaryPolicy {
            rules: missing_cross_scope,
        };
        let (violations, _) = evaluate(&graph, &policy);
        assert_eq!(violations.len(), 1);
        assert_eq!(
            decide(0, violations.len()),
            ValidationDecision::NonConformant
        );
        assert!(allowed_cross_scope.is_empty());
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
