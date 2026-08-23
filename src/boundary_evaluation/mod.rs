mod evaluation;
mod model;

pub(crate) use evaluation::{decide, evaluate, matches_pattern};
pub(crate) use model::{
    AnalysisContext, AnalysisIssue, AnalysisProfile, BoundaryPolicy, BoundaryRule,
    BoundaryViolation, Dependency, DependencyKind, LogicalModule, ModuleDeclaration,
    ObservedDependencyGraph, PolicyEvaluation, RuleKind, Severity, SourceEvidence, TargetScope,
    ValidationDecision, ValidationResult, ViolationKind,
};
