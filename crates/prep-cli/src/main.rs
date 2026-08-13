use std::process::ExitCode;

fn main() -> ExitCode {
    match prep_cli::run_from(std::env::args()) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("prep: {error}");
            ExitCode::FAILURE
        }
    }
}
