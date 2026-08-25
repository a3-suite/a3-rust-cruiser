use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use crate::boundary_evaluation::{
    AnalysisContext, AnalysisIssue, AnalysisObservationOutcome, BoundaryPolicy,
    LoadedBoundaryPolicy, ObservedDependencyGraph, PolicyImpactOutcome, RuleKind, ValidationResult,
    assess_component_attribution, assess_policy_impact, decide, evaluate, is_valid_pattern,
};

use super::{CruiserError, Result};

pub trait BoundaryPolicyLoader {
    fn load(
        &self,
        input: &BoundaryPolicyInput,
        project_root: &Path,
        target_source: &Path,
    ) -> Result<LoadedBoundaryPolicy>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundaryPolicyInput {
    Yaml {
        path: PathBuf,
    },
    SddDesign {
        logical_structure: PathBuf,
        projection: PathBuf,
        directory_structure: PathBuf,
    },
}

pub trait DependencyObserver {
    fn target_source(&self, project_root: &Path, context: &AnalysisContext) -> Result<PathBuf>;

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
    let target_source = match policy_input {
        BoundaryPolicyInput::Yaml { .. } => PathBuf::new(),
        BoundaryPolicyInput::SddDesign { .. } => observer.target_source(project_root, &context)?,
    };
    let loaded = policy_loader.load(policy_input, project_root, &target_source)?;
    let mut policy = loaded.policy;
    let mut analysis_issues = loaded.analysis_issues;
    validate_policy(&policy)?;
    validate_module_patterns(&context.include)?;
    validate_module_patterns(&context.exclude)?;
    let (graph, observation_issues) = observer.observe(project_root, context)?;
    analysis_issues.extend(observation_issues);
    validate_analysis_issues(&analysis_issues)?;
    if let Some(component_attribution) = loaded.component_attribution.as_ref() {
        policy.rules.extend(assess_component_attribution(
            &mut analysis_issues,
            &graph,
            component_attribution,
        ));
    }
    assess_policy_impact(&mut analysis_issues, &policy);
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
    use super::{BoundaryPolicyLoader, DependencyObserver, validate, validate_analysis_issues};
    use crate::boundary_validation::{
        AnalysisContext, AnalysisIssue, BoundaryPolicy, BoundaryRule, LoadedBoundaryPolicy,
        ObservedDependencyGraph, Result, RuleKind, Severity, ValidationDecision,
    };

    struct StubPolicyLoader;

    impl BoundaryPolicyLoader for StubPolicyLoader {
        fn load(
            &self,
            _input: &super::BoundaryPolicyInput,
            _project_root: &std::path::Path,
            _target_source: &std::path::Path,
        ) -> Result<LoadedBoundaryPolicy> {
            Ok(LoadedBoundaryPolicy {
                policy: BoundaryPolicy { rules: Vec::new() },
                analysis_issues: Vec::new(),
                component_attribution: None,
            })
        }
    }

    struct StubDependencyObserver;

    impl DependencyObserver for StubDependencyObserver {
        fn target_source(
            &self,
            project_root: &std::path::Path,
            _context: &AnalysisContext,
        ) -> Result<std::path::PathBuf> {
            Ok(project_root.join("src/main.rs"))
        }

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

    struct InvalidPolicyLoader;

    impl BoundaryPolicyLoader for InvalidPolicyLoader {
        fn load(
            &self,
            _input: &super::BoundaryPolicyInput,
            _project_root: &std::path::Path,
            _target_source: &std::path::Path,
        ) -> Result<LoadedBoundaryPolicy> {
            let rule = BoundaryRule {
                id: "duplicate".to_owned(),
                kind: RuleKind::Forbidden,
                severity: Severity::Error,
                from: vec!["crate::application".to_owned()],
                to: vec!["crate::infrastructure".to_owned()],
                modules: Vec::new(),
                dependency_kinds: Vec::new(),
            };
            Ok(LoadedBoundaryPolicy {
                policy: BoundaryPolicy {
                    rules: vec![rule.clone(), rule],
                },
                analysis_issues: Vec::new(),
                component_attribution: None,
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

        // Act
        let result = validate_analysis_issues(&[issue]);

        // Assert
        assert!(result.is_err());
    }
}
