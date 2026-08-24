mod evaluation;
mod model;
mod pattern;

pub(crate) use evaluation::{decide, evaluate};
pub(crate) use model::{
    AnalysisContext, AnalysisIssue, AnalysisProfile, BoundaryPolicy, BoundaryRule,
    BoundaryViolation, Dependency, DependencyKind, LogicalModule, ModuleDeclaration,
    ObservedDependencyGraph, PolicyEvaluation, RuleKind, Severity, SourceEvidence, TargetScope,
    ValidationDecision, ValidationResult, ViolationKind,
};
pub(crate) use pattern::{is_valid_pattern, matches_pattern};
