use std::{ffi::OsString, io::Write, path::PathBuf, process::ExitCode};

use crate::boundary_validation::{
    self, AnalysisContext, AnalysisProfile, BoundaryPolicyInput, BoundaryPolicyLoader,
    CruiserError, DependencyObserver, Result, ValidationDecision,
};

use super::report;

#[derive(Debug)]
struct CheckArgs {
    project_root: PathBuf,
    policy_input: BoundaryPolicyInput,
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
        Ok((payload, exit_code)) => write_result(std::io::stdout().lock(), &payload, exit_code),
        Err(error) => {
            eprintln!("a3-rust-cruiser: {error}");
            ExitCode::from(3)
        }
    }
}

fn write_result<W: Write>(mut stdout: W, payload: &str, exit_code: u8) -> ExitCode {
    if let Err(error) = writeln!(stdout, "{payload}").and_then(|_| stdout.flush()) {
        eprintln!("failed to write validation result: {error}");
        return ExitCode::from(3);
    }
    ExitCode::from(exit_code)
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
    if let Some(payload) = informational_output(&args) {
        return Ok((payload, 0));
    }
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
        &check.policy_input,
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

fn informational_output(args: &[String]) -> Option<String> {
    match args {
        [option] if option == "--help" || option == "-h" => Some(root_help().to_owned()),
        [option] if option == "--version" || option == "-V" => {
            Some(format!("a3-rust-cruiser {}", env!("CARGO_PKG_VERSION")))
        }
        [command, option] if command == "check" && (option == "--help" || option == "-h") => {
            Some(check_help().to_owned())
        }
        _ => None,
    }
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
    let mut resolved_implementation_attribution = None;
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
            "--resolved-implementation-attribution" => {
                resolved_implementation_attribution =
                    Some(PathBuf::from(takes_value(option, &mut index)?));
            }
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
        policy_input: parse_policy_input(policy, resolved_implementation_attribution)?,
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

fn parse_policy_input(
    policy: Option<PathBuf>,
    resolved_implementation_attribution: Option<PathBuf>,
) -> Result<BoundaryPolicyInput> {
    let source_count =
        usize::from(policy.is_some()) + usize::from(resolved_implementation_attribution.is_some());
    if source_count > 1 {
        return Err(CruiserError::InvalidInput(
            "boundary policy inputs are mutually exclusive: use exactly one of --policy or --resolved-implementation-attribution"
                .to_owned(),
        ));
    }
    if let Some(path) = policy {
        return Ok(BoundaryPolicyInput::Yaml { path });
    }
    if let Some(path) = resolved_implementation_attribution {
        return Ok(BoundaryPolicyInput::ResolvedImplementationAttribution { path });
    }
    Err(CruiserError::InvalidInput(
        "--policy or --resolved-implementation-attribution is required".to_owned(),
    ))
}

fn sorted_unique(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

fn usage() -> &'static str {
    "Usage: a3-rust-cruiser check <PROJECT_ROOT> (--policy <BOUNDARY_POLICY_PATH> | --resolved-implementation-attribution <RESOLVED_IMPLEMENTATION_ATTRIBUTION_PATH>) --package <PACKAGE> --target <TARGET> --target-triple <TARGET_TRIPLE> [--analysis-profile <compilation|source-union>] [--feature <FEATURE>]... [--cfg <CFG>]... [--include <MODULE_PATTERN>]... [--exclude <MODULE_PATTERN>]... [--format <text|json>]"
}

fn root_help() -> &'static str {
    "Usage: a3-rust-cruiser <COMMAND>\n\nCommands:\n  check  Validate logical module boundaries for one Cargo package and compilation target\n\nOptions:\n  -h, --help     Print help\n  -V, --version  Print version"
}

fn check_help() -> &'static str {
    "Usage: a3-rust-cruiser check <PROJECT_ROOT> (--policy <BOUNDARY_POLICY_PATH> | --resolved-implementation-attribution <RESOLVED_IMPLEMENTATION_ATTRIBUTION_PATH>) --package <PACKAGE> --target <TARGET> --target-triple <TARGET_TRIPLE> [--analysis-profile <compilation|source-union>] [--feature <FEATURE>]... [--cfg <CFG>]... [--include <MODULE_PATTERN>]... [--exclude <MODULE_PATTERN>]... [--format <text|json>]\n\nAnalysis input:\n  -p, --policy <BOUNDARY_POLICY_PATH>\n      --resolved-implementation-attribution <RESOLVED_IMPLEMENTATION_ATTRIBUTION_PATH>\n      --package <PACKAGE>\n      --target <TARGET>\n      --target-triple <TARGET_TRIPLE>\n      --analysis-profile <compilation|source-union>\n      --feature <FEATURE>\n      --cfg <CFG>\n      --include <MODULE_PATTERN>\n      --exclude <MODULE_PATTERN>\n\nOutput:\n  -f, --format <text|json>\n\nOptions:\n  -h, --help  Print help"
}

#[cfg(test)]
mod tests {
    use super::{decision_exit_code, informational_output, parse_check_args};
    use crate::boundary_validation::{AnalysisProfile, BoundaryPolicyInput, ValidationDecision};

    #[test]
    fn should_require_a_project_root_for_check() {
        // Arrange
        let arguments = vec!["check".to_owned()];

        // Act
        let result = parse_check_args(&arguments);

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn should_keep_decision_exit_codes_stable() {
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

    #[test]
    fn should_reject_multiple_policy_sources() {
        // Arrange
        let arguments = vec![
            "check",
            ".",
            "--policy",
            "policy.yaml",
            "--resolved-implementation-attribution",
            "resolved.sdd.yml",
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
        let result = parse_check_args(&arguments);

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn should_accept_resolved_implementation_attribution_as_the_policy_source() {
        // Arrange
        let arguments = vec![
            "check",
            ".",
            "--resolved-implementation-attribution",
            "resolved-implementation-attribution.sdd.yml",
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
        assert!(matches!(
            result.policy_input,
            BoundaryPolicyInput::ResolvedImplementationAttribution { path }
                if path.as_path()
                    == std::path::Path::new("resolved-implementation-attribution.sdd.yml")
        ));
    }

    #[test]
    fn should_reject_resolved_attribution_combined_with_another_policy_source() {
        // Arrange
        let cases = [vec![
            "--resolved-implementation-attribution",
            "resolved.sdd.yml",
            "--policy",
            "policy.yaml",
        ]];

        // Act
        let results = cases.map(|policy_args| {
            let mut arguments = vec!["check", "."];
            arguments.extend(policy_args);
            arguments.extend([
                "--package",
                "fixture",
                "--target",
                "bin:fixture",
                "--target-triple",
                "x86_64-unknown-linux-gnu",
            ]);
            parse_check_args(&arguments.into_iter().map(str::to_owned).collect::<Vec<_>>())
        });

        // Assert
        assert!(results.into_iter().all(|result| result.is_err()));
    }

    #[test]
    fn should_reject_retired_sdd_options_as_unknown() {
        // Arrange
        let retired_options = [
            "--logical-structure",
            "--projection",
            "--directory-structure",
        ];

        // Act
        let results = retired_options.map(|option| {
            let arguments = vec![
                "check".to_owned(),
                ".".to_owned(),
                option.to_owned(),
                "retired.sdd.yml".to_owned(),
            ];
            parse_check_args(&arguments)
        });

        // Assert
        assert!(results.into_iter().all(|result| matches!(
            result,
            Err(crate::boundary_validation::CruiserError::InvalidInput(message))
                if message == "unknown option: --logical-structure"
                    || message == "unknown option: --projection"
                    || message == "unknown option: --directory-structure"
        )));
    }

    #[test]
    fn should_support_informational_aliases() {
        // Arrange
        let arguments = [vec!["-h".to_owned()], vec!["-V".to_owned()]];

        // Act
        let outputs = arguments.map(|arguments| informational_output(&arguments));

        // Assert
        assert!(outputs.into_iter().all(|output| output.is_some()));
    }
}
