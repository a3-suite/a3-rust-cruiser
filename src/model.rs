use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisContext {
    pub package: String,
    pub compilation_target: String,
    pub features: Vec<String>,
    pub cfg: Vec<String>,
    pub target_triple: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceEvidence {
    pub path: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogicalModule {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModuleDeclaration {
    pub declaration_id: String,
    pub parent: String,
    pub child: String,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyKind {
    Import,
    ReExport,
    PathReference,
}

impl DependencyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::ReExport => "re-export",
            Self::PathReference => "path-reference",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TargetScope {
    InternalModule,
    ExternalCrate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dependency {
    pub candidate_id: String,
    pub source: String,
    pub target: String,
    pub kind: DependencyKind,
    pub target_scope: TargetScope,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObservedDependencyGraph {
    pub analysis_context: AnalysisContext,
    pub modules: Vec<LogicalModule>,
    pub module_declarations: Vec<ModuleDeclaration>,
    pub dependencies: Vec<Dependency>,
    pub excluded_scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BoundaryPolicy {
    pub rules: Vec<BoundaryRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BoundaryRule {
    pub id: String,
    pub kind: RuleKind,
    pub severity: Severity,
    #[serde(default)]
    pub from: Vec<String>,
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default)]
    pub dependency_kinds: Vec<DependencyKind>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuleKind {
    Forbidden,
    Allowed,
    Circular,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum ViolationKind {
    Forbidden,
    NotAllowed,
    Circular,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoundaryViolation {
    pub kind: ViolationKind,
    pub rule_id: String,
    pub severity: Severity,
    pub source: String,
    pub target: String,
    pub dependency_kind: DependencyKind,
    pub evidence: SourceEvidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle_path: Option<Vec<String>>,
}

impl BoundaryViolation {
    pub(crate) fn canonical_cmp(&self, other: &Self) -> Ordering {
        (
            self.rule_id.as_str(),
            self.source.as_str(),
            self.target.as_str(),
            self.dependency_kind,
            self.evidence.path.as_str(),
            self.evidence.line,
            self.evidence.column,
            self.kind,
            self.severity,
            self.cycle_path.as_deref(),
        )
            .cmp(&(
                other.rule_id.as_str(),
                other.source.as_str(),
                other.target.as_str(),
                other.dependency_kind,
                other.evidence.path.as_str(),
                other.evidence.line,
                other.evidence.column,
                other.kind,
                other.severity,
                other.cycle_path.as_deref(),
            ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisIssue {
    pub code: String,
    pub reason: String,
    pub affected_scope: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ValidationDecision {
    Conformant,
    NonConformant,
    Indeterminate,
}

impl ValidationDecision {
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Conformant => 0,
            Self::NonConformant => 1,
            Self::Indeterminate => 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyEvaluation {
    pub dependency_outcome: String,
    pub cycle_outcome: String,
    pub forbidden_match_handling: String,
    pub allowed_match_handling: String,
    pub cycle_handling: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[cfg(test)]
mod tests {
    use super::ValidationDecision;

    #[test]
    fn decision_exit_codes_are_stable() {
        // Arrange
        let decisions = [
            (ValidationDecision::Conformant, 0),
            (ValidationDecision::NonConformant, 1),
            (ValidationDecision::Indeterminate, 2),
        ];

        // Act
        let exit_codes: Vec<u8> = decisions
            .iter()
            .map(|(decision, _)| decision.exit_code())
            .collect();

        // Assert
        assert_eq!(
            exit_codes,
            decisions.iter().map(|(_, code)| *code).collect::<Vec<_>>()
        );
    }
}
