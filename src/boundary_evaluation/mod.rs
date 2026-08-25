mod evaluation;
mod model;
mod pattern;

pub(crate) use evaluation::{assess_component_attribution, assess_policy_impact, decide, evaluate};
#[cfg(test)]
pub(crate) use model::PolicyImpact;
pub(crate) use model::{
    AnalysisContext, AnalysisIssue, AnalysisObservationOutcome, AnalysisProfile, BoundaryPolicy,
    BoundaryRule, BoundaryViolation, ComponentAttributionModel, Dependency, DependencyKind,
    LoadedBoundaryPolicy, LogicalModule, ModuleDeclaration, ObservedDependencyGraph,
    PolicyEvaluation, PolicyImpactOutcome, RuleKind, Severity, SourceEvidence, TargetScope,
    ValidationDecision, ValidationResult, ViolationKind,
};
pub(crate) use pattern::{is_valid_pattern, matches_pattern};
