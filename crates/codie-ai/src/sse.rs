//! Incremental, byte-oriented Server-Sent Events parsing.
//!
//! Chunks arrive at arbitrary boundaries, including in the middle of a UTF-8
//! sequence or a line. [`SseParser`] buffers bytes until a complete line is
//! available and only decodes complete lines, so reassembly is exact
//! (`PROV-012`).

use bytes::Bytes;

/// One dispatched SSE event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` field, when the server supplied one.
    pub event: Option<String>,
    /// The `data:` payload with continuation lines joined by a newline.
    pub data: String,
}

impl SseEvent {
    /// Returns true when this event is the OpenAI stream terminator.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.data.trim() == "[DONE]"
    }
}

/// A stateful parser for the `text/event-stream` wire format.
///
/// Feed it every transport chunk with [`SseParser::push`]. Comment lines,
/// keepalive frames, and non-`data` fields are ignored. An event is emitted
/// only after its terminating blank line, or by [`SseParser::finish`] when the
/// peer closes the connection without a final blank line.
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    data: Vec<String>,
    event: Option<String>,
}

impl SseParser {
    /// Creates an empty parser.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one transport chunk and returns every event it completed.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();

        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=position).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.process_line(&line, &mut events);
        }

        events
    }

    /// Feeds a `bytes::Bytes` chunk.
    pub fn push_bytes(&mut self, chunk: Bytes) -> Vec<SseEvent> {
        self.push(&chunk)
    }

    /// Flushes a trailing partial line and dispatches any pending event.
    ///
    /// This tolerates servers that close without the final blank line.
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let mut line = std::mem::take(&mut self.buffer);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.process_line(&line, &mut events);
        }
        self.dispatch(&mut events);
        events
    }

    /// Returns the number of bytes buffered for an incomplete line.
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    fn process_line(&mut self, line: &[u8], events: &mut Vec<SseEvent>) {
        if line.is_empty() {
            self.dispatch(events);
            return;
        }
        if line.first() == Some(&b':') {
            return;
        }

        let (field, value) = match line.iter().position(|byte| *byte == b':') {
            Some(position) => (&line[..position], &line[position + 1..]),
            None => (line, &[][..]),
        };
        let value = value.strip_prefix(b" ").unwrap_or(value);
        let text = String::from_utf8_lossy(value).into_owned();

        match field {
            b"event" => self.event = Some(text),
            b"data" => self.data.push(text),
            _ => {}
        }
    }

    fn dispatch(&mut self, events: &mut Vec<SseEvent>) {
        let event = self.event.take();
        if self.data.is_empty() {
            return;
        }
        let data = std::mem::take(&mut self.data).join("\n");
        if data.is_empty() {
            return;
        }
        events.push(SseEvent { event, data });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_frame() {
        let mut parser = SseParser::new();
        let events = parser.push(b"data: {\"a\":1}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "{\"a\":1}");
        assert_eq!(events[0].event, None);
    }

    #[test]
    fn ignores_comments_and_keepalives() {
        let mut parser = SseParser::new();
        let events = parser.push(b": ping\n\ndata: 1\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "1");
    }

    #[test]
    fn joins_multiline_data() {
        let mut parser = SseParser::new();
        let events = parser.push(b"data: one\ndata: two\n\n");
        assert_eq!(events[0].data, "one\ntwo");
    }

    #[test]
    fn tolerates_crlf() {
        let mut parser = SseParser::new();
        let events = parser.push(b"data: hi\r\n\r\n");
        assert_eq!(events[0].data, "hi");
    }

    #[test]
    fn reassembles_at_every_byte_boundary() {
        let body = "event: message\ndata: h\u{e9}llo \u{4e16}\u{754c} \u{1f980}\n\n: ping\n\ndata: [DONE]\n\n";
        let expected = vec![
            SseEvent {
                event: Some("message".to_owned()),
                data: "h\u{e9}llo \u{4e16}\u{754c} \u{1f980}".to_owned(),
            },
            SseEvent {
                event: None,
                data: "[DONE]".to_owned(),
            },
        ];

        for size in 1..=body.len() {
            let mut parser = SseParser::new();
            let mut events = Vec::new();
            for chunk in body.as_bytes().chunks(size) {
                events.extend(parser.push(chunk));
            }
            events.extend(parser.finish());
            assert_eq!(events, expected, "failed with chunk size {size}");
        }
    }

    #[test]
    fn finish_dispatches_an_unterminated_event() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"data: tail").is_empty());
        let events = parser.finish();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "tail");
    }
}
