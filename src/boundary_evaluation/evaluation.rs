use std::collections::{BTreeMap, BTreeSet};

use super::model::{
    AnalysisIssue, AnalysisObservationOutcome, BoundaryPolicy, BoundaryRule, BoundaryViolation,
    ComponentAttributionModel, Dependency, DependencyKind, ObservedDependencyGraph,
    PolicyEvaluation, PolicyImpact, RuleKind, Severity, TargetScope, ValidationDecision,
    ViolationKind,
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

pub fn decide(possible_issue_count: usize, violation_count: usize) -> ValidationDecision {
    if violation_count > 0 {
        ValidationDecision::NonConformant
    } else if possible_issue_count > 0 {
        ValidationDecision::Indeterminate
    } else {
        ValidationDecision::Conformant
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AttributedDependency {
    source: String,
    target: String,
    kind: DependencyKind,
}

#[derive(Debug, Default)]
struct AttributionPresence {
    possible: bool,
    definite: bool,
}

pub fn assess_component_attribution(
    issues: &mut [AnalysisIssue],
    graph: &ObservedDependencyGraph,
    model: &ComponentAttributionModel,
) -> Vec<BoundaryRule> {
    let candidates = graph
        .modules
        .iter()
        .filter_map(|module| {
            let module_candidates = component_candidates(&module.id, model);
            (!module_candidates.is_empty()).then_some((module.id.clone(), module_candidates))
        })
        .collect::<BTreeMap<_, _>>();
    let (definite_violations, invariant) = analyze_component_attribution(graph, model, &candidates);
    if invariant {
        for issue in issues.iter_mut().filter(|issue| {
            issue.observation.outcome == AnalysisObservationOutcome::ComponentAttributionUnresolved
        }) {
            issue.policy_impact = PolicyImpact::none();
        }
    }

    definite_violations
        .into_iter()
        .enumerate()
        .map(|(index, dependency)| BoundaryRule {
            id: format!("sdd-attribution-forbidden-{:04}", index + 1),
            kind: RuleKind::Forbidden,
            severity: Severity::Error,
            from: vec![dependency.source],
            to: vec![dependency.target],
            modules: Vec::new(),
            dependency_kinds: vec![dependency.kind],
        })
        .collect()
}

fn analyze_component_attribution(
    graph: &ObservedDependencyGraph,
    model: &ComponentAttributionModel,
    candidates: &BTreeMap<String, Vec<String>>,
) -> (BTreeSet<AttributedDependency>, bool) {
    let has_ambiguous_module = candidates.values().any(|values| values.len() > 1);
    let mut presence = BTreeMap::<AttributedDependency, AttributionPresence>::new();
    for dependency in graph
        .dependencies
        .iter()
        .filter(|dependency| dependency.target_scope == TargetScope::InternalModule)
    {
        let Some(source_components) = candidates.get(&dependency.source) else {
            continue;
        };
        let Some(target_components) = candidates.get(&dependency.target) else {
            continue;
        };
        if source_components.len() == 1 && target_components.len() == 1 {
            continue;
        }
        let mut any = false;
        let mut all = true;
        for source_component in source_components {
            for target_component in target_components {
                let violation = source_component != target_component
                    && !model
                        .declared_dependencies
                        .contains(&(source_component.clone(), target_component.clone()));
                any |= violation;
                all &= violation;
            }
        }
        if !any {
            continue;
        }
        let key = AttributedDependency {
            source: dependency.source.clone(),
            target: dependency.target.clone(),
            kind: dependency.kind,
        };
        let entry = presence.entry(key).or_default();
        entry.possible |= any;
        entry.definite |= all;
    }
    let invariant = has_ambiguous_module
        && presence
            .values()
            .all(|value| value.possible == value.definite);
    let definite = presence
        .into_iter()
        .filter_map(|(dependency, value)| value.definite.then_some(dependency))
        .collect();
    (definite, invariant)
}

fn component_candidates(module: &str, model: &ComponentAttributionModel) -> Vec<String> {
    model
        .component_modules
        .iter()
        .filter(|(_, root)| module == root.as_str() || module.starts_with(&format!("{root}::")))
        .map(|(component, _)| component.clone())
        .collect()
}

pub fn assess_policy_impact(issues: &mut [AnalysisIssue], policy: &BoundaryPolicy) {
    for issue in issues {
        issue.policy_impact = match issue.observation.outcome {
            AnalysisObservationOutcome::DependencyNeutral => PolicyImpact::none(),
            AnalysisObservationOutcome::ComponentAttributionUnresolved => {
                issue.policy_impact.clone()
            }
            AnalysisObservationOutcome::UnresolvedDependencyCandidate
            | AnalysisObservationOutcome::AnalysisContextUnresolved => {
                let Some(source) = issue.observation.source_module.as_deref() else {
                    let mut rule_ids = policy
                        .rules
                        .iter()
                        .filter(|rule| issue_kind_can_match(issue, rule))
                        .map(|rule| rule.id.clone())
                        .collect::<Vec<_>>();
                    rule_ids.sort();
                    rule_ids.dedup();
                    issue.policy_impact = if rule_ids.is_empty() {
                        PolicyImpact::none()
                    } else {
                        PolicyImpact::possible(rule_ids)
                    };
                    continue;
                };
                let mut rule_ids = policy_impact_rule_ids(issue, source, policy);
                rule_ids.sort();
                rule_ids.dedup();
                if rule_ids.is_empty() {
                    PolicyImpact::none()
                } else {
                    PolicyImpact::possible(rule_ids)
                }
            }
        };
    }
}

fn policy_impact_rule_ids(
    issue: &AnalysisIssue,
    source: &str,
    policy: &BoundaryPolicy,
) -> Vec<String> {
    let target_can_match = |patterns: &[String]| {
        issue.observation.candidate_targets.is_empty()
            || issue.observation.candidate_targets.iter().any(|target| {
                patterns
                    .iter()
                    .any(|pattern| matches_pattern(pattern, target))
            })
    };
    let mut ids = policy
        .rules
        .iter()
        .filter(|rule| rule.kind == RuleKind::Forbidden)
        .filter(|rule| matches_source(rule, source) && issue_kind_can_match(issue, rule))
        .filter(|rule| target_can_match(&rule.to))
        .map(|rule| rule.id.clone())
        .collect::<Vec<_>>();

    let allowed = policy
        .rules
        .iter()
        .filter(|rule| rule.kind == RuleKind::Allowed)
        .filter(|rule| matches_source(rule, source) && issue_kind_can_match(issue, rule))
        .collect::<Vec<_>>();
    if !allowed.is_empty() {
        let allowed_targets = allowed
            .iter()
            .flat_map(|rule| rule.to.iter())
            .collect::<Vec<_>>();
        let may_be_rejected = issue.observation.candidate_targets.is_empty()
            || issue.observation.candidate_targets.iter().any(|target| {
                !allowed_targets
                    .iter()
                    .any(|pattern| matches_pattern(pattern, target))
            });
        if may_be_rejected {
            ids.extend(allowed.into_iter().map(|rule| rule.id.clone()));
        }
    }

    ids.extend(
        policy
            .rules
            .iter()
            .filter(|rule| rule.kind == RuleKind::Circular)
            .filter(|rule| issue_kind_can_match(issue, rule))
            .filter(|rule| {
                rule.modules
                    .iter()
                    .any(|pattern| matches_pattern(pattern, source))
                    && target_can_match(&rule.modules)
            })
            .map(|rule| rule.id.clone()),
    );
    ids
}

fn issue_kind_can_match(issue: &AnalysisIssue, rule: &BoundaryRule) -> bool {
    issue.observation.dependency_kinds.is_empty()
        || rule.dependency_kinds.is_empty()
        || issue
            .observation
            .dependency_kinds
            .iter()
            .any(|kind| rule.dependency_kinds.contains(kind))
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
        AnalysisContext, AnalysisIssue, AnalysisProfile, BoundaryPolicy, BoundaryRule, Dependency,
        DependencyKind, LogicalModule, ObservedDependencyGraph, PolicyImpactOutcome, RuleKind,
        Severity, SourceEvidence, TargetScope, ValidationDecision, ViolationKind,
    };
    use super::{assess_policy_impact, decide, evaluate};

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
    fn should_prioritize_confirmed_violations_over_analysis_issues() {
        assert_eq!(decide(1, 1), ValidationDecision::NonConformant);
    }

    #[test]
    fn should_keep_dependency_neutral_issues_conformant() {
        // Arrange
        let mut issues = vec![AnalysisIssue::dependency_neutral(
            "analysis.unsupported-syntax",
            "syntax was proven dependency-neutral",
            "crate",
            Some("crate".to_owned()),
            None,
        )];

        // Act
        assess_policy_impact(&mut issues, &BoundaryPolicy { rules: Vec::new() });
        let possible_issue_count = issues
            .iter()
            .filter(|issue| issue.policy_impact.outcome == PolicyImpactOutcome::Possible)
            .count();

        // Assert
        assert_eq!(issues[0].policy_impact.outcome, PolicyImpactOutcome::None);
        assert_eq!(
            decide(possible_issue_count, 0),
            ValidationDecision::Conformant
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
