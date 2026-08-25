mod sdd;
mod yaml;

use crate::boundary_validation::{
    BoundaryPolicy, BoundaryPolicyInput, BoundaryPolicyLoader, Result,
};

pub(crate) struct ProjectBoundaryPolicyLoader;

impl BoundaryPolicyLoader for ProjectBoundaryPolicyLoader {
    fn load(
        &self,
        input: &BoundaryPolicyInput,
        project_root: &std::path::Path,
        target_source: &std::path::Path,
    ) -> Result<BoundaryPolicy> {
        match input {
            BoundaryPolicyInput::Yaml { path } => yaml::load(path),
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
