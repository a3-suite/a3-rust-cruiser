use std::collections::{BTreeMap, BTreeSet};

use super::model::{
    BoundaryPolicy, BoundaryRule, BoundaryViolation, Dependency, DependencyKind,
    ObservedDependencyGraph, PolicyEvaluation, RuleKind, ValidationDecision, ViolationKind,
};
use super::pattern::matches_pattern;

pub fn evaluate(
    graph: &ObservedDependencyGraph,
    policy: &BoundaryPolicy,
) -> (Vec<BoundaryViolation>, PolicyEvaluation) {
    let mut violations = Vec::new();
    let mut dependency_outcomes = BTreeSet::new();

    for dependency in &graph.dependencies {
        let forbidden: Vec<&BoundaryRule> = policy
            .rules
            .iter()
            .filter(|rule| rule.kind == RuleKind::Forbidden && matches_dependency(rule, dependency))
            .collect();
        let allowed: Vec<&BoundaryRule> = policy
            .rules
            .iter()
            .filter(|rule| {
                rule.kind == RuleKind::Allowed && matches_source(rule, dependency.source.as_str())
            })
            .filter(|rule| matches_kind(rule, dependency.kind))
            .collect();

        if !forbidden.is_empty() {
            dependency_outcomes.insert("forbidden".to_owned());
            for rule in forbidden {
                violations.push(violation(rule, dependency, ViolationKind::Forbidden, None));
            }
        } else if !allowed.is_empty() {
            let allowed_targets: BTreeSet<&str> = allowed
                .iter()
                .flat_map(|rule| rule.to.iter().map(String::as_str))
                .collect();
            if !allowed_targets
                .iter()
                .any(|pattern| matches_pattern(pattern, dependency.target.as_str()))
            {
                dependency_outcomes.insert("not-allowed".to_owned());
                if let Some(rule) = allowed.first() {
                    violations.push(violation(rule, dependency, ViolationKind::NotAllowed, None));
                }
            } else {
                dependency_outcomes.insert("allowed".to_owned());
            }
        } else {
            dependency_outcomes.insert("allowed".to_owned());
        }
    }

    let mut cycle_outcome = "acyclic";
    for rule in policy
        .rules
        .iter()
        .filter(|rule| rule.kind == RuleKind::Circular)
    {
        for cycle in find_cycles(graph, rule) {
            cycle_outcome = "circular";
            if let Some(edge) = graph
                .dependencies
                .iter()
                .find(|dependency| dependency.source == cycle[0] && dependency.target == cycle[1])
            {
                violations.push(violation(rule, edge, ViolationKind::Circular, Some(cycle)));
            }
        }
    }

    violations.sort_by(BoundaryViolation::canonical_cmp);

    let dependency_outcome = if dependency_outcomes.contains("forbidden") {
        "forbidden"
    } else if dependency_outcomes.contains("not-allowed") {
        "not-allowed"
    } else {
        "allowed"
    };

    (
        violations,
        PolicyEvaluation {
            dependency_outcome: dependency_outcome.to_owned(),
            cycle_outcome: cycle_outcome.to_owned(),
            forbidden_match_handling: "all-matches".to_owned(),
            allowed_match_handling: "union".to_owned(),
            cycle_handling: "independent".to_owned(),
        },
    )
}

pub fn decide(issue_count: usize, violation_count: usize) -> ValidationDecision {
    if issue_count > 0 {
        ValidationDecision::Indeterminate
    } else if violation_count > 0 {
        ValidationDecision::NonConformant
    } else {
        ValidationDecision::Conformant
    }
}

fn violation(
    rule: &BoundaryRule,
    dependency: &Dependency,
    kind: ViolationKind,
    cycle_path: Option<Vec<String>>,
) -> BoundaryViolation {
    BoundaryViolation {
        kind,
        rule_id: rule.id.clone(),
        severity: rule.severity,
        source: dependency.source.clone(),
        target: dependency.target.clone(),
        dependency_kind: dependency.kind,
        evidence: dependency.evidence.clone(),
        cycle_path,
    }
}

fn matches_dependency(rule: &BoundaryRule, dependency: &Dependency) -> bool {
    matches_source(rule, dependency.source.as_str())
        && rule
            .to
            .iter()
            .any(|pattern| matches_pattern(pattern, dependency.target.as_str()))
        && matches_kind(rule, dependency.kind)
}

fn matches_source(rule: &BoundaryRule, source: &str) -> bool {
    rule.from
        .iter()
        .any(|pattern| matches_pattern(pattern, source))
}

fn matches_kind(rule: &BoundaryRule, kind: DependencyKind) -> bool {
    rule.dependency_kinds.is_empty() || rule.dependency_kinds.contains(&kind)
}

fn find_cycles(graph: &ObservedDependencyGraph, rule: &BoundaryRule) -> Vec<Vec<String>> {
    let nodes: BTreeSet<String> = graph
        .modules
        .iter()
        .filter(|module| {
            rule.modules
                .iter()
                .any(|pattern| matches_pattern(pattern, &module.id))
        })
        .map(|module| module.id.clone())
        .collect();
    let mut edges: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for dependency in graph.dependencies.iter().filter(|dependency| {
        nodes.contains(&dependency.source)
            && nodes.contains(&dependency.target)
            && matches_kind(rule, dependency.kind)
    }) {
        edges
            .entry(dependency.source.clone())
            .or_default()
            .push(dependency.target.clone());
    }
    for targets in edges.values_mut() {
        targets.sort();
        targets.dedup();
    }

    let mut cycles = Vec::new();
    for start in &nodes {
        let mut path = vec![start.clone()];
        collect_cycles(start, start, &edges, &mut path, &mut cycles);
    }
    cycles.sort();
    cycles.dedup();
    cycles
}

fn collect_cycles(
    start: &str,
    current: &str,
    edges: &BTreeMap<String, Vec<String>>,
    path: &mut Vec<String>,
    cycles: &mut Vec<Vec<String>>,
) {
    for target in edges.get(current).into_iter().flatten() {
        if target == start && path.len() > 1 {
            let mut cycle = path.clone();
            cycle.push(start.to_owned());
            cycles.push(canonical_cycle(&cycle));
        } else if !path.contains(target) && path.len() < edges.len() + 1 {
            path.push(target.clone());
            collect_cycles(start, target, edges, path, cycles);
            path.pop();
        }
    }
}

fn canonical_cycle(cycle: &[String]) -> Vec<String> {
    let body = &cycle[..cycle.len() - 1];
    let mut best: Option<Vec<String>> = None;
    for offset in 0..body.len() {
        let mut rotated = body[offset..].to_vec();
        rotated.extend_from_slice(&body[..offset]);
        rotated.push(rotated[0].clone());
        if best
            .as_ref()
            .is_none_or(|candidate| rotated.as_slice() < candidate.as_slice())
        {
            best = Some(rotated);
        }
    }
    best.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::model::{
        AnalysisContext, AnalysisProfile, BoundaryPolicy, BoundaryRule, Dependency, DependencyKind,
        LogicalModule, ObservedDependencyGraph, RuleKind, Severity, SourceEvidence, TargetScope,
        ValidationDecision, ViolationKind,
    };
    use super::{decide, evaluate};

    #[test]
    fn should_prioritize_forbidden_rule_over_allowed_rule() {
        // Arrange
        let graph = graph_with_dependency("crate::application", "crate::infrastructure");
        let policy = BoundaryPolicy {
            rules: vec![
                rule("forbidden-application", RuleKind::Forbidden),
                rule("allowed-application", RuleKind::Allowed),
            ],
        };

        // Act
        let (violations, _) = evaluate(&graph, &policy);

        // Assert
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].kind, ViolationKind::Forbidden);
        assert_eq!(
            decide(0, violations.len()),
            ValidationDecision::NonConformant
        );
    }

    #[test]
    fn should_reject_an_unlisted_target_from_the_allowed_target_union() {
        // Arrange
        let graph = graph_with_dependency("crate::application", "crate::infrastructure");
        let policy = BoundaryPolicy {
            rules: vec![BoundaryRule {
                id: "allowed-application-domain".to_owned(),
                kind: RuleKind::Allowed,
                severity: Severity::Warning,
                from: vec!["crate::application".to_owned()],
                to: vec!["crate::domain".to_owned()],
                modules: Vec::new(),
                dependency_kinds: Vec::new(),
            }],
        };

        // Act
        let (violations, _) = evaluate(&graph, &policy);

        // Assert
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].kind, ViolationKind::NotAllowed);
    }

    #[test]
    fn should_report_a_canonical_cycle_path_for_a_circular_rule() {
        // Arrange
        let mut graph = graph_with_dependency("crate::application", "crate::infrastructure");
        graph.dependencies.push(Dependency {
            candidate_id: "reverse".to_owned(),
            source: "crate::infrastructure".to_owned(),
            target: "crate::application".to_owned(),
            kind: DependencyKind::PathReference,
            target_scope: TargetScope::InternalModule,
            evidence: SourceEvidence {
                path: "src/infrastructure.rs".to_owned(),
                line: 1,
                column: 1,
            },
        });
        let policy = BoundaryPolicy {
            rules: vec![BoundaryRule {
                id: "circular-core-modules".to_owned(),
                kind: RuleKind::Circular,
                severity: Severity::Error,
                from: Vec::new(),
                to: Vec::new(),
                modules: vec!["crate::**".to_owned()],
                dependency_kinds: Vec::new(),
            }],
        };

        // Act
        let (violations, evaluation) = evaluate(&graph, &policy);

        // Assert
        assert_eq!(evaluation.cycle_outcome, "circular");
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].kind, ViolationKind::Circular);
        assert_eq!(
            violations[0]
                .cycle_path
                .as_ref()
                .map(|path| path.join(" -> ")),
            Some("crate::application -> crate::infrastructure -> crate::application".to_owned())
        );
    }

    #[test]
    fn should_keep_dependency_outcome_within_the_contract_value_set() {
        // Arrange
        let mut graph = graph_with_dependency("crate::application", "crate::infrastructure");
        graph.modules.push(LogicalModule {
            id: "crate::domain".to_owned(),
        });
        graph.dependencies.push(Dependency {
            candidate_id: "allowed".to_owned(),
            source: "crate::application".to_owned(),
            target: "crate::domain".to_owned(),
            kind: DependencyKind::Import,
            target_scope: TargetScope::InternalModule,
            evidence: SourceEvidence {
                path: "src/application.rs".to_owned(),
                line: 2,
                column: 1,
            },
        });
        let policy = BoundaryPolicy {
            rules: vec![BoundaryRule {
                id: "allowed-application-domain".to_owned(),
                kind: RuleKind::Allowed,
                severity: Severity::Warning,
                from: vec!["crate::application".to_owned()],
                to: vec!["crate::domain".to_owned()],
                modules: Vec::new(),
                dependency_kinds: Vec::new(),
            }],
        };

        // Act
        let (_, evaluation) = evaluate(&graph, &policy);

        // Assert
        assert_eq!(evaluation.dependency_outcome, "not-allowed");
    }

    #[test]
    fn should_filter_circular_rule_edges_by_dependency_kind() {
        // Arrange
        let mut graph = graph_with_dependency("crate::application", "crate::infrastructure");
        graph.dependencies.push(Dependency {
            candidate_id: "reverse".to_owned(),
            source: "crate::infrastructure".to_owned(),
            target: "crate::application".to_owned(),
            kind: DependencyKind::PathReference,
            target_scope: TargetScope::InternalModule,
            evidence: SourceEvidence {
                path: "src/infrastructure.rs".to_owned(),
                line: 1,
                column: 1,
            },
        });
        let policy = BoundaryPolicy {
            rules: vec![BoundaryRule {
                id: "circular-paths-only".to_owned(),
                kind: RuleKind::Circular,
                severity: Severity::Error,
                from: Vec::new(),
                to: Vec::new(),
                modules: vec!["crate::**".to_owned()],
                dependency_kinds: vec![DependencyKind::PathReference],
            }],
        };

        // Act
        let (violations, evaluation) = evaluate(&graph, &policy);

        // Assert
        assert_eq!(evaluation.cycle_outcome, "acyclic");
        assert!(violations.is_empty());
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
                },
                LogicalModule {
                    id: target.to_owned(),
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

    fn rule(id: &str, kind: RuleKind) -> BoundaryRule {
        BoundaryRule {
            id: id.to_owned(),
            kind,
            severity: Severity::Error,
            from: vec!["crate::application".to_owned()],
            to: vec!["crate::infrastructure".to_owned()],
            modules: Vec::new(),
            dependency_kinds: Vec::new(),
        }
    }
}
