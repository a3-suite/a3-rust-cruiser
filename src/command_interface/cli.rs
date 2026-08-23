use std::{ffi::OsString, io::Write, path::PathBuf, process::ExitCode};

use crate::boundary_validation::{
    self, AnalysisContext, AnalysisProfile, BoundaryPolicyLoader, CruiserError, DependencyObserver,
    Result, ValidationDecision,
};

use super::report;

#[derive(Debug)]
struct CheckArgs {
    project_root: PathBuf,
    policy: PathBuf,
    package: String,
    target: String,
    target_triple: String,
    analysis_profile: AnalysisProfile,
    features: Vec<String>,
    cfg: Vec<String>,
    include: Vec<String>,
    exclude: Vec<String>,
    format: OutputFormat,
}

#[derive(Debug, Clone, Copy)]
enum OutputFormat {
    Text,
    Json,
}

impl OutputFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
        }
    }
}

pub fn run<I, O, L>(args: I, observer: &O, policy_loader: &L) -> ExitCode
where
    I: IntoIterator<Item = OsString>,
    O: DependencyObserver,
    L: BoundaryPolicyLoader,
{
    match execute(args, observer, policy_loader) {
        Ok((payload, exit_code)) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(error) = writeln!(stdout, "{payload}") {
                eprintln!("failed to write validation result: {error}");
                return ExitCode::from(3);
            }
            ExitCode::from(exit_code)
        }
        Err(error) => {
            eprintln!("a3-rust-cruiser: {error}");
            ExitCode::from(3)
        }
    }
}

fn execute<I, O, L>(args: I, observer: &O, policy_loader: &L) -> Result<(String, u8)>
where
    I: IntoIterator<Item = OsString>,
    O: DependencyObserver,
    L: BoundaryPolicyLoader,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let check = parse_check_args(&args)?;
    let context = AnalysisContext {
        analysis_profile: check.analysis_profile,
        package: check.package,
        compilation_target: check.target,
        features: sorted_unique(check.features),
        cfg: sorted_unique(check.cfg),
        target_triple: check.target_triple,
        include: check.include,
        exclude: check.exclude,
    };
    let result = boundary_validation::validate(
        observer,
        policy_loader,
        &check.project_root,
        context,
        &check.policy,
    )?;
    let report = report::build(result, check.format.as_str());
    let payload = match check.format {
        OutputFormat::Text => report::to_text(&report),
        OutputFormat::Json => report::to_json(&report).map_err(|error| {
            CruiserError::Parse(format!("failed to encode JSON report: {error}"))
        })?,
    };
    Ok((payload, decision_exit_code(report.decision)))
}

fn decision_exit_code(decision: ValidationDecision) -> u8 {
    match decision {
        ValidationDecision::Conformant => 0,
        ValidationDecision::NonConformant => 1,
        ValidationDecision::Indeterminate => 2,
    }
}

fn parse_check_args(args: &[String]) -> Result<CheckArgs> {
    if args.first().map(String::as_str) != Some("check") {
        return Err(CruiserError::InvalidInput(usage().to_owned()));
    }
    let project_root = args
        .get(1)
        .ok_or_else(|| CruiserError::InvalidInput(usage().to_owned()))?;
    let mut policy = None;
    let mut package = None;
    let mut target = None;
    let mut target_triple = None;
    let mut analysis_profile = AnalysisProfile::Compilation;
    let mut features = Vec::new();
    let mut cfg = Vec::new();
    let mut include = Vec::new();
    let mut exclude = Vec::new();
    let mut format = OutputFormat::Text;
    let mut index = 2;
    while index < args.len() {
        let option = args[index].as_str();
        let takes_value = |name: &str, index: &mut usize| -> Result<String> {
            *index += 1;
            args.get(*index)
                .cloned()
                .ok_or_else(|| CruiserError::InvalidInput(format!("missing value for {name}")))
        };
        match option {
            "--policy" | "-p" => policy = Some(PathBuf::from(takes_value(option, &mut index)?)),
            "--package" => package = Some(takes_value(option, &mut index)?),
            "--target" => target = Some(takes_value(option, &mut index)?),
            "--target-triple" => target_triple = Some(takes_value(option, &mut index)?),
            "--analysis-profile" => {
                analysis_profile = match takes_value(option, &mut index)?.as_str() {
                    "compilation" => AnalysisProfile::Compilation,
                    "source-union" => AnalysisProfile::SourceUnion,
                    value => {
                        return Err(CruiserError::InvalidInput(format!(
                            "unsupported analysis profile: {value}"
                        )));
                    }
                };
            }
            "--feature" => features.push(takes_value(option, &mut index)?),
            "--cfg" => cfg.push(takes_value(option, &mut index)?),
            "--include" => include.push(takes_value(option, &mut index)?),
            "--exclude" => exclude.push(takes_value(option, &mut index)?),
            "--format" | "-f" => {
                format = match takes_value(option, &mut index)?.as_str() {
                    "text" => OutputFormat::Text,
                    "json" => OutputFormat::Json,
                    value => {
                        return Err(CruiserError::InvalidInput(format!(
                            "unsupported output format: {value}"
                        )));
                    }
                };
            }
            value => {
                return Err(CruiserError::InvalidInput(format!(
                    "unknown option: {value}"
                )));
            }
        }
        index += 1;
    }
    Ok(CheckArgs {
        project_root: PathBuf::from(project_root),
        policy: policy
            .ok_or_else(|| CruiserError::InvalidInput("--policy is required".to_owned()))?,
        package: package
            .ok_or_else(|| CruiserError::InvalidInput("--package is required".to_owned()))?,
        target: target
            .ok_or_else(|| CruiserError::InvalidInput("--target is required".to_owned()))?,
        target_triple: target_triple
            .ok_or_else(|| CruiserError::InvalidInput("--target-triple is required".to_owned()))?,
        analysis_profile,
        features,
        cfg,
        include,
        exclude,
        format,
    })
}

fn sorted_unique(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

fn usage() -> &'static str {
    "usage: a3-rust-cruiser check <PROJECT_ROOT> --policy <BOUNDARY_POLICY_PATH> --package <PACKAGE> --target <TARGET> --target-triple <TARGET_TRIPLE> [--analysis-profile <compilation|source-union>] [--feature <FEATURE>]... [--cfg <CFG>]... [--include <MODULE_PATTERN>]... [--exclude <MODULE_PATTERN>]... [--format <text|json>]"
}

#[cfg(test)]
mod tests {
    use super::{decision_exit_code, parse_check_args};
    use crate::boundary_validation::{AnalysisProfile, ValidationDecision};

    #[test]
    fn check_requires_a_project_root() {
        // Arrange
        let arguments = vec!["check".to_owned()];

        // Act
        let result = parse_check_args(&arguments);

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn decision_exit_codes_are_stable() {
        // Arrange
        let decisions = [
            (ValidationDecision::Conformant, 0),
            (ValidationDecision::NonConformant, 1),
            (ValidationDecision::Indeterminate, 2),
        ];

        // Act
        let exit_codes: Vec<u8> = decisions
            .iter()
            .map(|(decision, _)| decision_exit_code(*decision))
            .collect();

        // Assert
        assert_eq!(
            exit_codes,
            decisions.iter().map(|(_, code)| *code).collect::<Vec<_>>()
        );
    }

    #[test]
    fn should_default_analysis_profile_to_compilation() {
        // Arrange
        let arguments = vec![
            "check",
            ".",
            "--policy",
            "policy.yaml",
            "--package",
            "fixture",
            "--target",
            "bin:fixture",
            "--target-triple",
            "x86_64-unknown-linux-gnu",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();

        // Act
        let result = parse_check_args(&arguments).expect("arguments should be valid");

        // Assert
        assert_eq!(result.analysis_profile, AnalysisProfile::Compilation);
    }

    #[test]
    fn should_reject_an_unsupported_analysis_profile() {
        // Arrange
        let arguments = vec![
            "check",
            ".",
            "--policy",
            "policy.yaml",
            "--package",
            "fixture",
            "--target",
            "bin:fixture",
            "--target-triple",
            "x86_64-unknown-linux-gnu",
            "--analysis-profile",
            "unknown",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();

        // Act
        let result = parse_check_args(&arguments);

        // Assert
        assert!(result.is_err());
    }
}
