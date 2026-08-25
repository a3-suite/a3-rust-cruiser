use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisProfile {
    Compilation,
    SourceUnion,
}

impl AnalysisProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compilation => "compilation",
            Self::SourceUnion => "source-union",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisContext {
    pub analysis_profile: AnalysisProfile,
    pub package: String,
    pub compilation_target: String,
    pub features: Vec<String>,
    pub cfg: Vec<String>,
    pub target_triple: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEvidence {
    pub path: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalModule {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDeclaration {
    pub declaration_id: String,
    pub parent: String,
    pub child: String,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetScope {
    InternalModule,
    ExternalCrate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub candidate_id: String,
    pub source: String,
    pub target: String,
    pub kind: DependencyKind,
    pub target_scope: TargetScope,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedDependencyGraph {
    pub analysis_context: AnalysisContext,
    pub modules: Vec<LogicalModule>,
    pub module_declarations: Vec<ModuleDeclaration>,
    pub dependencies: Vec<Dependency>,
    pub excluded_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryPolicy {
    pub rules: Vec<BoundaryRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryRule {
    pub id: String,
    pub kind: RuleKind,
    pub severity: Severity,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub modules: Vec<String>,
    pub dependency_kinds: Vec<DependencyKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    Forbidden,
    Allowed,
    Circular,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ViolationKind {
    Forbidden,
    NotAllowed,
    Circular,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryViolation {
    pub kind: ViolationKind,
    pub rule_id: String,
    pub severity: Severity,
    pub source: String,
    pub target: String,
    pub dependency_kind: DependencyKind,
    pub evidence: SourceEvidence,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisIssue {
    pub code: String,
    pub reason: String,
    pub affected_scope: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationDecision {
    Conformant,
    NonConformant,
    Indeterminate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyEvaluation {
    pub dependency_outcome: String,
    pub cycle_outcome: String,
    pub forbidden_match_handling: String,
    pub allowed_match_handling: String,
    pub cycle_handling: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationResult {
    pub graph: ObservedDependencyGraph,
    pub violations: Vec<BoundaryViolation>,
    pub policy_evaluation: PolicyEvaluation,
    pub analysis_issues: Vec<AnalysisIssue>,
    pub decision: ValidationDecision,
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::{BoundaryViolation, DependencyKind, Severity, SourceEvidence, ViolationKind};

    #[test]
    fn should_order_violations_by_rule_id_first() {
        // Arrange
        let violation = |rule_id: &str| BoundaryViolation {
            kind: ViolationKind::Forbidden,
            rule_id: rule_id.to_owned(),
            severity: Severity::Error,
            source: "crate::application".to_owned(),
            target: "crate::infrastructure".to_owned(),
            dependency_kind: DependencyKind::Import,
            evidence: SourceEvidence {
                path: "src/application.rs".to_owned(),
                line: 1,
                column: 1,
            },
            cycle_path: None,
        };
        let first = violation("first");
        let second = violation("second");

        // Act
        let ordering = first.canonical_cmp(&second);

        // Assert
        assert_eq!(ordering, Ordering::Less);
    }
}
