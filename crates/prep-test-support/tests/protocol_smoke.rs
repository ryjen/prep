use prep_cli::{CliError, run_from};
use prep_core::PluginProbeError;

#[test]
fn cli_core_external_plugin_smoke_path_succeeds() {
    let plugin = env!("CARGO_BIN_EXE_prep-synthetic-plugin").to_owned();
    let output = run_from(vec![
        "prep".to_owned(),
        "internal".to_owned(),
        "probe-plugin".to_owned(),
        plugin,
    ])
    .expect("synthetic plugin probe should succeed");

    assert!(output.contains("plugin=synthetic"));
    assert!(output.contains("supported=true"));
}

#[test]
fn malformed_external_plugin_output_is_typed_failure() {
    let plugin = env!("CARGO_BIN_EXE_prep-invalid-plugin").to_owned();
    let error = run_from(vec![
        "prep".to_owned(),
        "internal".to_owned(),
        "probe-plugin".to_owned(),
        plugin,
    ])
    .expect_err("invalid plugin output should fail");

    assert!(matches!(
        error,
        CliError::Probe(PluginProbeError::Protocol(_))
    ));
}

#[test]
fn unsuccessful_plugin_process_is_typed_failure() {
    let plugin = env!("CARGO_BIN_EXE_prep-crash-plugin").to_owned();
    let error = run_from(vec![
        "prep".to_owned(),
        "internal".to_owned(),
        "probe-plugin".to_owned(),
        plugin,
    ])
    .expect_err("non-zero plugin exit should fail");

    assert!(matches!(
        error,
        CliError::Probe(PluginProbeError::ProcessFailed { code: Some(17), .. })
    ));
}
