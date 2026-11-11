//! Robustness tests for the byte-oriented SSE parser (`PROV-012`, P1-T02).
//!
//! The parser must never panic on malformed input, arbitrary binary data, or
//! chunk boundaries inside UTF-8 sequences. This test is deterministic and runs
//! in CI; `fuzz/` contains the libFuzzer harness for longer campaigns.

use codie_ai::SseParser;
use codie_testkit::spec;

/// A small deterministic PRNG so the test has no external fixture.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u8 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u8
    }
}

#[test]
#[spec("PROV-012")]
fn spec_prov_012_sse_parser_survives_arbitrary_bytes() {
    let mut rng = Lcg(0x1234_5678_9abc_def0);

    for case in 0..2_000 {
        let length = usize::from(rng.next()) + 1;
        let mut bytes = vec![0_u8; length];
        for byte in &mut bytes {
            *byte = rng.next();
        }

        let mut parser = SseParser::new();
        let mut offset = 0;
        while offset < bytes.len() {
            let size = usize::from(rng.next() % 13) + 1;
            let end = (offset + size).min(bytes.len());
            let _ = parser.push(&bytes[offset..end]);
            offset = end;
        }
        let _ = parser.finish();

        // A complete frame embedded in a random prefix/suffix must still parse.
        if case % 50 == 0 {
            let mut framed = b"garbage\n\n: keepalive\n\ndata: hello\n\n".to_vec();
            framed.extend_from_slice(&bytes);
            let mut parser = SseParser::new();
            let events = parser.push(&framed);
            assert!(events.iter().any(|event| event.data == "hello"));
        }
    }
}

#[test]
#[spec("PROV-012")]
fn spec_prov_012_sse_parser_handles_every_split_of_valid_frames() {
    let frame = "event: message\r\ndata: {\"text\":\"héllo—世界 🦀\"}\r\n\r\n";
    let bytes = frame.as_bytes();

    for split in 0..=bytes.len() {
        let mut parser = SseParser::new();
        let mut events = parser.push(&bytes[..split]);
        events.extend(parser.push(&bytes[split..]));
        events.extend(parser.finish());

        assert_eq!(events.len(), 1, "split {split} lost the event");
        assert_eq!(events[0].event.as_deref(), Some("message"));
        assert_eq!(events[0].data, "{\"text\":\"héllo—世界 🦀\"}");
    }
}
