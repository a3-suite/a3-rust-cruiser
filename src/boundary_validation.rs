use std::path::Path;

use crate::{
    boundary_evaluation::{decide, evaluate},
    boundary_policy_input,
    error::Result,
    model::{AnalysisContext, ValidationReport},
    report, rust_dependency_observation,
};

pub fn validate(
    project_root: &Path,
    context: AnalysisContext,
    policy_path: &Path,
    serialization: &str,
) -> Result<ValidationReport> {
    if context.package.is_empty()
        || context.compilation_target.is_empty()
        || context.target_triple.is_empty()
    {
        return Err(crate::error::CruiserError::InvalidInput(
            "package, target, and target_triple are required".to_owned(),
        ));
    }
    let policy = boundary_policy_input::load(policy_path)?;
    crate::boundary_evaluation::validate_module_patterns(&context.include)?;
    crate::boundary_evaluation::validate_module_patterns(&context.exclude)?;
    let (graph, analysis_issues) = rust_dependency_observation::observe(project_root, context)?;
    let (violations, policy_evaluation) = evaluate(&graph, &policy);
    let decision = decide(analysis_issues.len(), violations.len());
    Ok(report::build(
        graph,
        violations,
        policy_evaluation,
        analysis_issues,
        decision,
        serialization,
    ))
}

#[cfg(test)]
mod tests {
    use super::validate;

    #[test]
    fn validation_requires_an_existing_project() {
        // Arrange
        let project_root = std::path::Path::new("project-that-does-not-exist");
        let policy = std::path::Path::new("policy-that-does-not-exist.yaml");
        let context = crate::model::AnalysisContext {
            package: "fixture".to_owned(),
            compilation_target: "bin:fixture".to_owned(),
            features: Vec::new(),
            cfg: Vec::new(),
            target_triple: "x86_64-unknown-linux-gnu".to_owned(),
            include: Vec::new(),
            exclude: Vec::new(),
        };

        // Act
        let result = validate(project_root, context, policy, "json");

        // Assert
        assert!(result.is_err());
    }
}
