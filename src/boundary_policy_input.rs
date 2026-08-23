use std::path::Path;

use crate::{
    error::{CruiserError, Result},
    model::BoundaryPolicy,
};

pub fn load(path: &Path) -> Result<BoundaryPolicy> {
    let content = std::fs::read_to_string(path)?;
    let policy: BoundaryPolicy = serde_yaml::from_str(&content)
        .map_err(|error| CruiserError::Parse(format!("invalid boundary policy YAML: {error}")))?;
    crate::boundary_evaluation::validate_policy(&policy)?;
    Ok(policy)
}

#[cfg(test)]
mod tests {
    use super::load;

    #[test]
    fn missing_policy_is_an_error() {
        // Arrange
        let path = std::path::Path::new("policy-that-does-not-exist.yaml");

        // Act
        let result = load(path);

        // Assert
        assert!(result.is_err());
    }
}
