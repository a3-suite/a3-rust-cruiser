mod loader;
pub(crate) mod normalized_sdd;
mod yaml;

pub(crate) use loader::ProjectBoundaryPolicyLoader;

use crate::boundary_validation::CruiserError;

pub(crate) fn invalid_boundary_policy_input(message: impl Into<String>) -> CruiserError {
    CruiserError::InvalidInput(format!("invalid boundary policy input: {}", message.into()))
}
