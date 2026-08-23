use std::process::ExitCode;

mod boundary_evaluation;
mod boundary_policy_input;
mod boundary_validation;
mod command_interface;
mod rust_dependency_observation;

fn main() -> ExitCode {
    command_interface::run(
        std::env::args_os().skip(1),
        &rust_dependency_observation::RustDependencyObserver,
        &boundary_policy_input::YamlBoundaryPolicyLoader,
    )
}
