#![no_main]

use libfuzzer_sys::fuzz_target;
use prep_manifest::Lockfile;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(lockfile) = Lockfile::parse(text) {
            let _ = lockfile.to_toml();
        }
    }
});
