use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::boundary_validation::{
    AnalysisContext, AnalysisIssue, BoundaryViolation, Dependency, LogicalModule,
    ModuleDeclaration, PolicyEvaluation, ValidationDecision, ValidationResult,
};

pub struct ValidationReport {
    pub schema_version: String,
    pub serialization: String,
    pub ordering: String,
    pub analysis_context: AnalysisContext,
    pub modules: Vec<LogicalModule>,
    pub module_declarations: Vec<ModuleDeclaration>,
    pub dependencies: Vec<Dependency>,
    pub excluded_scopes: Vec<String>,
    pub violations: Vec<BoundaryViolation>,
    pub policy_evaluation: PolicyEvaluation,
    pub analysis_issues: Vec<AnalysisIssue>,
    pub decision: ValidationDecision,
}

pub fn build(result: ValidationResult, serialization: &str) -> ValidationReport {
    let ValidationResult {
        mut graph,
        mut violations,
        policy_evaluation,
        mut analysis_issues,
        decision,
    } = result;
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
    serde_json::to_string_pretty(&report_value(report))
}

fn report_value(report: &ValidationReport) -> Value {
    json!({
        "schema_version": report.schema_version,
        "serialization": report.serialization,
        "ordering": report.ordering,
        "analysis_context": analysis_context_value(&report.analysis_context),
        "modules": report.modules.iter().map(|module| json!({"id": module.id})).collect::<Vec<_>>(),
        "module_declarations": report.module_declarations.iter().map(module_declaration_value).collect::<Vec<_>>(),
        "dependencies": report.dependencies.iter().map(dependency_value).collect::<Vec<_>>(),
        "excluded_scopes": report.excluded_scopes,
        "violations": report.violations.iter().map(violation_value).collect::<Vec<_>>(),
        "policy_evaluation": {
            "dependency_outcome": report.policy_evaluation.dependency_outcome,
            "cycle_outcome": report.policy_evaluation.cycle_outcome,
            "forbidden_match_handling": report.policy_evaluation.forbidden_match_handling,
            "allowed_match_handling": report.policy_evaluation.allowed_match_handling,
            "cycle_handling": report.policy_evaluation.cycle_handling,
        },
        "analysis_issues": report.analysis_issues.iter().map(|issue| json!({
            "code": issue.code,
            "reason": issue.reason,
            "affected_scope": issue.affected_scope,
        })).collect::<Vec<_>>(),
        "decision": decision_name(report.decision),
    })
}

fn analysis_context_value(context: &AnalysisContext) -> Value {
    json!({
        "analysis_profile": context.analysis_profile.as_str(),
        "package": context.package,
        "compilation_target": context.compilation_target,
        "features": context.features,
        "cfg": context.cfg,
        "target_triple": context.target_triple,
        "include": context.include,
        "exclude": context.exclude,
    })
}

fn source_evidence_value(evidence: &crate::boundary_validation::SourceEvidence) -> Value {
    json!({
        "path": evidence.path,
        "line": evidence.line,
        "column": evidence.column,
    })
}

fn module_declaration_value(declaration: &ModuleDeclaration) -> Value {
    json!({
        "declaration_id": declaration.declaration_id,
        "parent": declaration.parent,
        "child": declaration.child,
        "evidence": source_evidence_value(&declaration.evidence),
    })
}

fn dependency_value(dependency: &Dependency) -> Value {
    json!({
        "candidate_id": dependency.candidate_id,
        "source": dependency.source,
        "target": dependency.target,
        "kind": dependency.kind.as_str(),
        "target_scope": target_scope_name(dependency.target_scope),
        "evidence": source_evidence_value(&dependency.evidence),
    })
}

fn violation_value(violation: &BoundaryViolation) -> Value {
    let mut value = Map::new();
    value.insert("kind".to_owned(), json!(rule_name(violation.kind)));
    value.insert("rule_id".to_owned(), json!(violation.rule_id));
    value.insert(
        "severity".to_owned(),
        json!(severity_name(violation.severity)),
    );
    value.insert("source".to_owned(), json!(violation.source));
    value.insert("target".to_owned(), json!(violation.target));
    value.insert(
        "dependency_kind".to_owned(),
        json!(violation.dependency_kind.as_str()),
    );
    value.insert(
        "evidence".to_owned(),
        source_evidence_value(&violation.evidence),
    );
    if let Some(cycle_path) = &violation.cycle_path {
        value.insert("cycle_path".to_owned(), json!(cycle_path));
    }
    Value::Object(value)
}

pub fn to_text(report: &ValidationReport) -> String {
    let mut output = String::new();
    output.push_str(&format!("schema_version: {}\n", report.schema_version));
    output.push_str(&format!("serialization: {}\n", report.serialization));
    output.push_str(&format!("ordering: {}\n", report.ordering));
    output.push_str(&format!("decision: {}\n", decision_name(report.decision)));
    output.push_str(&format!(
        "analysis_profile: {}\n",
        report.analysis_context.analysis_profile.as_str()
    ));
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

fn rule_name(kind: crate::boundary_validation::ViolationKind) -> &'static str {
    match kind {
        crate::boundary_validation::ViolationKind::Forbidden => "forbidden",
        crate::boundary_validation::ViolationKind::NotAllowed => "not-allowed",
        crate::boundary_validation::ViolationKind::Circular => "circular",
    }
}

fn target_scope_name(scope: crate::boundary_validation::TargetScope) -> &'static str {
    match scope {
        crate::boundary_validation::TargetScope::InternalModule => "internal-module",
        crate::boundary_validation::TargetScope::ExternalCrate => "external-crate",
    }
}

fn severity_name(severity: crate::boundary_validation::Severity) -> &'static str {
    match severity {
        crate::boundary_validation::Severity::Error => "error",
        crate::boundary_validation::Severity::Warning => "warning",
    }
}

#[cfg(test)]
mod tests {
    use super::ValidationReport;
    use super::to_text;
    use crate::boundary_validation::{
        AnalysisContext, AnalysisProfile, PolicyEvaluation, ValidationDecision,
    };

    #[test]
    fn text_report_contains_the_decision() {
        // Arrange
        let report = ValidationReport {
            schema_version: "1".to_owned(),
            serialization: "text".to_owned(),
            ordering: "canonical".to_owned(),
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
