//! Scatters arbitrary bytes as a 4-page block stream and verifies it: must never panic.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = rustern_client::blocks::page_stream(data);
});
