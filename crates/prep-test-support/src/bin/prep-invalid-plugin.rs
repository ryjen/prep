use std::io::{self, BufRead, Write};

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let _ = lines.next().transpose()?;

    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    writeln!(stdout, "not-json")?;
    stdout.flush()
}
