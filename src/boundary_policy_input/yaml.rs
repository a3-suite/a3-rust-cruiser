use std::path::Path;

use serde::Deserialize;

use crate::boundary_validation::{
    BoundaryPolicy, BoundaryRule, CruiserError, DependencyKind, Result, RuleKind, Severity,
};

pub(super) fn load(path: &Path) -> Result<BoundaryPolicy> {
    let content = std::fs::read_to_string(path)
        .map_err(|error| CruiserError::io(format!("boundary policy {}", path.display()), error))?;
    let dto: BoundaryPolicyDto = serde_yaml::from_str(&content).map_err(|error| {
        CruiserError::parse(
            format!("invalid boundary policy YAML {}", path.display()),
            error,
        )
    })?;
    Ok(dto.into_domain())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundaryPolicyDto {
    rules: Vec<BoundaryRuleDto>,
}

impl BoundaryPolicyDto {
    fn into_domain(self) -> BoundaryPolicy {
        BoundaryPolicy {
            rules: self
                .rules
                .into_iter()
                .map(BoundaryRuleDto::into_domain)
                .collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundaryRuleDto {
    id: String,
    kind: RuleKindDto,
    severity: SeverityDto,
    #[serde(default)]
    from: Vec<String>,
    #[serde(default)]
    to: Vec<String>,
    #[serde(default)]
    modules: Vec<String>,
    #[serde(default)]
    dependency_kinds: Vec<DependencyKindDto>,
}

impl BoundaryRuleDto {
    fn into_domain(self) -> BoundaryRule {
        BoundaryRule {
            id: self.id,
            kind: self.kind.into_domain(),
            severity: self.severity.into_domain(),
            from: self.from,
            to: self.to,
            modules: self.modules,
            dependency_kinds: self
                .dependency_kinds
                .into_iter()
                .map(DependencyKindDto::into_domain)
                .collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum RuleKindDto {
    Forbidden,
    Allowed,
    Circular,
}

impl RuleKindDto {
    fn into_domain(self) -> RuleKind {
        match self {
            Self::Forbidden => RuleKind::Forbidden,
            Self::Allowed => RuleKind::Allowed,
            Self::Circular => RuleKind::Circular,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum SeverityDto {
    Error,
    Warning,
}

impl SeverityDto {
    fn into_domain(self) -> Severity {
        match self {
            Self::Error => Severity::Error,
            Self::Warning => Severity::Warning,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum DependencyKindDto {
    Import,
    ReExport,
    PathReference,
}

impl DependencyKindDto {
    fn into_domain(self) -> DependencyKind {
        match self {
            Self::Import => DependencyKind::Import,
            Self::ReExport => DependencyKind::ReExport,
            Self::PathReference => DependencyKind::PathReference,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::load;

    #[test]
    fn should_return_an_error_for_a_missing_policy() {
        // Arrange
        let path = std::path::Path::new("policy-that-does-not-exist.yaml");

        // Act
        let result = load(path);

        // Assert
        assert!(result.is_err());
    }
}
