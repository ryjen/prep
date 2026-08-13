#![no_main]

use libfuzzer_sys::fuzz_target;
use prep_manifest::Manifest;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(manifest) = Manifest::parse(text) {
            let _ = manifest.to_toml();
        }
    }
});
