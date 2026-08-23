use std::collections::BTreeSet;

use crate::model::{
    AnalysisIssue, BoundaryViolation, ObservedDependencyGraph, PolicyEvaluation,
    ValidationDecision, ValidationReport,
};

pub fn build(
    mut graph: ObservedDependencyGraph,
    mut violations: Vec<BoundaryViolation>,
    policy_evaluation: PolicyEvaluation,
    mut analysis_issues: Vec<AnalysisIssue>,
    decision: ValidationDecision,
    serialization: &str,
) -> ValidationReport {
    graph.analysis_context.features.sort();
    graph.analysis_context.features.dedup();
    graph.analysis_context.cfg.sort();
    graph.analysis_context.cfg.dedup();
    dedup_preserving_order(&mut graph.analysis_context.include);
    dedup_preserving_order(&mut graph.analysis_context.exclude);
    graph.modules.sort_by(|left, right| left.id.cmp(&right.id));
    graph.module_declarations.sort_by(|left, right| {
        (
            left.parent.as_str(),
            left.child.as_str(),
            left.evidence.path.as_str(),
            left.evidence.line,
            left.evidence.column,
        )
            .cmp(&(
                right.parent.as_str(),
                right.child.as_str(),
                right.evidence.path.as_str(),
                right.evidence.line,
                right.evidence.column,
            ))
    });
    graph.dependencies.sort_by(|left, right| {
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
    graph.excluded_scopes.sort();
    violations.sort_by(BoundaryViolation::canonical_cmp);
    analysis_issues.sort_by(|left, right| {
        (
            left.code.as_str(),
            left.affected_scope.as_str(),
            left.reason.as_str(),
        )
            .cmp(&(
                right.code.as_str(),
                right.affected_scope.as_str(),
                right.reason.as_str(),
            ))
    });
    ValidationReport {
        schema_version: "1".to_owned(),
        serialization: serialization.to_owned(),
        ordering: "canonical".to_owned(),
        analysis_context: graph.analysis_context,
        modules: graph.modules,
        module_declarations: graph.module_declarations,
        dependencies: graph.dependencies,
        excluded_scopes: graph.excluded_scopes,
        violations,
        policy_evaluation,
        analysis_issues,
        decision,
    }
}

fn dedup_preserving_order(values: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

pub fn to_json(report: &ValidationReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

pub fn to_text(report: &ValidationReport) -> String {
    let mut output = String::new();
    output.push_str(&format!("schema_version: {}\n", report.schema_version));
    output.push_str(&format!("serialization: {}\n", report.serialization));
    output.push_str(&format!("ordering: {}\n", report.ordering));
    output.push_str(&format!("decision: {}\n", decision_name(report.decision)));
    output.push_str(&format!("package: {}\n", report.analysis_context.package));
    output.push_str(&format!(
        "compilation_target: {}\n",
        report.analysis_context.compilation_target
    ));
    output.push_str(&format!(
        "target_triple: {}\n",
        report.analysis_context.target_triple
    ));
    output.push_str("features:\n");
    for feature in &report.analysis_context.features {
        output.push_str(&format!("  - {feature}\n"));
    }
    output.push_str("cfg:\n");
    for cfg in &report.analysis_context.cfg {
        output.push_str(&format!("  - {cfg}\n"));
    }
    output.push_str("include:\n");
    for include in &report.analysis_context.include {
        output.push_str(&format!("  - {include}\n"));
    }
    output.push_str("exclude:\n");
    for exclude in &report.analysis_context.exclude {
        output.push_str(&format!("  - {exclude}\n"));
    }
    output.push_str("modules:\n");
    for module in &report.modules {
        output.push_str(&format!("  - {}\n", module.id));
    }
    output.push_str("module_declarations:\n");
    for declaration in &report.module_declarations {
        output.push_str(&format!(
            "  - {} -> {} [{}:{}:{}]\n",
            declaration.parent,
            declaration.child,
            declaration.evidence.path,
            declaration.evidence.line,
            declaration.evidence.column
        ));
    }
    output.push_str("dependencies:\n");
    for dependency in &report.dependencies {
        output.push_str(&format!(
            "  - {} -> {} ({}, {}) [{}:{}:{}]\n",
            dependency.source,
            dependency.target,
            dependency.kind.as_str(),
            target_scope_name(dependency.target_scope),
            dependency.evidence.path,
            dependency.evidence.line,
            dependency.evidence.column
        ));
    }
    output.push_str("excluded_scopes:\n");
    for scope in &report.excluded_scopes {
        output.push_str(&format!("  - {scope}\n"));
    }
    output.push_str("violations:\n");
    for violation in &report.violations {
        output.push_str(&format!(
            "  - {} {} {} -> {} ({}, {}) [{}:{}:{}]\n",
            violation.rule_id,
            rule_name(violation.kind),
            violation.source,
            violation.target,
            severity_name(violation.severity),
            violation.dependency_kind.as_str(),
            violation.evidence.path,
            violation.evidence.line,
            violation.evidence.column
        ));
        if let Some(cycle_path) = &violation.cycle_path {
            output.push_str(&format!("    cycle_path: {}\n", cycle_path.join(" -> ")));
        }
    }
    output.push_str("analysis_issues:\n");
    for issue in &report.analysis_issues {
        output.push_str(&format!(
            "  - {} [{}] {}\n",
            issue.code, issue.affected_scope, issue.reason
        ));
    }
    output.push_str(&format!(
        "policy_evaluation: dependency={} cycle={} forbidden={} allowed={} circular={}\n",
        report.policy_evaluation.dependency_outcome,
        report.policy_evaluation.cycle_outcome,
        report.policy_evaluation.forbidden_match_handling,
        report.policy_evaluation.allowed_match_handling,
        report.policy_evaluation.cycle_handling
    ));
    output
}

fn decision_name(decision: ValidationDecision) -> &'static str {
    match decision {
        ValidationDecision::Conformant => "conformant",
        ValidationDecision::NonConformant => "non-conformant",
        ValidationDecision::Indeterminate => "indeterminate",
    }
}

fn rule_name(kind: crate::model::ViolationKind) -> &'static str {
    match kind {
        crate::model::ViolationKind::Forbidden => "forbidden",
        crate::model::ViolationKind::NotAllowed => "not-allowed",
        crate::model::ViolationKind::Circular => "circular",
    }
}

fn target_scope_name(scope: crate::model::TargetScope) -> &'static str {
    match scope {
        crate::model::TargetScope::InternalModule => "internal-module",
        crate::model::TargetScope::ExternalCrate => "external-crate",
    }
}

fn severity_name(severity: crate::model::Severity) -> &'static str {
    match severity {
        crate::model::Severity::Error => "error",
        crate::model::Severity::Warning => "warning",
    }
}

#[cfg(test)]
mod tests {
    use super::to_text;
    use crate::model::{AnalysisContext, PolicyEvaluation, ValidationDecision, ValidationReport};

    #[test]
    fn text_report_contains_the_decision() {
        // Arrange
        let report = ValidationReport {
            schema_version: "1".to_owned(),
            serialization: "text".to_owned(),
            ordering: "canonical".to_owned(),
            analysis_context: AnalysisContext {
                package: "fixture".to_owned(),
                compilation_target: "bin:fixture".to_owned(),
                features: Vec::new(),
                cfg: Vec::new(),
                target_triple: "x86_64-unknown-linux-gnu".to_owned(),
                include: Vec::new(),
                exclude: Vec::new(),
            },
            modules: Vec::new(),
            module_declarations: Vec::new(),
            dependencies: Vec::new(),
            excluded_scopes: Vec::new(),
            violations: Vec::new(),
            policy_evaluation: PolicyEvaluation {
                dependency_outcome: "allowed".to_owned(),
                cycle_outcome: "acyclic".to_owned(),
                forbidden_match_handling: "all-matches".to_owned(),
                allowed_match_handling: "union".to_owned(),
                cycle_handling: "independent".to_owned(),
            },
            analysis_issues: Vec::new(),
            decision: ValidationDecision::Conformant,
        };

        // Act
        let text = to_text(&report);

        // Assert
        assert!(text.contains("decision: conformant"));
    }
}
