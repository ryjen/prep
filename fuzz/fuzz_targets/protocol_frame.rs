#![no_main]

use libfuzzer_sys::fuzz_target;
use prep_protocol::{DEFAULT_MAX_FRAME_BYTES, ResultFrame, decode_frame};

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = decode_frame::<ResultFrame>(text, DEFAULT_MAX_FRAME_BYTES);
    }
});
