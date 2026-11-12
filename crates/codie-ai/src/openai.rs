//! The OpenAI-compatible chat-completions adapter used by DeepSeek and
//! OpenRouter (`SPEC-PROV-001`).
//!
//! [`OpenAiProvider`] owns HTTP, retries, SSE framing, tool-call assembly,
//! stop-reason mapping, and usage normalization. It implements
//! [`codie_core::agent::ChatClient`], so the agent loop never sees a
//! provider-specific type.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use codie_core::agent::{ChatClient, StreamOptions};
use codie_core::content::{Content, Cost, StopReason, ThinkingLevel, ToolCall, ToolDecl, Usage};
use codie_core::event::{AssistantDelta, AssistantEventStream, AssistantStreamSender};
use codie_core::message::{AssistantMessage, Message, collapse_system_messages};
use futures_util::StreamExt;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use crate::model::{
    Compat, DEFAULT_HTTP_REFERER, DEFAULT_X_TITLE, HeaderMap, MaxTokensField, Model, ModelCost,
    PROVIDER_DEEPSEEK, PROVIDER_OPENROUTER, ThinkingFormat, builtin_models, fallback_model,
    provider_base_url,
};
use crate::record::Recorder;
use crate::redact::{redact_text, redacted_headers};
use crate::sse::{SseEvent, SseParser};

/// Maximum request attempts, including the initial attempt.
pub const MAX_ATTEMPTS: usize = 3;

/// Maximum delay a server may request before codie fails fast.
pub const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);

/// Error bodies are truncated to this many characters (`PROV-004`).
pub const MAX_ERROR_BODY_CHARS: usize = 4000;

const CHANNEL_CAPACITY: usize = 256;

/// An OpenAI-compatible provider for DeepSeek and OpenRouter.
///
/// Construct one with [`OpenAiProvider::deepseek`],
/// [`OpenAiProvider::openrouter`], or [`OpenAiProvider::new`]. The provider is
/// cheap to clone and is usually wrapped in an [`std::sync::Arc`] by the agent.
#[derive(Clone)]
pub struct OpenAiProvider {
    provider_id: String,
    base_url: String,
    api_key: String,
    models: Vec<Model>,
    compat: Compat,
    headers: HeaderMap,
    client: reqwest::Client,
}

impl std::fmt::Debug for OpenAiProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenAiProvider")
            .field("provider_id", &self.provider_id)
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("models", &self.models)
            .field("compat", &self.compat)
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

impl OpenAiProvider {
    /// Creates a DeepSeek provider with the bundled model catalog.
    ///
    /// A trailing `/v1` in the base URL is accepted and normalized away
    /// (`SPEC-PROV-001` provider differences).
    #[must_use]
    pub fn deepseek(api_key: impl Into<String>) -> Self {
        let models = builtin_models()
            .into_iter()
            .filter(|model| model.provider == PROVIDER_DEEPSEEK)
            .collect();
        Self::new(
            PROVIDER_DEEPSEEK,
            provider_base_url(PROVIDER_DEEPSEEK),
            api_key,
            models,
            Compat::deepseek(),
            HeaderMap::new(),
        )
    }

    /// Creates an OpenRouter provider with `openrouter/auto` as its model.
    ///
    /// The default attribution headers required by OpenRouter are installed
    /// here; callers can override them through [`OpenAiProvider::new`].
    #[must_use]
    pub fn openrouter(api_key: impl Into<String>) -> Self {
        let models = builtin_models()
            .into_iter()
            .filter(|model| model.provider == PROVIDER_OPENROUTER)
            .collect();
        let mut headers = HeaderMap::new();
        headers.insert("HTTP-Referer".to_owned(), DEFAULT_HTTP_REFERER.to_owned());
        headers.insert("X-Title".to_owned(), DEFAULT_X_TITLE.to_owned());
        Self::new(
            PROVIDER_OPENROUTER,
            provider_base_url(PROVIDER_OPENROUTER),
            api_key,
            models,
            Compat::openrouter(),
            headers,
        )
    }

    /// Creates a provider from explicit configuration.
    ///
    /// `compat` is the fallback compatibility set for model identifiers that
    /// are not present in `models`. A model that *is* present always uses its
    /// own [`Model::compat`], keeping capability resolution per model
    /// (`PROV-019`).
    #[must_use]
    pub fn new(
        provider_id: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        models: Vec<Model>,
        compat: Compat,
        headers: HeaderMap,
    ) -> Self {
        let provider_id = provider_id.into();
        let base_url = normalize_base_url(&provider_id, &base_url.into());
        Self {
            provider_id,
            base_url,
            api_key: api_key.into(),
            models,
            compat,
            headers,
            client: reqwest::Client::new(),
        }
    }

    /// Returns the provider identifier.
    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    /// Returns the configured models.
    #[must_use]
    pub fn models(&self) -> &[Model] {
        &self.models
    }

    /// Returns the normalized base URL.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns a copy of this provider that sends requests to `base_url`.
    ///
    /// The override is intended for proxies and local test endpoints. Model
    /// descriptors keep their own base URL; only the transport target changes.
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = normalize_base_url(&self.provider_id, &base_url.into());
        self
    }

    /// Builds the JSON request body for a transcript.
    ///
    /// This is exposed for diagnostics and request-conversion tests; the
    /// streaming path uses the same function.
    #[must_use]
    pub fn request_body(&self, context: &[Message], options: &StreamOptions) -> Value {
        let model = self.resolve_model(&options.model.model);
        build_request_body(&model, context, options)
    }

    fn resolve_model(&self, id: &str) -> Model {
        if let Some(model) = self.models.iter().find(|model| model.id == id) {
            return model.clone();
        }
        let mut model = fallback_model(&self.provider_id, id);
        model.compat = self.compat;
        model.headers = self.headers.clone();
        model
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    fn apply_headers(
        &self,
        request: reqwest::RequestBuilder,
        session_id: Option<&str>,
        model_headers: &HeaderMap,
    ) -> reqwest::RequestBuilder {
        // `RequestBuilder::json` installs `Content-Type: application/json`;
        // adding it again here would send a duplicate header.
        let mut request = request
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Accept", "text/event-stream")
            .header("User-Agent", format!("codie/{}", codie_core::VERSION));

        if self.provider_id.eq_ignore_ascii_case(PROVIDER_OPENROUTER)
            && let Some(session_id) = session_id.filter(|value| !value.is_empty())
        {
            request = request.header("x-session-id", session_id);
        }

        let mut headers = self.headers.clone();
        headers.extend(model_headers.clone());
        for (name, value) in &headers {
            if name.eq_ignore_ascii_case("authorization")
                || name.eq_ignore_ascii_case("content-type")
            {
                continue;
            }
            request = request.header(name, value);
        }

        request
    }

    async fn run(
        &self,
        context: Vec<Message>,
        options: StreamOptions,
        sender: AssistantStreamSender,
    ) {
        let model = self.resolve_model(&options.model.model);
        let mut partial = AssistantMessage::partial(
            options.model.api.clone(),
            options.model.provider.clone(),
            options.model.model.clone(),
        );
        partial.thinking_level = Some(options.thinking.as_str().to_owned());
        partial.provider_thinking_level = provider_thinking_level(&model, options.thinking);

        let mut state = StreamState::new(partial, model, options.model.provider.clone());
        let started = Instant::now();

        if emit(
            &sender,
            AssistantDelta::Start {
                partial: state.partial.clone(),
            },
        )
        .await
        .is_err()
        {
            return;
        }

        let body = build_request_body(&state.model, &context, &options);
        let request = redact_text(&body.to_string(), [self.api_key.as_str()]);
        let headers = redacted_headers(&self.headers, &self.api_key);
        tracing::trace!(
            target: "codie_ai::openai",
            provider = %self.provider_id,
            endpoint = %self.endpoint(),
            headers = %headers,
            request = %request,
            "provider request"
        );

        let mut attempt = 0_usize;
        loop {
            if options.cancel.is_cancelled() {
                let _ = state
                    .fail(StopReason::Aborted, "request aborted", &sender, started)
                    .await;
                return;
            }

            attempt += 1;
            let request = self.client.post(self.endpoint()).json(&body);
            let request =
                self.apply_headers(request, options.session_id.as_deref(), &state.model.headers);

            let send_result = tokio::select! {
                biased;
                () = options.cancel.cancelled() => {
                    let _ = state
                        .fail(StopReason::Aborted, "request aborted", &sender, started)
                        .await;
                    return;
                }
                result = request.send() => result,
            };

            match send_result {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        match self
                            .consume(response, &mut state, &options, &sender, started)
                            .await
                        {
                            Ok(()) | Err(ConsumeError::Closed) => return,
                            Err(ConsumeError::Message(message)) => {
                                let _ = state
                                    .fail(StopReason::Error, message, &sender, started)
                                    .await;
                                return;
                            }
                        }
                    }

                    let retry_after = retry_after_delay(&response);
                    let retryable = should_retry_response(&response);
                    let body_text = truncate_chars(
                        &response.text().await.unwrap_or_default(),
                        MAX_ERROR_BODY_CHARS,
                    );

                    if retryable {
                        if retry_after.is_some_and(|delay| delay > MAX_RETRY_DELAY) {
                            let delay = retry_after.unwrap_or_default();
                            let message = format!(
                                "server requested a retry delay of {}ms, above the {}ms cap: {}: {}",
                                delay.as_millis(),
                                MAX_RETRY_DELAY.as_millis(),
                                status.as_u16(),
                                body_text
                            );
                            let _ = state
                                .fail(StopReason::Error, message, &sender, started)
                                .await;
                            return;
                        }

                        if attempt < MAX_ATTEMPTS {
                            let delay = retry_after.unwrap_or_else(|| {
                                backoff_delay(u32::try_from(attempt - 1).unwrap_or(0))
                            });

                            if !sleep_or_cancel(&options.cancel, delay).await {
                                let _ = state
                                    .fail(StopReason::Aborted, "request aborted", &sender, started)
                                    .await;
                                return;
                            }
                            continue;
                        }
                    }

                    let message = format!("{}: {body_text}", status.as_u16());
                    let _ = state
                        .fail(StopReason::Error, message, &sender, started)
                        .await;
                    return;
                }
                Err(error) => {
                    let message = normalize_transport_error(&error);
                    if attempt < MAX_ATTEMPTS {
                        let delay = backoff_delay(u32::try_from(attempt - 1).unwrap_or(0));
                        if !sleep_or_cancel(&options.cancel, delay).await {
                            let _ = state
                                .fail(StopReason::Aborted, "request aborted", &sender, started)
                                .await;
                            return;
                        }
                        continue;
                    }

                    let _ = state
                        .fail(StopReason::Error, message, &sender, started)
                        .await;
                    return;
                }
            }
        }
    }

    async fn consume(
        &self,
        response: reqwest::Response,
        state: &mut StreamState,
        options: &StreamOptions,
        sender: &AssistantStreamSender,
        started: Instant,
    ) -> Result<(), ConsumeError> {
        let mut bytes = response.bytes_stream();
        let mut parser = SseParser::new();
        let mut recorder = Recorder::from_env(&self.provider_id);

        loop {
            let next = tokio::select! {
                biased;
                () = options.cancel.cancelled() => {
                    state
                        .fail(StopReason::Aborted, "request aborted", sender, started)
                        .await?;
                    return Ok(());
                }
                next = bytes.next() => next,
            };

            let Some(chunk) = next else {
                break;
            };
            let chunk =
                chunk.map_err(|error| ConsumeError::Message(normalize_transport_error(&error)))?;
            recorder.write(&chunk);

            for event in parser.push(&chunk) {
                tracing::trace!(
                    target: "codie_ai::openai",
                    data = %redact_text(&event.data, [self.api_key.as_str()]),
                    "provider response event"
                );
                if event.is_done() {
                    state.finish(sender, started).await?;
                    return Ok(());
                }
                state.handle_event(&event, sender).await?;
            }
        }

        for event in parser.finish() {
            tracing::trace!(
                target: "codie_ai::openai",
                data = %redact_text(&event.data, [self.api_key.as_str()]),
                "provider response event"
            );
            if event.is_done() {
                state.finish(sender, started).await?;
                return Ok(());
            }
            state.handle_event(&event, sender).await?;
        }

        state.finish(sender, started).await
    }
}

impl ChatClient for OpenAiProvider {
    fn stream(&self, context: &[Message], options: &StreamOptions) -> AssistantEventStream {
        let (stream, sender) = AssistantEventStream::channel(CHANNEL_CAPACITY);
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            let message = AssistantMessage::error(
                options.model.api.clone(),
                options.model.provider.clone(),
                options.model.model.clone(),
                StopReason::Error,
                "no Tokio runtime is available for the provider request",
            );
            let _ = sender.try_send(AssistantDelta::Error {
                reason: StopReason::Error,
                message,
            });
            return stream;
        };

        let provider = self.clone();
        let context = context.to_vec();
        let options = options.clone();
        handle.spawn(async move {
            provider.run(context, options, sender).await;
        });
        stream
    }
}

fn normalize_base_url(provider: &str, base_url: &str) -> String {
    let mut normalized = base_url.trim().trim_end_matches('/').to_owned();
    if provider.eq_ignore_ascii_case(PROVIDER_DEEPSEEK)
        && let Some(stripped) = normalized.strip_suffix("/v1")
    {
        normalized = stripped.trim_end_matches('/').to_owned();
    }
    normalized
}

#[derive(Debug)]
enum ConsumeError {
    Closed,
    Message(String),
}

async fn emit(sender: &AssistantStreamSender, event: AssistantDelta) -> Result<(), ConsumeError> {
    if sender.send(event).await {
        Ok(())
    } else {
        Err(ConsumeError::Closed)
    }
}

struct PendingCall {
    content_index: usize,
    id: String,
    name: String,
    arguments: String,
}

struct StreamState {
    partial: AssistantMessage,
    model: Model,
    provider: String,
    usage: Option<Value>,
    finish_reason: Option<String>,
    response_id: Option<String>,
    response_model: Option<String>,
    thinking_index: Option<usize>,
    text_index: Option<usize>,
    thinking: String,
    text: String,
    calls: BTreeMap<usize, PendingCall>,
    warnings: Vec<String>,
}

impl StreamState {
    fn new(partial: AssistantMessage, model: Model, provider: String) -> Self {
        Self {
            partial,
            model,
            provider,
            usage: None,
            finish_reason: None,
            response_id: None,
            response_model: None,
            thinking_index: None,
            text_index: None,
            thinking: String::new(),
            text: String::new(),
            calls: BTreeMap::new(),
            warnings: Vec::new(),
        }
    }

    async fn handle_event(
        &mut self,
        event: &SseEvent,
        sender: &AssistantStreamSender,
    ) -> Result<(), ConsumeError> {
        let chunk: Value = serde_json::from_str(&event.data).map_err(|error| {
            ConsumeError::Message(format!(
                "invalid SSE payload: {error}; raw: {}",
                truncate_chars(&event.data, MAX_ERROR_BODY_CHARS)
            ))
        })?;

        self.handle_chunk(&chunk, sender).await
    }

    async fn handle_chunk(
        &mut self,
        chunk: &Value,
        sender: &AssistantStreamSender,
    ) -> Result<(), ConsumeError> {
        if let Some(id) = chunk.get("id").and_then(Value::as_str) {
            self.response_id = Some(id.to_owned());
        }
        if let Some(model) = chunk.get("model").and_then(Value::as_str) {
            self.response_model = Some(model.to_owned());
        }
        if let Some(usage) = chunk.get("usage").filter(|value| !value.is_null()) {
            self.usage = Some(usage.clone());
        }

        let Some(choices) = chunk.get("choices").and_then(Value::as_array) else {
            return Ok(());
        };
        if choices.len() > 1 {
            tracing::debug!(
                target: "codie_ai::openai",
                choices = choices.len(),
                "provider returned more than one choice; only index 0 is consumed"
            );
        }

        let Some(choice) = choices.first() else {
            return Ok(());
        };
        if choice
            .get("index")
            .and_then(json_u64)
            .is_some_and(|index| index != 0)
        {
            return Ok(());
        }

        if let Some(delta) = choice.get("delta").filter(|value| value.is_object()) {
            if let Some(reasoning) = delta
                .get("reasoning_content")
                .and_then(Value::as_str)
                .or_else(|| delta.get("reasoning").and_then(Value::as_str))
            {
                self.push_thinking(reasoning, sender).await?;
            }

            if let Some(content) = delta.get("content") {
                let text = text_content(content);
                if !text.is_empty() {
                    self.push_text(&text, sender).await?;
                }
            }

            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    self.push_tool_delta(call, sender).await?;
                }
            }
        }

        if let Some(finish_reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(finish_reason.to_owned());
            self.close(sender).await?;
        }

        Ok(())
    }

    async fn push_thinking(
        &mut self,
        delta: &str,
        sender: &AssistantStreamSender,
    ) -> Result<(), ConsumeError> {
        if delta.is_empty() {
            return Ok(());
        }

        if self.thinking_index.is_none() {
            let index = self.partial.content.len();
            self.partial.content.push(Content::thinking(""));
            self.thinking_index = Some(index);
            emit(sender, AssistantDelta::ThinkingStart { index }).await?;
        }

        let Some(index) = self.thinking_index else {
            return Ok(());
        };
        self.thinking.push_str(delta);
        if let Some(Content::Thinking { thinking, .. }) = self.partial.content.get_mut(index) {
            thinking.push_str(delta);
        }
        emit(
            sender,
            AssistantDelta::ThinkingDelta {
                index,
                delta: delta.to_owned(),
            },
        )
        .await
    }

    async fn push_text(
        &mut self,
        delta: &str,
        sender: &AssistantStreamSender,
    ) -> Result<(), ConsumeError> {
        if delta.is_empty() {
            return Ok(());
        }

        if self.text_index.is_none() {
            let index = self.partial.content.len();
            self.partial.content.push(Content::text(""));
            self.text_index = Some(index);
            emit(sender, AssistantDelta::TextStart { index }).await?;
        }

        let Some(index) = self.text_index else {
            return Ok(());
        };
        self.text.push_str(delta);
        if let Some(Content::Text { text, .. }) = self.partial.content.get_mut(index) {
            text.push_str(delta);
        }
        emit(
            sender,
            AssistantDelta::TextDelta {
                index,
                delta: delta.to_owned(),
            },
        )
        .await
    }

    async fn push_tool_delta(
        &mut self,
        call: &Value,
        sender: &AssistantStreamSender,
    ) -> Result<(), ConsumeError> {
        let provider_index =
            usize::try_from(call.get("index").and_then(json_u64).unwrap_or(0)).unwrap_or(0);

        if !self.calls.contains_key(&provider_index) {
            let content_index = self.partial.content.len();
            self.partial.content.push(Content::ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: json!({}),
                thought_signature: None,
            });
            self.calls.insert(
                provider_index,
                PendingCall {
                    content_index,
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                },
            );
            emit(
                sender,
                AssistantDelta::ToolCallStart {
                    index: content_index,
                },
            )
            .await?;
        }

        let mut argument_delta = None;
        let mut content_index = 0;
        if let Some(pending) = self.calls.get_mut(&provider_index) {
            content_index = pending.content_index;

            if let Some(id) = call.get("id").and_then(Value::as_str)
                && pending.id.is_empty()
                && !id.is_empty()
            {
                pending.id = id.to_owned();
            }

            if let Some(function) = call.get("function") {
                if let Some(name) = function.get("name").and_then(Value::as_str)
                    && pending.name.is_empty()
                    && !name.is_empty()
                {
                    pending.name = name.to_owned();
                }
                if let Some(arguments) = function
                    .get("arguments")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                {
                    pending.arguments.push_str(arguments);
                    argument_delta = Some(arguments.to_owned());
                }
            }
        }

        if let Some(delta) = argument_delta {
            emit(
                sender,
                AssistantDelta::ToolCallDelta {
                    index: content_index,
                    delta,
                },
            )
            .await?;
        }

        Ok(())
    }

    async fn close(&mut self, sender: &AssistantStreamSender) -> Result<(), ConsumeError> {
        if let Some(index) = self.thinking_index.take() {
            let content = self.thinking.clone();
            if let Some(Content::Thinking { thinking, .. }) = self.partial.content.get_mut(index) {
                *thinking = content.clone();
            }
            emit(sender, AssistantDelta::ThinkingEnd { index, content }).await?;
        }

        if let Some(index) = self.text_index.take() {
            let content = self.text.clone();
            if let Some(Content::Text { text, .. }) = self.partial.content.get_mut(index) {
                *text = content.clone();
            }
            emit(sender, AssistantDelta::TextEnd { index, content }).await?;
        }

        self.finish_calls(sender).await
    }

    async fn finish_calls(&mut self, sender: &AssistantStreamSender) -> Result<(), ConsumeError> {
        let calls = std::mem::take(&mut self.calls);

        for pending in calls.into_values() {
            let (arguments, error) = parse_arguments(&pending.arguments);
            let call = ToolCall {
                id: pending.id.clone(),
                name: pending.name.clone(),
                arguments: arguments.clone(),
            };

            if let Some(Content::ToolCall {
                id,
                name,
                arguments: slot,
                ..
            }) = self.partial.content.get_mut(pending.content_index)
            {
                *id = pending.id.clone();
                *name = pending.name.clone();
                *slot = arguments;
            }

            emit(
                sender,
                AssistantDelta::ToolCallEnd {
                    index: pending.content_index,
                    call,
                },
            )
            .await?;

            if let Some(error) = error {
                self.warnings.push(format!(
                    "{}: {error} (raw arguments: {})",
                    pending.name,
                    truncate_chars(&pending.arguments, 200)
                ));
            }
        }

        if !self.warnings.is_empty() {
            let note = format!(
                "[codie: malformed tool-call arguments: {}]",
                self.warnings.join("; ")
            );
            self.warnings.clear();
            let index = self.partial.content.len();
            self.partial.content.push(Content::text(note.clone()));
            emit(sender, AssistantDelta::TextStart { index }).await?;
            emit(
                sender,
                AssistantDelta::TextDelta {
                    index,
                    delta: note.clone(),
                },
            )
            .await?;
            emit(
                sender,
                AssistantDelta::TextEnd {
                    index,
                    content: note,
                },
            )
            .await?;
        }

        Ok(())
    }

    async fn finish(
        &mut self,
        sender: &AssistantStreamSender,
        started: Instant,
    ) -> Result<(), ConsumeError> {
        self.close(sender).await?;

        self.partial.usage = match self.usage.as_ref() {
            Some(raw) => normalize_usage(&self.provider, raw, &self.model.cost),
            None => {
                let mut usage = Usage::default();
                usage.cost = estimate_cost(&self.model.cost, &usage);
                usage
            }
        };
        self.partial.response_id = self.response_id.clone();
        self.partial.response_model = self.response_model.clone();
        self.partial.duration_ms = Some(elapsed_ms(started));

        match self.finish_reason.as_deref() {
            Some(raw) => match map_stop_reason(Some(raw)) {
                Ok(reason) => {
                    self.partial.stop_reason = reason;
                    self.partial.raw_stop_reason = Some(raw.to_owned());
                    emit(
                        sender,
                        AssistantDelta::Done {
                            reason,
                            message: self.partial.clone(),
                        },
                    )
                    .await?;
                }
                Err(message) => {
                    self.partial.stop_reason = StopReason::Error;
                    self.partial.raw_stop_reason = Some(raw.to_owned());
                    self.partial.error_message = Some(message);
                    emit(
                        sender,
                        AssistantDelta::Error {
                            reason: StopReason::Error,
                            message: self.partial.clone(),
                        },
                    )
                    .await?;
                }
            },
            None if self.model.compat.supports_finish_reason => {
                let message = "stream ended without finish_reason".to_owned();
                self.partial.stop_reason = StopReason::Error;
                self.partial.error_message = Some(message);
                emit(
                    sender,
                    AssistantDelta::Error {
                        reason: StopReason::Error,
                        message: self.partial.clone(),
                    },
                )
                .await?;
            }
            None => {
                self.partial.stop_reason = StopReason::Stop;
                emit(
                    sender,
                    AssistantDelta::Done {
                        reason: StopReason::Stop,
                        message: self.partial.clone(),
                    },
                )
                .await?;
            }
        }

        Ok(())
    }

    async fn fail(
        &mut self,
        reason: StopReason,
        message: impl Into<String>,
        sender: &AssistantStreamSender,
        started: Instant,
    ) -> Result<(), ConsumeError> {
        let _ = self.close(sender).await;
        self.partial.stop_reason = reason;
        self.partial.error_message = Some(message.into());
        self.partial.duration_ms = Some(elapsed_ms(started));
        emit(
            sender,
            AssistantDelta::Error {
                reason,
                message: self.partial.clone(),
            },
        )
        .await
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn text_content(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn parse_arguments(raw: &str) -> (Value, Option<String>) {
    if raw.trim().is_empty() {
        return (json!({}), Some("tool-call arguments were empty".to_owned()));
    }

    match serde_json::from_str::<Value>(raw) {
        Ok(arguments @ Value::Object(_)) => (arguments, None),
        Ok(other) => (
            json!({}),
            Some(format!(
                "tool-call arguments must be a JSON object, got {}",
                json_kind(&other)
            )),
        ),
        Err(error) => (json!({}), Some(format!("invalid JSON: {error}"))),
    }
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Maps a provider finish reason to a normalized [`StopReason`] (`PROV-014`).
///
/// Returns the diagnostic message for reasons that must be treated as errors.
/// Unknown values are never guessed as a normal stop.
///
/// # Errors
///
/// Returns a human-readable message for `content_filter` and unknown reasons.
pub fn map_stop_reason(raw: Option<&str>) -> Result<StopReason, String> {
    match raw {
        None | Some("stop") | Some("end_turn") => Ok(StopReason::Stop),
        Some("length") | Some("max_tokens") => Ok(StopReason::Length),
        Some("tool_calls") | Some("function_call") => Ok(StopReason::ToolUse),
        Some("content_filter") => Err("provider stopped because of content_filter".to_owned()),
        Some(other) => Err(format!("provider returned unknown finish_reason: {other}")),
    }
}

fn should_retry_response(response: &reqwest::Response) -> bool {
    let status = response.status().as_u16();
    let retryable_status =
        status == 408 || status == 409 || status == 429 || (500..=599).contains(&status);
    retryable_status
        || response
            .headers()
            .get("x-should-retry")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn retry_after_delay(response: &reqwest::Response) -> Option<Duration> {
    if let Some(value) = response
        .headers()
        .get("retry-after-ms")
        .and_then(|value| value.to_str().ok())
    {
        if let Ok(milliseconds) = value.trim().parse::<f64>()
            && milliseconds.is_finite()
            && milliseconds >= 0.0
        {
            return Some(Duration::from_secs_f64(milliseconds / 1000.0));
        }
    }

    if let Some(value) = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
    {
        if let Ok(seconds) = value.trim().parse::<f64>()
            && seconds.is_finite()
            && seconds >= 0.0
        {
            return Some(Duration::from_secs_f64(seconds));
        }
        // HTTP-date form is intentionally skipped; the caller falls back to
        // exponential backoff rather than guessing a wall-clock delay.
    }

    None
}

/// Returns the exponential backoff delay for a zero-based retry number.
///
/// The delay is `min(0.5 * 2^attempt, 8s)` (`PROV-003`). Jitter is omitted so
/// retry behavior is deterministic and observable in tests.
#[must_use]
pub fn backoff_delay(attempt: u32) -> Duration {
    let exponent = i32::try_from(attempt).unwrap_or(i32::MAX);
    let seconds = 0.5 * 2_f64.powi(exponent);
    Duration::from_secs_f64(seconds.min(8.0))
}

async fn sleep_or_cancel(cancel: &CancellationToken, delay: Duration) -> bool {
    tokio::select! {
        biased;
        () = cancel.cancelled() => false,
        () = tokio::time::sleep(delay) => true,
    }
}

/// Normalizes a transport error by appending its deepest cause chain.
#[must_use]
pub fn normalize_transport_error(error: &reqwest::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    truncate_chars(&parts.join(": "), MAX_ERROR_BODY_CHARS)
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn json_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|number| number.is_finite() && *number >= 0.0)
            .map(|number| number as u64)
    })
}

fn field_u64(value: &Value, field: &str) -> Option<u64> {
    value.get(field).and_then(json_u64)
}

fn nested_u64(value: &Value, outer: &str, inner: &str) -> Option<u64> {
    value
        .get(outer)
        .and_then(|details| details.get(inner))
        .and_then(json_u64)
}

/// Normalizes provider usage into the codie [`Usage`] shape (`PROV-015`).
///
/// DeepSeek reports `prompt_cache_hit_tokens`; OpenRouter reports
/// `prompt_tokens_details.cached_tokens` and an authoritative USD `cost`.
/// `total_tokens` is always recomputed with the `MSG-005` formula.
#[must_use]
pub fn normalize_usage(provider: &str, raw: &Value, model_cost: &ModelCost) -> Usage {
    let deepseek = provider.eq_ignore_ascii_case(PROVIDER_DEEPSEEK);
    let prompt_tokens = field_u64(raw, "prompt_tokens").unwrap_or(0);
    let completion_tokens = field_u64(raw, "completion_tokens").unwrap_or(0);

    let cache_read = if deepseek {
        field_u64(raw, "prompt_cache_hit_tokens")
            .or_else(|| nested_u64(raw, "prompt_tokens_details", "cached_tokens"))
    } else {
        nested_u64(raw, "prompt_tokens_details", "cached_tokens")
            .or_else(|| field_u64(raw, "prompt_cache_hit_tokens"))
    }
    .unwrap_or(0);

    let mut usage = Usage::new(
        prompt_tokens.saturating_sub(cache_read),
        completion_tokens,
        cache_read,
        0,
    );
    usage.reasoning = nested_u64(raw, "completion_tokens_details", "reasoning_tokens");
    usage.recompute_total();

    if !deepseek && let Some(cost) = raw.get("cost").and_then(Value::as_f64) {
        usage.cost = Cost {
            total: Cost::nanos_from_usd(cost),
            total_from_provider: true,
            ..Cost::default()
        };
        if let Some(byok) = raw.get("is_byok") {
            tracing::debug!(target: "codie_ai::openai", is_byok = %byok, "openrouter BYOK usage");
        }
        if let Some(details) = raw.get("cost_details") {
            tracing::debug!(target: "codie_ai::openai", cost_details = %details, "openrouter cost details");
        }
    }

    if !usage.cost.total_from_provider {
        usage.cost = estimate_cost(model_cost, &usage);
    }

    usage
}

/// Estimates cost in nano-USD from a model's per-million-token prices.
///
/// `nano = round(tokens * usd_per_million * 1000)` (`MSG-006`).
#[must_use]
pub fn estimate_cost(model_cost: &ModelCost, usage: &Usage) -> Cost {
    let mut cost = Cost::new(
        token_cost(usage.input, model_cost.input),
        token_cost(usage.output, model_cost.output),
        token_cost(usage.cache_read, model_cost.cache_read),
        token_cost(usage.cache_write, model_cost.cache_write),
    );
    cost.total_from_provider = false;
    cost
}

fn token_cost(tokens: u64, usd_per_million: f64) -> u64 {
    if tokens == 0 || !usd_per_million.is_finite() || usd_per_million <= 0.0 {
        return 0;
    }
    let nanos = tokens as f64 * usd_per_million * 1000.0;
    if nanos >= u64::MAX as f64 {
        u64::MAX
    } else {
        nanos.round() as u64
    }
}

/// Builds the OpenAI-compatible JSON request body for one model.
///
/// Fields are emitted only when the model's [`Compat`] allows them
/// (`PROV-019`).
#[must_use]
pub fn build_request_body(model: &Model, context: &[Message], options: &StreamOptions) -> Value {
    let messages = collapse_system_messages(context.to_vec());
    let tools = leading_tools(&messages);

    let mut body = Map::new();
    body.insert("model".to_owned(), Value::String(model.id.clone()));
    body.insert(
        "messages".to_owned(),
        Value::Array(
            messages
                .iter()
                .map(|message| wire_message(message, &model.compat))
                .collect(),
        ),
    );
    body.insert("stream".to_owned(), Value::Bool(true));

    if model.compat.supports_usage_in_streaming {
        body.insert(
            "stream_options".to_owned(),
            json!({ "include_usage": true }),
        );
    }

    let max_tokens = options
        .max_output_tokens
        .or_else(|| (model.max_output_tokens > 0).then_some(model.max_output_tokens));
    if let Some(max_tokens) = max_tokens {
        let field = match model.compat.max_tokens_field {
            MaxTokensField::MaxTokens => "max_tokens",
            MaxTokensField::MaxCompletionTokens => "max_completion_tokens",
        };
        body.insert(field.to_owned(), json!(max_tokens));
    }

    if let Some(temperature) = options.temperature {
        body.insert("temperature".to_owned(), json!(temperature));
    }

    if !tools.is_empty() {
        body.insert("tools".to_owned(), wire_tools(&tools));
    }

    insert_thinking(&mut body, model, options.thinking);

    Value::Object(body)
}

fn leading_tools(messages: &[Message]) -> Vec<ToolDecl> {
    messages
        .first()
        .and_then(|message| match message {
            Message::System(system) => Some(system.tools.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn wire_tools(tools: &[ToolDecl]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            })
            .collect(),
    )
}

fn wire_message(message: &Message, compat: &Compat) -> Value {
    match message {
        Message::System(system) => json!({
            "role": "system",
            "content": system.content,
        }),
        Message::User(user) => json!({
            "role": "user",
            "content": wire_user_content(&user.content),
        }),
        Message::Assistant(assistant) => {
            let mut object = Map::new();
            object.insert("role".to_owned(), Value::String("assistant".to_owned()));

            let text = assistant.text();
            object.insert(
                "content".to_owned(),
                if text.is_empty() {
                    Value::Null
                } else {
                    Value::String(text)
                },
            );

            let calls = assistant
                .content
                .iter()
                .filter_map(|block| match block {
                    Content::ToolCall {
                        id,
                        name,
                        arguments,
                        ..
                    } => Some(json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": serde_json::to_string(arguments)
                                .unwrap_or_else(|_| "{}".to_owned()),
                        }
                    })),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if !calls.is_empty() {
                object.insert("tool_calls".to_owned(), Value::Array(calls));
            }

            if compat.requires_reasoning_content_on_assistant_messages {
                let reasoning = assistant
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        Content::Thinking { thinking, .. } => Some(thinking.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                object.insert("reasoning_content".to_owned(), Value::String(reasoning));
            }

            Value::Object(object)
        }
        Message::ToolResult(result) => json!({
            "role": "tool",
            "tool_call_id": result.tool_call_id,
            "content": plain_content(&result.content),
        }),
    }
}

fn wire_user_content(blocks: &[Content]) -> Value {
    let all_text = blocks
        .iter()
        .all(|block| matches!(block, Content::Text { .. }));
    if all_text {
        return Value::String(
            blocks
                .iter()
                .filter_map(Content::as_text)
                .collect::<Vec<_>>()
                .join(""),
        );
    }

    Value::Array(
        blocks
            .iter()
            .filter_map(|block| match block {
                Content::Text { text, .. } => Some(json!({ "type": "text", "text": text })),
                Content::Image { data, mime_type } => Some(json!({
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:{mime_type};base64,{data}"),
                    },
                })),
                _ => None,
            })
            .collect(),
    )
}

fn plain_content(blocks: &[Content]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            Content::Text { text, .. } => Some(text.clone()),
            Content::Image { mime_type, .. } => Some(format!("[image omitted: {mime_type}]")),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn effective_level(model: &Model, requested: ThinkingLevel) -> Option<ThinkingLevel> {
    if requested == ThinkingLevel::Off {
        return None;
    }
    if model.thinking_levels.contains_key(&requested) {
        return Some(requested);
    }
    model
        .thinking_levels
        .keys()
        .rev()
        .copied()
        .find(|candidate| *candidate < requested && *candidate != ThinkingLevel::Off)
}

fn insert_thinking(body: &mut Map<String, Value>, model: &Model, requested: ThinkingLevel) {
    if !model.reasoning {
        return;
    }

    match model.compat.thinking_format {
        ThinkingFormat::DeepSeek => {
            if requested == ThinkingLevel::Off {
                body.insert("thinking".to_owned(), json!({ "type": "disabled" }));
                return;
            }
            let Some(level) = effective_level(model, requested) else {
                return;
            };
            body.insert("thinking".to_owned(), json!({ "type": "enabled" }));
            if let Some(effort) = model.thinking_levels.get(&level).cloned().flatten() {
                body.insert("reasoning_effort".to_owned(), Value::String(effort));
            }
        }
        ThinkingFormat::OpenRouter => {
            let effort = if requested == ThinkingLevel::Off {
                "none".to_owned()
            } else {
                model
                    .thinking_effort(requested)
                    .unwrap_or_else(|| requested.as_str().to_owned())
            };
            body.insert("reasoning".to_owned(), json!({ "effort": effort }));
        }
        ThinkingFormat::OpenAi => {
            if requested != ThinkingLevel::Off
                && let Some(effort) = model.thinking_effort(requested)
            {
                body.insert("reasoning_effort".to_owned(), Value::String(effort));
            }
        }
    }
}

fn provider_thinking_level(model: &Model, requested: ThinkingLevel) -> Option<String> {
    if !model.reasoning || requested == ThinkingLevel::Off {
        return None;
    }
    match model.compat.thinking_format {
        ThinkingFormat::DeepSeek => effective_level(model, requested)
            .and_then(|level| model.thinking_levels.get(&level).cloned().flatten()),
        ThinkingFormat::OpenRouter | ThinkingFormat::OpenAi => model.thinking_effort(requested),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::find_model;

    #[test]
    fn stop_reasons_map_exactly() {
        assert_eq!(map_stop_reason(None), Ok(StopReason::Stop));
        assert_eq!(map_stop_reason(Some("stop")), Ok(StopReason::Stop));
        assert_eq!(map_stop_reason(Some("end_turn")), Ok(StopReason::Stop));
        assert_eq!(map_stop_reason(Some("length")), Ok(StopReason::Length));
        assert_eq!(map_stop_reason(Some("max_tokens")), Ok(StopReason::Length));
        assert_eq!(map_stop_reason(Some("tool_calls")), Ok(StopReason::ToolUse));
        assert_eq!(
            map_stop_reason(Some("function_call")),
            Ok(StopReason::ToolUse)
        );
        assert!(map_stop_reason(Some("content_filter")).is_err());
        assert!(map_stop_reason(Some("surprise")).is_err());
    }

    #[test]
    fn backoff_is_capped_at_eight_seconds() {
        assert_eq!(backoff_delay(0), Duration::from_millis(500));
        assert_eq!(backoff_delay(1), Duration::from_secs(1));
        assert_eq!(backoff_delay(2), Duration::from_secs(2));
        assert_eq!(backoff_delay(4), Duration::from_secs(8));
        assert_eq!(backoff_delay(30), Duration::from_secs(8));
    }

    #[test]
    fn non_object_tool_arguments_are_recoverable() {
        let (arguments, error) = parse_arguments("[]");
        assert_eq!(arguments, json!({}));
        assert!(error.is_some_and(|error| error.contains("JSON object")));

        let (arguments, error) = parse_arguments("{\"path\":\"a\"}");
        assert_eq!(arguments, json!({"path": "a"}));
        assert!(error.is_none());
    }

    #[test]
    fn estimates_cost_in_nano_usd() {
        let flash = find_model("deepseek", "deepseek-flash").unwrap();
        let usage = Usage::new(1_000_000, 1_000_000, 0, 0);
        let cost = estimate_cost(&flash.cost, &usage);
        assert_eq!(cost.input, 300_000_000);
        assert_eq!(cost.output, 1_200_000_000);
        assert_eq!(cost.total, 1_500_000_000);
        assert!(!cost.total_from_provider);
    }

    #[test]
    fn usage_mapping_subtracts_cached_input() {
        let flash = find_model("deepseek", "deepseek-flash").unwrap();
        let raw = json!({
            "prompt_tokens": 1200,
            "completion_tokens": 80,
            "prompt_cache_hit_tokens": 900,
            "completion_tokens_details": {"reasoning_tokens": 20}
        });
        let usage = normalize_usage("deepseek", &raw, &flash.cost);
        assert_eq!(usage.input, 300);
        assert_eq!(usage.cache_read, 900);
        assert_eq!(usage.output, 80);
        assert_eq!(usage.reasoning, Some(20));
        assert_eq!(usage.total_tokens, 1280);
    }

    #[test]
    fn openrouter_cost_is_authoritative() {
        let flash = find_model("deepseek", "deepseek-flash").unwrap();
        let raw = json!({
            "prompt_tokens": 100,
            "completion_tokens": 10,
            "prompt_tokens_details": {"cached_tokens": 40},
            "cost": 0.00042
        });
        let usage = normalize_usage("openrouter", &raw, &flash.cost);
        assert_eq!(usage.input, 60);
        assert_eq!(usage.cache_read, 40);
        assert_eq!(usage.cost.total, 420_000);
        assert!(usage.cost.total_from_provider);
    }
}
