use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use crate::boundary_evaluation::{
    AnalysisContext, AnalysisIssue, AnalysisObservationOutcome, BoundaryPolicy,
    BoundaryPolicyAssessment, LoadedBoundaryPolicy, ObservedDependencyGraph, PolicyImpactOutcome,
    RuleKind, ValidationResult, assess_normalized_sdd, assess_policy_impact, decide, evaluate,
    is_valid_pattern,
};

use super::{CruiserError, Result};

pub trait BoundaryPolicyLoader {
    fn prepare(&self, input: &BoundaryPolicyInput) -> Result<PreparedBoundaryPolicy>;

    fn finalize(
        &self,
        prepared: PreparedBoundaryPolicy,
        project_root: &Path,
    ) -> Result<LoadedBoundaryPolicy>;
}

pub(crate) enum PreparedBoundaryPolicy {
    Yaml(BoundaryPolicy),
    ResolvedImplementationAttribution(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundaryPolicyInput {
    Yaml { path: PathBuf },
    ResolvedImplementationAttribution { path: PathBuf },
}

pub trait DependencyObserver {
    fn observe(
        &self,
        project_root: &Path,
        context: AnalysisContext,
    ) -> Result<(ObservedDependencyGraph, Vec<AnalysisIssue>)>;
}

pub fn validate<O, L>(
    observer: &O,
    policy_loader: &L,
    project_root: &Path,
    context: AnalysisContext,
    policy_input: &BoundaryPolicyInput,
) -> Result<ValidationResult>
where
    O: DependencyObserver,
    L: BoundaryPolicyLoader,
{
    if context.package.is_empty()
        || context.compilation_target.is_empty()
        || context.target_triple.is_empty()
    {
        return Err(CruiserError::InvalidInput(
            "package, target, and target_triple are required".to_owned(),
        ));
    }
    let prepared = policy_loader.prepare(policy_input)?;
    let loaded = policy_loader.finalize(prepared, project_root)?;
    let LoadedBoundaryPolicy {
        mut policy,
        mut analysis_issues,
        assessment,
    } = loaded;
    validate_policy(&policy)?;
    validate_module_patterns(&context.include)?;
    validate_module_patterns(&context.exclude)?;
    let (graph, observation_issues) = observer.observe(project_root, context)?;
    analysis_issues.extend(observation_issues);
    validate_analysis_issues(&analysis_issues)?;
    let mut normalized_sdd_coverage = None;
    match &assessment {
        BoundaryPolicyAssessment::None => {}
        BoundaryPolicyAssessment::ResolvedImplementationAttribution(model) => {
            let (rules, coverage) = assess_normalized_sdd(&mut analysis_issues, &graph, model);
            policy.rules.extend(rules);
            normalized_sdd_coverage = Some(coverage);
        }
    }
    assess_policy_impact(&mut analysis_issues, &policy);
    validate_analysis_issues(&analysis_issues)?;
    let (violations, policy_evaluation) = evaluate(&graph, &policy);
    let possible_issue_count = analysis_issues
        .iter()
        .filter(|issue| issue.policy_impact.outcome == PolicyImpactOutcome::Possible)
        .count();
    let decision = decide(possible_issue_count, violations.len());
    Ok(ValidationResult {
        graph,
        violations,
        policy_evaluation,
        analysis_issues,
        decision,
        normalized_sdd_coverage,
    })
}

fn validate_analysis_issues(issues: &[AnalysisIssue]) -> Result<()> {
    for issue in issues {
        if issue.observation.outcome == AnalysisObservationOutcome::DependencyNeutral
            && issue.policy_impact.outcome != PolicyImpactOutcome::None
        {
            return Err(CruiserError::InvalidInput(format!(
                "dependency-neutral analysis issue {} cannot have possible policy impact",
                issue.code
            )));
        }
        if issue.policy_impact.outcome == PolicyImpactOutcome::None
            && !issue.policy_impact.rule_ids.is_empty()
        {
            return Err(CruiserError::InvalidInput(format!(
                "analysis issue {} with no policy impact cannot reference rules",
                issue.code
            )));
        }
    }
    Ok(())
}

fn validate_policy(policy: &BoundaryPolicy) -> Result<()> {
    let mut ids = BTreeSet::new();
    for rule in &policy.rules {
        if !is_kebab_case(&rule.id) || !ids.insert(rule.id.as_str()) {
            return Err(CruiserError::InvalidInput(format!(
                "invalid or duplicate boundary policy rule id: {}",
                rule.id
            )));
        }
        if rule.from.is_empty() && rule.kind != RuleKind::Circular {
            return Err(CruiserError::InvalidInput(format!(
                "rule {} requires at least one from pattern",
                rule.id
            )));
        }
        if rule.to.is_empty() && rule.kind != RuleKind::Circular {
            return Err(CruiserError::InvalidInput(format!(
                "rule {} requires at least one to pattern",
                rule.id
            )));
        }
        if rule.modules.is_empty() && rule.kind == RuleKind::Circular {
            return Err(CruiserError::InvalidInput(format!(
                "circular rule {} requires at least one modules pattern",
                rule.id
            )));
        }
        if rule.kind == RuleKind::Circular && (!rule.from.is_empty() || !rule.to.is_empty()) {
            return Err(CruiserError::InvalidInput(format!(
                "circular rule {} cannot define from or to patterns",
                rule.id
            )));
        }
        if rule.kind != RuleKind::Circular && !rule.modules.is_empty() {
            return Err(CruiserError::InvalidInput(format!(
                "non-circular rule {} cannot define modules patterns",
                rule.id
            )));
        }
        for pattern in rule
            .from
            .iter()
            .chain(rule.to.iter())
            .chain(rule.modules.iter())
        {
            if !is_valid_pattern(pattern) {
                return Err(CruiserError::InvalidInput(format!(
                    "invalid logical module pattern in rule {}: {pattern}",
                    rule.id
                )));
            }
        }
    }
    Ok(())
}

fn validate_module_patterns(patterns: &[String]) -> Result<()> {
    for pattern in patterns {
        if !is_valid_pattern(pattern) {
            return Err(CruiserError::InvalidInput(format!(
                "invalid logical module pattern: {pattern}"
            )));
        }
    }
    Ok(())
}

fn is_kebab_case(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
}

#[cfg(test)]
mod tests {
    use super::{BoundaryPolicyLoader, DependencyObserver, PreparedBoundaryPolicy, validate};
    use crate::boundary_validation::{
        AnalysisContext, AnalysisIssue, BoundaryPolicy, BoundaryPolicyAssessment, BoundaryRule,
        CruiserError, LoadedBoundaryPolicy, ObservedDependencyGraph, Result, RuleKind, Severity,
        ValidationDecision,
    };

    struct StubPolicyLoader;

    impl BoundaryPolicyLoader for StubPolicyLoader {
        fn prepare(&self, _input: &super::BoundaryPolicyInput) -> Result<PreparedBoundaryPolicy> {
            Ok(PreparedBoundaryPolicy::Yaml(BoundaryPolicy {
                rules: Vec::new(),
            }))
        }

        fn finalize(
            &self,
            prepared: PreparedBoundaryPolicy,
            _project_root: &std::path::Path,
        ) -> Result<LoadedBoundaryPolicy> {
            let PreparedBoundaryPolicy::Yaml(policy) = prepared else {
                panic!("stub policy loader only prepares YAML policies")
            };
            Ok(LoadedBoundaryPolicy {
                policy,
                analysis_issues: Vec::new(),
                assessment: BoundaryPolicyAssessment::None,
            })
        }
    }

    struct StubDependencyObserver;

    impl DependencyObserver for StubDependencyObserver {
        fn observe(
            &self,
            _project_root: &std::path::Path,
            context: AnalysisContext,
        ) -> Result<(ObservedDependencyGraph, Vec<AnalysisIssue>)> {
            Ok((
                ObservedDependencyGraph {
                    analysis_context: context,
                    modules: Vec::new(),
                    module_declarations: Vec::new(),
                    dependencies: Vec::new(),
                    excluded_scopes: Vec::new(),
                },
                Vec::new(),
            ))
        }
    }

    struct AnalysisIssueObserver {
        issue: AnalysisIssue,
    }

    impl DependencyObserver for AnalysisIssueObserver {
        fn observe(
            &self,
            _project_root: &std::path::Path,
            context: AnalysisContext,
        ) -> Result<(ObservedDependencyGraph, Vec<AnalysisIssue>)> {
            Ok((
                ObservedDependencyGraph {
                    analysis_context: context,
                    modules: Vec::new(),
                    module_declarations: Vec::new(),
                    dependencies: Vec::new(),
                    excluded_scopes: Vec::new(),
                },
                vec![self.issue.clone()],
            ))
        }
    }

    struct InvalidPolicyLoader;

    impl BoundaryPolicyLoader for InvalidPolicyLoader {
        fn prepare(&self, _input: &super::BoundaryPolicyInput) -> Result<PreparedBoundaryPolicy> {
            let rule = BoundaryRule {
                id: "duplicate".to_owned(),
                kind: RuleKind::Forbidden,
                severity: Severity::Error,
                from: vec!["crate::application".to_owned()],
                to: vec!["crate::infrastructure".to_owned()],
                modules: Vec::new(),
                dependency_kinds: Vec::new(),
            };
            Ok(PreparedBoundaryPolicy::Yaml(BoundaryPolicy {
                rules: vec![rule.clone(), rule],
            }))
        }

        fn finalize(
            &self,
            prepared: PreparedBoundaryPolicy,
            _project_root: &std::path::Path,
        ) -> Result<LoadedBoundaryPolicy> {
            let PreparedBoundaryPolicy::Yaml(policy) = prepared else {
                panic!("invalid policy loader only prepares YAML policies")
            };
            Ok(LoadedBoundaryPolicy {
                policy,
                analysis_issues: Vec::new(),
                assessment: BoundaryPolicyAssessment::None,
            })
        }
    }

    #[test]
    fn should_coordinate_validation_ports() {
        // Arrange
        let project_root = std::path::Path::new("project-that-does-not-exist");
        let policy = super::BoundaryPolicyInput::Yaml {
            path: "policy-that-does-not-exist.yaml".into(),
        };
        let context = AnalysisContext {
            analysis_profile: crate::boundary_validation::AnalysisProfile::Compilation,
            package: "fixture".to_owned(),
            compilation_target: "bin:fixture".to_owned(),
            features: Vec::new(),
            cfg: Vec::new(),
            target_triple: "x86_64-unknown-linux-gnu".to_owned(),
            include: Vec::new(),
            exclude: Vec::new(),
        };

        // Act
        let result = validate(
            &StubDependencyObserver,
            &StubPolicyLoader,
            project_root,
            context,
            &policy,
        )
        .expect("port-backed validation should succeed");

        // Assert
        assert_eq!(result.decision, ValidationDecision::Conformant);
    }

    #[test]
    fn should_reject_invalid_policy_from_a_port() {
        // Arrange
        let context = AnalysisContext {
            analysis_profile: crate::boundary_validation::AnalysisProfile::Compilation,
            package: "fixture".to_owned(),
            compilation_target: "bin:fixture".to_owned(),
            features: Vec::new(),
            cfg: Vec::new(),
            target_triple: "x86_64-unknown-linux-gnu".to_owned(),
            include: Vec::new(),
            exclude: Vec::new(),
        };

        // Act
        let result = validate(
            &StubDependencyObserver,
            &InvalidPolicyLoader,
            std::path::Path::new("unused-project"),
            context,
            &super::BoundaryPolicyInput::Yaml {
                path: "unused-policy".into(),
            },
        );

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn should_reject_possible_policy_impact_for_a_dependency_neutral_issue() {
        // Arrange
        let mut issue = AnalysisIssue::dependency_neutral(
            "analysis.unsupported-syntax",
            "syntax was proven dependency-neutral",
            "crate",
            Some("crate".to_owned()),
            None,
        );
        issue.policy_impact = crate::boundary_evaluation::PolicyImpact::possible(Vec::new());
        let context = AnalysisContext {
            analysis_profile: crate::boundary_validation::AnalysisProfile::Compilation,
            package: "fixture".to_owned(),
            compilation_target: "bin:fixture".to_owned(),
            features: Vec::new(),
            cfg: Vec::new(),
            target_triple: "x86_64-unknown-linux-gnu".to_owned(),
            include: Vec::new(),
            exclude: Vec::new(),
        };

        // Act
        let result = validate(
            &AnalysisIssueObserver { issue },
            &StubPolicyLoader,
            std::path::Path::new("unused-project"),
            context,
            &super::BoundaryPolicyInput::Yaml {
                path: "unused-policy".into(),
            },
        );

        // Assert
        assert!(matches!(
            result,
            Err(CruiserError::InvalidInput(message))
                if message == "dependency-neutral analysis issue analysis.unsupported-syntax cannot have possible policy impact"
        ));
    }

    #[test]
    fn should_reject_rule_ids_for_an_issue_without_policy_impact() {
        // Arrange
        let mut issue = AnalysisIssue::dependency_neutral(
            "analysis.unsupported-syntax",
            "syntax was proven dependency-neutral",
            "crate",
            Some("crate".to_owned()),
            None,
        );
        issue
            .policy_impact
            .rule_ids
            .push("unexpected-rule".to_owned());
        let context = AnalysisContext {
            analysis_profile: crate::boundary_validation::AnalysisProfile::Compilation,
            package: "fixture".to_owned(),
            compilation_target: "bin:fixture".to_owned(),
            features: Vec::new(),
            cfg: Vec::new(),
            target_triple: "x86_64-unknown-linux-gnu".to_owned(),
            include: Vec::new(),
            exclude: Vec::new(),
        };

        // Act
        let result = validate(
            &AnalysisIssueObserver { issue },
            &StubPolicyLoader,
            std::path::Path::new("unused-project"),
            context,
            &super::BoundaryPolicyInput::Yaml {
                path: "unused-policy".into(),
            },
        );

        // Assert
        assert!(matches!(
            result,
            Err(CruiserError::InvalidInput(message))
                if message == "analysis issue analysis.unsupported-syntax with no policy impact cannot reference rules"
        ));
    }
}
