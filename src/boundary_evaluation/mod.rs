mod evaluation;
mod model;
mod normalized_sdd;
mod pattern;

pub(crate) use evaluation::{assess_policy_impact, decide, evaluate};
pub(crate) use model::{
    AnalysisContext, AnalysisIssue, AnalysisObservationOutcome, AnalysisProfile, BoundaryPolicy,
    BoundaryPolicyAssessment, BoundaryRule, BoundaryViolation, Dependency, DependencyKind,
    LoadedBoundaryPolicy, LogicalModule, ModuleDeclaration, NormalizedSddCoverage,
    NormalizedSddPolicy, NormalizedSddProvenance, NormalizedSddSourceArtifact,
    ObservedDependencyGraph, PolicyEvaluation, PolicyImpactOutcome, RuleKind,
    ScopedComponentAttribution, Severity, SourceEvidence, TargetScope, ValidationDecision,
    ValidationResult, ViolationKind,
};
#[cfg(test)]
pub(crate) use model::{PolicyImpact, UnresolvedModuleAttribution};
pub(crate) use normalized_sdd::assess_normalized_sdd;
pub(crate) use pattern::{is_valid_pattern, matches_pattern};
