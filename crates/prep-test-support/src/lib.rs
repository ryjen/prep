//! Test fixtures and synthetic plugin binaries used by Prep integration tests.

use prep_protocol::{
    DEFAULT_MAX_FRAME_BYTES, HelloFrame, HelloResultFrame, PROTOCOL_V1, PluginCapability,
    PluginDescriptor, PluginOperation, ProbeBuildSystemRequest, ProbeBuildSystemResult,
    ResultFrame, decode_frame, encode_frame,
};
use std::error::Error;
use std::io::{self, BufRead, Write};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

const FIXTURE_HANG: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdversarialPluginMode {
    HangHandshake,
    HangResult,
    HangExit,
    FloodStderr,
    SpawnChild,
    SpawnChildExit,
    DeclareNetwork,
}

pub fn run_adversarial_plugin(mode: AdversarialPluginMode) -> Result<(), Box<dyn Error>> {
    if mode == AdversarialPluginMode::HangHandshake {
        thread::sleep(FIXTURE_HANG);
        return Ok(());
    }

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    let hello_line = next_line(&mut lines)?;
    let hello: HelloFrame = decode_frame(&hello_line, DEFAULT_MAX_FRAME_BYTES)?;
    if hello.protocol != PROTOCOL_V1 || hello.frame_type != "hello" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid hello frame").into());
    }

    let hello_result = HelloResultFrame {
        protocol: PROTOCOL_V1.to_owned(),
        id: hello.id,
        frame_type: "hello_result".to_owned(),
        plugin: PluginDescriptor {
            name: fixture_name(mode).to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            operations: vec![PluginOperation::ProbeBuildSystem],
            capabilities: if mode == AdversarialPluginMode::DeclareNetwork {
                vec![PluginCapability::Network]
            } else {
                Vec::new()
            },
        },
    };
    writeln!(stdout, "{}", encode_frame(&hello_result)?)?;
    stdout.flush()?;

    let probe_line = next_line(&mut lines)?;
    let probe: ProbeBuildSystemRequest = decode_frame(&probe_line, DEFAULT_MAX_FRAME_BYTES)?;
    if probe.protocol != PROTOCOL_V1 || probe.frame_type != PluginOperation::ProbeBuildSystem {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid probe frame").into());
    }

    match mode {
        AdversarialPluginMode::HangHandshake => unreachable!("handled before protocol input"),
        AdversarialPluginMode::HangResult => {
            thread::sleep(FIXTURE_HANG);
            Ok(())
        }
        AdversarialPluginMode::SpawnChild => {
            let child = spawn_sleep_child()?;
            report_child_pid(&child)?;
            thread::sleep(FIXTURE_HANG);
            Ok(())
        }
        AdversarialPluginMode::SpawnChildExit => {
            let child = spawn_sleep_child()?;
            report_child_pid(&child)?;
            write_success_result(&mut stdout, probe)
        }
        AdversarialPluginMode::FloodStderr => {
            let chunk = vec![b'x'; 4096];
            let mut stderr = io::stderr().lock();
            for _ in 0..64 {
                stderr.write_all(&chunk)?;
            }
            writeln!(stderr, "flood-complete")?;
            stderr.flush()?;
            write_success_result(&mut stdout, probe)
        }
        AdversarialPluginMode::HangExit => {
            write_success_result(&mut stdout, probe)?;
            thread::sleep(FIXTURE_HANG);
            Ok(())
        }
        AdversarialPluginMode::DeclareNetwork => write_success_result(&mut stdout, probe),
    }
}

fn spawn_sleep_child() -> Result<Child, io::Error> {
    Command::new("/bin/sleep")
        .arg("60")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

fn report_child_pid(child: &Child) -> Result<(), io::Error> {
    eprintln!("child_pid={}", child.id());
    io::stderr().flush()
}

fn write_success_result(
    stdout: &mut impl Write,
    probe: ProbeBuildSystemRequest,
) -> Result<(), Box<dyn Error>> {
    let result = ResultFrame::ok(probe.id, ProbeBuildSystemResult { supported: true });
    writeln!(stdout, "{}", encode_frame(&result)?)?;
    stdout.flush()?;
    Ok(())
}

fn fixture_name(mode: AdversarialPluginMode) -> &'static str {
    match mode {
        AdversarialPluginMode::HangHandshake => "hang-handshake",
        AdversarialPluginMode::HangResult => "hang-result",
        AdversarialPluginMode::HangExit => "hang-exit",
        AdversarialPluginMode::FloodStderr => "flood-stderr",
        AdversarialPluginMode::SpawnChild => "spawn-child",
        AdversarialPluginMode::SpawnChildExit => "spawn-child-exit",
        AdversarialPluginMode::DeclareNetwork => "declare-network",
    }
}

fn next_line(
    lines: &mut impl Iterator<Item = io::Result<String>>,
) -> Result<String, Box<dyn Error>> {
    match lines.next() {
        Some(line) => Ok(line?),
        None => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing protocol frame").into()),
    }
}
