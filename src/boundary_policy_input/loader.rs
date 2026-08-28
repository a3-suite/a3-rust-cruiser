use crate::boundary_validation::{
    BoundaryPolicyInput, BoundaryPolicyLoader, LoadedBoundaryPolicy, PreparedBoundaryPolicy,
    Result, invalid_sdd,
};

use super::{sdd, yaml};

pub(crate) struct ProjectBoundaryPolicyLoader;

impl BoundaryPolicyLoader for ProjectBoundaryPolicyLoader {
    fn prepare(&self, input: &BoundaryPolicyInput) -> Result<PreparedBoundaryPolicy> {
        match input {
            BoundaryPolicyInput::Yaml { path } => {
                yaml::load(path).map(PreparedBoundaryPolicy::Yaml)
            }
            BoundaryPolicyInput::SddDesign {
                logical_structure,
                projection,
                directory_structure,
            } => sdd::prepare(logical_structure, projection, directory_structure)
                .map(PreparedBoundaryPolicy::Sdd),
        }
    }

    fn finalize(
        &self,
        prepared: PreparedBoundaryPolicy,
        project_root: &std::path::Path,
        target_source: Option<&std::path::Path>,
    ) -> Result<LoadedBoundaryPolicy> {
        match prepared {
            PreparedBoundaryPolicy::Yaml(policy) => Ok(LoadedBoundaryPolicy {
                policy,
                analysis_issues: Vec::new(),
                component_attribution: None,
            }),
            PreparedBoundaryPolicy::Sdd(design) => {
                let target_source = target_source.ok_or_else(|| {
                    invalid_sdd("selected Cargo target source is required for SDD policy input")
                })?;
                sdd::project(design, project_root, target_source)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::ProjectBoundaryPolicyLoader;
    use crate::boundary_validation::{
        BoundaryPolicyLoader, CruiserError, PreparedBoundaryPolicy, PreparedSddDesign,
    };

    #[test]
    fn should_reject_sdd_finalization_without_target_source() {
        // Arrange
        let loader = ProjectBoundaryPolicyLoader;

        // Act
        let result = loader.finalize(
            PreparedBoundaryPolicy::Sdd(PreparedSddDesign::valid_for_test()),
            Path::new("project"),
            None,
        );

        // Assert
        assert!(matches!(
            result,
            Err(CruiserError::InvalidInput(message))
                if message == "invalid SDD boundary policy input: selected Cargo target source is required for SDD policy input"
        ));
    }
}
