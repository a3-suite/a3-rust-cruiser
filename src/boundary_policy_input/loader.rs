use crate::boundary_validation::{
    BoundaryPolicyAssessment, BoundaryPolicyInput, BoundaryPolicyLoader, LoadedBoundaryPolicy,
    PreparedBoundaryPolicy, Result,
};

use super::{normalized_sdd, yaml};

pub(crate) struct ProjectBoundaryPolicyLoader;

impl BoundaryPolicyLoader for ProjectBoundaryPolicyLoader {
    fn prepare(&self, input: &BoundaryPolicyInput) -> Result<PreparedBoundaryPolicy> {
        match input {
            BoundaryPolicyInput::Yaml { path } => {
                yaml::load(path).map(PreparedBoundaryPolicy::Yaml)
            }
            BoundaryPolicyInput::ResolvedImplementationAttribution { path } => {
                normalized_sdd::prepare(path)
                    .map(PreparedBoundaryPolicy::ResolvedImplementationAttribution)
            }
        }
    }

    fn finalize(
        &self,
        prepared: PreparedBoundaryPolicy,
        project_root: &std::path::Path,
    ) -> Result<LoadedBoundaryPolicy> {
        match prepared {
            PreparedBoundaryPolicy::Yaml(policy) => Ok(LoadedBoundaryPolicy {
                policy,
                analysis_issues: Vec::new(),
                assessment: BoundaryPolicyAssessment::None,
            }),
            PreparedBoundaryPolicy::ResolvedImplementationAttribution(prepared) => {
                normalized_sdd::finalize(prepared, project_root)
            }
        }
    }
}
