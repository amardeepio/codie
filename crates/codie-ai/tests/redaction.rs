//! Trace redaction and fixture-recording gate (`PROV-005`).

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use codie_ai::{OpenAiProvider, redact_header, redact_text, redacted_headers};
use codie_core::agent::ChatClient;
use codie_core::content::ThinkingLevel;
use codie_core::message::Message;
use codie_testkit::spec;

const KEY: &str = "sk-secret-123";

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("buffer lock")).into_owned()
    }
}

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("buffer lock").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buffer {
    type Writer = Buffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
#[spec("PROV-005")]
#[spec("SEC-005")]
async fn spec_prov_005_trace_redaction_and_recording() {
    // The recording gate is false unless both the feature and the environment
    // opt-in are present.
    let expected =
        cfg!(feature = "fixtures") && std::env::var_os("CODIE_RECORD_FIXTURES").is_some();
    assert_eq!(codie_ai::record::enabled(), expected);

    // Header and free-text redaction.
    let mut headers = codie_ai::HeaderMap::new();
    headers.insert("X-Title".to_owned(), "codie".to_owned());
    headers.insert("X-Custom".to_owned(), format!("prefix {KEY}"));
    let summary = redacted_headers(&headers, KEY);
    assert!(!summary.contains(KEY));
    assert!(summary.contains("authorization: Bearer <redacted>"));
    assert_eq!(redact_header("Authorization", KEY, KEY), "<redacted>");
    assert!(!redact_text(&format!("key={KEY}"), [KEY]).contains(KEY));

    // The provider debug representation never exposes the key.
    let debug = format!("{:?}", OpenAiProvider::deepseek(KEY));
    assert!(!debug.contains(KEY));

    // A real trace capture of a request and response contains no key.
    let server = wiremock::MockServer::start().await;
    common::mount_sse(
        &server,
        common::script(&[
            r#"{"id":"resp-1","choices":[{"index":0,"delta":{"content":"ok"}}]}"#,
            r#"{"id":"resp-1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        ]),
    )
    .await;

    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(buffer.clone())
        .without_time()
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);

    let model = common::deepseek_model();
    let provider = OpenAiProvider::deepseek(KEY).with_base_url(server.uri());
    let events = common::drain(provider.stream(
        &[Message::user("hi")],
        &common::options(&model, ThinkingLevel::Off),
    ))
    .await;
    assert!(
        events
            .iter()
            .any(|event| event.terminal_message().is_some())
    );

    drop(guard);
    let logs = buffer.contents();
    assert!(logs.contains("provider request"), "trace log was empty");
    assert!(logs.contains("provider response event"));
    assert!(!logs.contains(KEY), "the API key leaked into the trace log");
}
