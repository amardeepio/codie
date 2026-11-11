#![no_main]

use libfuzzer_sys::fuzz_target;

// Feed arbitrary bytes at arbitrary boundaries. The parser must not panic; a
// discarded event is acceptable for malformed input.
fuzz_target!(|data: &[u8]| {
    let mut parser = codie_ai::SseParser::new();
    for chunk in data.chunks(7) {
        let _ = parser.push(chunk);
    }
    let _ = parser.finish();
});
