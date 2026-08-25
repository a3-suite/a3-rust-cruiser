use crate::boundary_validation::{
    BoundaryPolicyInput, BoundaryPolicyLoader, LoadedBoundaryPolicy, Result,
};

use super::{sdd, yaml};

pub(crate) struct ProjectBoundaryPolicyLoader;

impl BoundaryPolicyLoader for ProjectBoundaryPolicyLoader {
    fn load(
        &self,
        input: &BoundaryPolicyInput,
        project_root: &std::path::Path,
        target_source: &std::path::Path,
    ) -> Result<LoadedBoundaryPolicy> {
        match input {
            BoundaryPolicyInput::Yaml { path } => {
                yaml::load(path).map(|policy| LoadedBoundaryPolicy {
                    policy,
                    analysis_issues: Vec::new(),
                    component_attribution: None,
                })
            }
            BoundaryPolicyInput::SddDesign {
                logical_structure,
                projection,
                directory_structure,
            } => sdd::load(
                logical_structure,
                projection,
                directory_structure,
                project_root,
                target_source,
            ),
        }
    }
}
