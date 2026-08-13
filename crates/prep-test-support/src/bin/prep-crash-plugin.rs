use prep_protocol::{
    DEFAULT_MAX_FRAME_BYTES, HelloFrame, HelloResultFrame, PROTOCOL_V1, PluginDescriptor,
    ProbeBuildSystemRequest, ResultFrame, decode_frame, encode_frame,
};
use serde_json::json;
use std::error::Error;
use std::io::{self, BufRead, Write};

fn main() -> Result<(), Box<dyn Error>> {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    let hello_line = next_line(&mut lines)?;
    let hello: HelloFrame = decode_frame(&hello_line, DEFAULT_MAX_FRAME_BYTES)?;
    let hello_result = HelloResultFrame {
        protocol: PROTOCOL_V1.to_owned(),
        id: hello.id,
        frame_type: "hello_result".to_owned(),
        plugin: PluginDescriptor {
            name: "crash-after-result".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            operations: vec!["probe_build_system".to_owned()],
            capabilities: Vec::new(),
        },
    };
    writeln!(stdout, "{}", encode_frame(&hello_result)?)?;
    stdout.flush()?;

    let probe_line = next_line(&mut lines)?;
    let probe: ProbeBuildSystemRequest = decode_frame(&probe_line, DEFAULT_MAX_FRAME_BYTES)?;
    let result = ResultFrame {
        protocol: PROTOCOL_V1.to_owned(),
        id: probe.id,
        frame_type: "result".to_owned(),
        status: "ok".to_owned(),
        value: Some(json!({"supported": true})),
        error: None,
    };
    writeln!(stdout, "{}", encode_frame(&result)?)?;
    stdout.flush()?;

    std::process::exit(17);
}

fn next_line(
    lines: &mut impl Iterator<Item = io::Result<String>>,
) -> Result<String, Box<dyn Error>> {
    match lines.next() {
        Some(line) => Ok(line?),
        None => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing protocol frame").into()),
    }
}
