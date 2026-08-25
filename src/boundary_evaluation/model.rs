use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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
pub struct LoadedBoundaryPolicy {
    pub policy: BoundaryPolicy,
    pub analysis_issues: Vec<AnalysisIssue>,
    pub component_attribution: Option<ComponentAttributionModel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentAttributionModel {
    pub component_modules: BTreeMap<String, String>,
    pub declared_dependencies: BTreeSet<(String, String)>,
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
    pub observation: AnalysisObservation,
    pub policy_impact: PolicyImpact,
}

impl AnalysisIssue {
    pub fn unresolved_dependency_candidate(
        code: impl Into<String>,
        reason: impl Into<String>,
        affected_scope: impl Into<String>,
        source_module: String,
        evidence: SourceEvidence,
        candidate_targets: Vec<String>,
        dependency_kinds: Vec<DependencyKind>,
    ) -> Self {
        Self {
            code: code.into(),
            reason: reason.into(),
            affected_scope: affected_scope.into(),
            observation: AnalysisObservation {
                outcome: AnalysisObservationOutcome::UnresolvedDependencyCandidate,
                source_module: Some(source_module),
                evidence: Some(evidence),
                candidate_targets,
                candidate_components: Vec::new(),
                dependency_kinds,
            },
            policy_impact: PolicyImpact::possible(Vec::new()),
        }
    }

    pub fn analysis_context_unresolved(
        code: impl Into<String>,
        reason: impl Into<String>,
        affected_scope: impl Into<String>,
        source_module: Option<String>,
        evidence: Option<SourceEvidence>,
        dependency_kinds: Vec<DependencyKind>,
    ) -> Self {
        Self {
            code: code.into(),
            reason: reason.into(),
            affected_scope: affected_scope.into(),
            observation: AnalysisObservation {
                outcome: AnalysisObservationOutcome::AnalysisContextUnresolved,
                source_module,
                evidence,
                candidate_targets: Vec::new(),
                candidate_components: Vec::new(),
                dependency_kinds,
            },
            policy_impact: PolicyImpact::possible(Vec::new()),
        }
    }

    pub fn component_attribution_unresolved(
        code: impl Into<String>,
        reason: impl Into<String>,
        affected_scope: impl Into<String>,
        candidate_components: Vec<String>,
    ) -> Self {
        Self {
            code: code.into(),
            reason: reason.into(),
            affected_scope: affected_scope.into(),
            observation: AnalysisObservation {
                outcome: AnalysisObservationOutcome::ComponentAttributionUnresolved,
                source_module: None,
                evidence: None,
                candidate_targets: Vec::new(),
                candidate_components,
                dependency_kinds: Vec::new(),
            },
            policy_impact: PolicyImpact::possible(Vec::new()),
        }
    }

    // A producer must call this only after proving that the observed syntax cannot alter an edge.
    #[allow(dead_code)]
    pub fn dependency_neutral(
        code: impl Into<String>,
        reason: impl Into<String>,
        affected_scope: impl Into<String>,
        source_module: Option<String>,
        evidence: Option<SourceEvidence>,
    ) -> Self {
        Self {
            code: code.into(),
            reason: reason.into(),
            affected_scope: affected_scope.into(),
            observation: AnalysisObservation {
                outcome: AnalysisObservationOutcome::DependencyNeutral,
                source_module,
                evidence,
                candidate_targets: Vec::new(),
                candidate_components: Vec::new(),
                dependency_kinds: Vec::new(),
            },
            policy_impact: PolicyImpact::none(),
        }
    }

    pub(crate) fn canonical_cmp(&self, other: &Self) -> Ordering {
        (
            self.code.as_str(),
            self.affected_scope.as_str(),
            self.reason.as_str(),
            self.observation.outcome,
            self.observation.source_module.as_deref(),
            self.observation.evidence.as_ref(),
            self.observation.candidate_targets.as_slice(),
            self.observation.candidate_components.as_slice(),
            self.observation.dependency_kinds.as_slice(),
            self.policy_impact.outcome,
            self.policy_impact.rule_ids.as_slice(),
        )
            .cmp(&(
                other.code.as_str(),
                other.affected_scope.as_str(),
                other.reason.as_str(),
                other.observation.outcome,
                other.observation.source_module.as_deref(),
                other.observation.evidence.as_ref(),
                other.observation.candidate_targets.as_slice(),
                other.observation.candidate_components.as_slice(),
                other.observation.dependency_kinds.as_slice(),
                other.policy_impact.outcome,
                other.policy_impact.rule_ids.as_slice(),
            ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisObservation {
    pub outcome: AnalysisObservationOutcome,
    pub source_module: Option<String>,
    pub evidence: Option<SourceEvidence>,
    pub candidate_targets: Vec<String>,
    pub candidate_components: Vec<String>,
    pub dependency_kinds: Vec<DependencyKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnalysisObservationOutcome {
    UnresolvedDependencyCandidate,
    DependencyNeutral,
    AnalysisContextUnresolved,
    ComponentAttributionUnresolved,
}

impl AnalysisObservationOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnresolvedDependencyCandidate => "unresolved-dependency-candidate",
            Self::DependencyNeutral => "dependency-neutral",
            Self::AnalysisContextUnresolved => "analysis-context-unresolved",
            Self::ComponentAttributionUnresolved => "component-attribution-unresolved",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyImpact {
    pub outcome: PolicyImpactOutcome,
    pub rule_ids: Vec<String>,
}

impl PolicyImpact {
    pub fn none() -> Self {
        Self {
            outcome: PolicyImpactOutcome::None,
            rule_ids: Vec::new(),
        }
    }

    pub fn possible(rule_ids: Vec<String>) -> Self {
        Self {
            outcome: PolicyImpactOutcome::Possible,
            rule_ids,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PolicyImpactOutcome {
    None,
    Possible,
}

impl PolicyImpactOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Possible => "possible",
        }
    }
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
