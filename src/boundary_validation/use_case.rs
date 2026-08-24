use std::{collections::BTreeSet, path::Path};

use crate::boundary_evaluation::{
    AnalysisContext, AnalysisIssue, BoundaryPolicy, ObservedDependencyGraph, RuleKind,
    ValidationResult, decide, evaluate, is_valid_pattern,
};

use super::{CruiserError, Result};

pub trait BoundaryPolicyLoader {
    fn load(&self, path: &Path) -> Result<BoundaryPolicy>;
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
    policy_path: &Path,
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
    let policy = policy_loader.load(policy_path)?;
    validate_policy(&policy)?;
    validate_module_patterns(&context.include)?;
    validate_module_patterns(&context.exclude)?;
    let (graph, analysis_issues) = observer.observe(project_root, context)?;
    let (violations, policy_evaluation) = evaluate(&graph, &policy);
    let decision = decide(analysis_issues.len(), violations.len());
    Ok(ValidationResult {
        graph,
        violations,
        policy_evaluation,
        analysis_issues,
        decision,
    })
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
    use super::{BoundaryPolicyLoader, DependencyObserver, validate};
    use crate::boundary_validation::{
        AnalysisContext, AnalysisIssue, BoundaryPolicy, BoundaryRule, ObservedDependencyGraph,
        Result, RuleKind, Severity, ValidationDecision,
    };

    struct StubPolicyLoader;

    impl BoundaryPolicyLoader for StubPolicyLoader {
        fn load(&self, _path: &std::path::Path) -> Result<BoundaryPolicy> {
            Ok(BoundaryPolicy { rules: Vec::new() })
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

    struct InvalidPolicyLoader;

    impl BoundaryPolicyLoader for InvalidPolicyLoader {
        fn load(&self, _path: &std::path::Path) -> Result<BoundaryPolicy> {
            let rule = BoundaryRule {
                id: "duplicate".to_owned(),
                kind: RuleKind::Forbidden,
                severity: Severity::Error,
                from: vec!["crate::application".to_owned()],
                to: vec!["crate::infrastructure".to_owned()],
                modules: Vec::new(),
                dependency_kinds: Vec::new(),
            };
            Ok(BoundaryPolicy {
                rules: vec![rule.clone(), rule],
            })
        }
    }

    #[test]
    fn validation_coordinates_ports() {
        // Arrange
        let project_root = std::path::Path::new("project-that-does-not-exist");
        let policy = std::path::Path::new("policy-that-does-not-exist.yaml");
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
            policy,
        )
        .expect("port-backed validation should succeed");

        // Assert
        assert_eq!(result.decision, ValidationDecision::Conformant);
    }

    #[test]
    fn validation_rejects_invalid_policy_from_a_port() {
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
            std::path::Path::new("unused-policy"),
        );

        // Assert
        assert!(result.is_err());
    }
}
