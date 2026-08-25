mod evaluation;
mod model;
mod pattern;

pub(crate) use evaluation::{assess_policy_impact, decide, evaluate};
#[cfg(test)]
pub(crate) use model::PolicyImpact;
pub(crate) use model::{
    AnalysisContext, AnalysisIssue, AnalysisObservationOutcome, AnalysisProfile, BoundaryPolicy,
    BoundaryRule, BoundaryViolation, Dependency, DependencyKind, LogicalModule, ModuleDeclaration,
    ObservedDependencyGraph, PolicyEvaluation, PolicyImpactOutcome, RuleKind, Severity,
    SourceEvidence, TargetScope, ValidationDecision, ValidationResult, ViolationKind,
};
pub(crate) use pattern::{is_valid_pattern, matches_pattern};
