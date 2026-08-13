use prep_core::{PluginProbeError, probe_build_system};
use std::error::Error;
use std::fmt;
use std::path::Path;

const HELP: &str =
    "Prep 2\n\nUsage:\n  prep --version\n  prep help\n  prep internal probe-plugin <executable>\n";

#[derive(Debug)]
pub enum CliError {
    Usage(String),
    Probe(PluginProbeError),
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => write!(formatter, "{message}"),
            Self::Probe(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Usage(_) => None,
            Self::Probe(error) => Some(error),
        }
    }
}

impl From<PluginProbeError> for CliError {
    fn from(error: PluginProbeError) -> Self {
        Self::Probe(error)
    }
}

pub fn run_from<I>(args: I) -> Result<String, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let _program = args.next();

    match args.next().as_deref() {
        None | Some("help" | "--help" | "-h") => Ok(HELP.to_owned()),
        Some("--version" | "-V") => Ok(format!("prep {}", env!("CARGO_PKG_VERSION"))),
        Some("internal") => run_internal(args),
        Some(command) => Err(CliError::Usage(format!(
            "unknown command {command:?}\n\n{HELP}"
        ))),
    }
}

fn run_internal(mut args: impl Iterator<Item = String>) -> Result<String, CliError> {
    match args.next().as_deref() {
        Some("probe-plugin") => {
            let executable = args.next().ok_or_else(|| {
                CliError::Usage("internal probe-plugin requires an executable path".to_owned())
            })?;
            if args.next().is_some() {
                return Err(CliError::Usage(
                    "internal probe-plugin accepts exactly one executable path".to_owned(),
                ));
            }
            let probe = probe_build_system(Path::new(&executable))?;
            Ok(format!(
                "plugin={} version={} supported={}",
                probe.name, probe.version, probe.supported
            ))
        }
        Some(command) => Err(CliError::Usage(format!(
            "unknown internal command {command:?}"
        ))),
        None => Err(CliError::Usage("missing internal command".to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_available_without_runtime_dependencies() {
        let output = run_from(["prep".to_owned(), "--version".to_owned()])
            .expect("version command should succeed");
        assert!(output.starts_with("prep "));
    }

    #[test]
    fn unknown_command_is_a_usage_error() {
        let error = run_from(["prep".to_owned(), "unknown".to_owned()])
            .expect_err("unknown command should fail");
        assert!(matches!(error, CliError::Usage(_)));
    }
}
