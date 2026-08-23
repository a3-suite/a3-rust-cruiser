use std::process::ExitCode;

fn main() -> ExitCode {
    a3_rust_cruiser::command_interface::run(std::env::args_os().skip(1))
}
