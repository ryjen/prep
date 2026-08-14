use prep_core::{
    PluginPhase, PluginProbeError, PluginProcessPolicy, probe_build_system_with_policy,
};
use prep_protocol::DEFAULT_MAX_FRAME_BYTES;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

const OPERATION_TIMEOUT: Duration = Duration::from_millis(500);
const TERMINATION_GRACE: Duration = Duration::from_millis(150);
const TEST_DEADLINE: Duration = Duration::from_secs(3);
const DIAGNOSTIC_LIMIT: usize = 1024;

fn test_policy() -> PluginProcessPolicy {
    PluginProcessPolicy {
        operation_timeout: OPERATION_TIMEOUT,
        termination_grace: TERMINATION_GRACE,
        max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
        max_diagnostic_bytes: DIAGNOSTIC_LIMIT,
    }
}

#[test]
fn handshake_hang_times_out_within_process_policy() {
    assert_timeout(
        Path::new(env!("CARGO_BIN_EXE_prep-hang-handshake-plugin")),
        PluginPhase::Handshake,
    );
}

#[test]
fn result_hang_times_out_within_process_policy() {
    assert_timeout(
        Path::new(env!("CARGO_BIN_EXE_prep-hang-result-plugin")),
        PluginPhase::Result,
    );
}

#[test]
fn successful_result_cannot_hide_a_process_that_refuses_to_exit() {
    assert_timeout(
        Path::new(env!("CARGO_BIN_EXE_prep-hang-exit-plugin")),
        PluginPhase::Exit,
    );
}

#[test]
fn stderr_flood_is_drained_but_diagnostic_memory_is_bounded() {
    let plugin = Path::new(env!("CARGO_BIN_EXE_prep-flood-stderr-plugin"));
    let started = Instant::now();
    let probe = probe_build_system_with_policy(plugin, test_policy())
        .expect("stderr flood must not deadlock the valid plugin operation");

    assert!(probe.supported);
    assert!(probe.diagnostics.truncated);
    assert_eq!(probe.diagnostics.text.len(), DIAGNOSTIC_LIMIT);
    assert!(started.elapsed() < TEST_DEADLINE);
}

#[cfg(unix)]
#[test]
fn timeout_cleans_up_an_ordinary_spawned_child_process() {
    let plugin = Path::new(env!("CARGO_BIN_EXE_prep-spawn-child-plugin"));
    let started = Instant::now();
    let error = probe_build_system_with_policy(plugin, test_policy())
        .expect_err("spawned-child fixture should time out");

    let diagnostics = match error {
        PluginProbeError::Timeout {
            phase: PluginPhase::Result,
            diagnostics,
            ..
        } => diagnostics,
        other => panic!("expected result timeout, got {other:?}"),
    };
    let child_pid = parse_child_pid(&diagnostics.text).expect("fixture must report child pid");

    let cleanup_deadline = Instant::now() + Duration::from_secs(1);
    while process_is_running(child_pid) && Instant::now() < cleanup_deadline {
        thread::sleep(Duration::from_millis(25));
    }

    assert!(
        !process_is_running(child_pid),
        "managed plugin child {child_pid} remained running after timeout cleanup"
    );
    assert!(started.elapsed() < TEST_DEADLINE);
}

fn assert_timeout(plugin: &Path, expected_phase: PluginPhase) {
    let started = Instant::now();
    let error = probe_build_system_with_policy(plugin, test_policy())
        .expect_err("fixture should time out");

    assert!(matches!(
        error,
        PluginProbeError::Timeout { phase, .. } if phase == expected_phase
    ));
    assert!(started.elapsed() < TEST_DEADLINE);
}

#[cfg(unix)]
fn parse_child_pid(diagnostics: &str) -> Option<u32> {
    diagnostics.lines().find_map(|line| {
        line.strip_prefix("child_pid=")
            .and_then(|value| value.trim().parse().ok())
    })
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    let output = Command::new("/bin/ps")
        .args(["-o", "state=", "-p", &pid.to_string()])
        .output()
        .expect("inspect fixture child process");
    if !output.status.success() {
        return false;
    }

    let state = String::from_utf8_lossy(&output.stdout);
    let state = state.trim();
    !state.is_empty() && !state.starts_with('Z')
}
