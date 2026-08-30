mod error;
mod sdd_policy;
mod use_case;

#[cfg(test)]
pub(crate) use crate::boundary_evaluation::UnresolvedModuleAttribution;
pub(crate) use crate::boundary_evaluation::{
    AnalysisContext, AnalysisIssue, AnalysisProfile, BoundaryPolicy, BoundaryRule,
    BoundaryViolation, Dependency, DependencyKind, LoadedBoundaryPolicy, LogicalModule,
    ModuleDeclaration, NormalizedSddCoverage, NormalizedSddPolicy, NormalizedSddProvenance,
    NormalizedSddSourceArtifact, ObservedDependencyGraph, PolicyEvaluation, RuleKind,
    ScopedComponentAttribution, Severity, SourceEvidence, TargetScope, ValidationDecision,
    ValidationResult, ViolationKind, matches_pattern,
};
pub(crate) use error::{CruiserError, Result};
pub(crate) use sdd_policy::{
    DirectoryStructure, LogicalStructure, PreparedSddDesign, Projection, SddDesignInput,
    invalid_sdd, prepare_sdd_design, project_sdd_policy,
};
pub(crate) use use_case::{
    BoundaryPolicyInput, BoundaryPolicyLoader, DependencyObserver, PreparedBoundaryPolicy, validate,
};
