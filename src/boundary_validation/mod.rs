mod error;
mod use_case;

pub(crate) use crate::boundary_evaluation::{
    AnalysisContext, AnalysisIssue, AnalysisProfile, BoundaryPolicy, BoundaryRule,
    BoundaryViolation, Dependency, DependencyKind, LogicalModule, ModuleDeclaration,
    ObservedDependencyGraph, PolicyEvaluation, RuleKind, Severity, SourceEvidence, TargetScope,
    ValidationDecision, ValidationResult, ViolationKind, matches_pattern,
};
pub(crate) use error::{CruiserError, Result};
pub(crate) use use_case::{BoundaryPolicyLoader, DependencyObserver, validate};
