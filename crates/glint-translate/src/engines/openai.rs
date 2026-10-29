//! OpenAI-compatible Chat Completions with streaming.
//!
//! Works with OpenAI, DeepSeek, Ollama, OpenRouter and anything else that speaks
//! `POST {base_url}/chat/completions` with `stream: true`.
//!
//! * `base_url` defaults to `https://api.openai.com/v1`. A base with no path (such as
//!   `http://localhost:11434`) gets `/v1` added, since that is where every compatible server
//!   puts the API. The API key is optional for anything but OpenAI itself (Ollama has none).
//! * On `api.openai.com` the token limit is sent as `max_completion_tokens`, and the reasoning
//!   models (`o1`, `o3`, `o4`, `gpt-5`) get no `temperature`, because they reject it. Other
//!   servers get `max_tokens` and `temperature: 0`, which every compatible server accepts.
//! * Reasoning models served locally (DeepSeek-R1, Qwen3 on Ollama) often put a
//!   `<think>...</think>` block at the start of `content`. [`ThinkFilter`] removes it from the
//!   stream.

use serde_json::{json, Value};

use super::{non_empty, parse_err};
use crate::http::{
    is_event_stream, join_endpoint, retrying, send, stream_sse, Failure, Provider, Step,
    LLM_TIMEOUT,
};
use crate::prompt;
use crate::sse::SseEvent;
use crate::{DeltaSink, EngineConfig, Result, TranslateError, TranslateRequest, Translator};

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

pub(crate) struct OpenAi {
    id: String,
    name: String,
    api_key: Option<String>,
    model: String,
    url: String,
    official: bool,
    extra_prompt: Option<String>,
    http: reqwest::Client,
}

impl OpenAi {
    pub fn new(
        id: String,
        name: String,
        cfg: &EngineConfig,
        http: reqwest::Client,
    ) -> Result<Self> {
        let api_key = non_empty(cfg.api_key.as_deref()).map(str::to_owned);
        let base = non_empty(cfg.base_url.as_deref());
        let official = base.is_none_or(|b| b.contains("api.openai.com"));
        if official && api_key.is_none() {
            return Err(TranslateError::Config(
                "The OpenAI API key is not set".into(),
            ));
        }
        Ok(Self {
            id,
            name,
            api_key,
            model: super::effective_model(cfg),
            url: endpoint_url(base.unwrap_or(DEFAULT_BASE_URL)),
            official,
            extra_prompt: non_empty(cfg.prompt.as_deref()).map(str::to_owned),
            http,
        })
    }

    async fn attempt(
        &self,
        body: &Value,
        deltas: &DeltaSink,
    ) -> std::result::Result<String, Failure> {
        let mut rb = self
            .http
            .post(&self.url)
            .timeout(LLM_TIMEOUT)
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .json(body);
        if let Some(key) = &self.api_key {
            rb = rb.bearer_auth(key);
        }
        let resp = send(rb, Provider::OpenAi).await?;

        if !is_event_stream(&resp) {
            let text = resp.text().await?;
            let out = parse_completion_json(&text)?;
            let _ = deltas.send(out.clone());
            return Ok(out);
        }

        let mut state = StreamState::default();
        let streamed = stream_sse(resp, deltas, |ev| state.on_event(ev)).await?;
        // Text the think filter was still holding back (a stream that ends mid-tag).
        let tail = state.flush();
        if !tail.is_empty() {
            let _ = deltas.send(tail);
        }
        state
            .finish()
            .map_err(|e| Failure::fatal(e).no_retry_if(streamed))
    }
}

#[async_trait::async_trait]
impl Translator for OpenAi {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    async fn translate(&self, req: &TranslateRequest, deltas: DeltaSink) -> Result<String> {
        if req.text.trim().is_empty() {
            return Ok(String::new());
        }
        let system = prompt::system_prompt(req, self.extra_prompt.as_deref());
        let user = prompt::user_message(&req.text);
        let body = request_body(&self.model, &system, &user, &req.text, self.official);
        retrying(|| self.attempt(&body, &deltas)).await
    }
}

/// `{base}/chat/completions`, adding `/v1` to a base that has no path at all.
pub(crate) fn endpoint_url(base: &str) -> String {
    let base = base.trim();
    let with_version = match reqwest::Url::parse(base) {
        Ok(u) if u.path() == "/" || u.path().is_empty() => {
            format!("{}/v1", base.trim_end_matches('/'))
        }
        _ => base.to_string(),
    };
    join_endpoint(&with_version, "/chat/completions")
}

/// Models that only accept the default `temperature` and count thinking against the limit.
fn is_reasoning_model(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    ["o1", "o3", "o4", "gpt-5"].iter().any(|p| m.starts_with(p))
}

pub(crate) fn request_body(
    model: &str,
    system: &str,
    user: &str,
    source_text: &str,
    official: bool,
) -> Value {
    let reasoning = official && is_reasoning_model(model);
    let max = prompt::max_output_tokens(source_text, if reasoning { 4096 } else { 0 });
    let mut body = json!({
        "model": model,
        "stream": true,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
    });
    if official {
        body["max_completion_tokens"] = json!(max);
    } else {
        body["max_tokens"] = json!(max);
    }
    if !reasoning {
        body["temperature"] = json!(0);
    }
    body
}

/// One parsed streaming chunk.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Chunk {
    Text(String),
    Done,
    /// Nothing for us in this chunk (role announcement, usage, empty choices, keep-alive).
    Skip,
}

/// Parse the `data:` payload of one streaming event.
pub(crate) fn parse_chunk(
    data: &str,
) -> std::result::Result<(Chunk, Option<String>), TranslateError> {
    let data = data.trim();
    if data == "[DONE]" {
        return Ok((Chunk::Done, None));
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return Ok((Chunk::Skip, None));
    };
    if v.get("error").is_some_and(|e| !e.is_null()) {
        return Err(api_error(&v));
    }
    let choice = v.pointer("/choices/0");
    let finish = choice
        .and_then(|c| c.get("finish_reason"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let text = choice
        .and_then(|c| c.pointer("/delta/content"))
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty());
    Ok((
        text.map_or(Chunk::Skip, |t| Chunk::Text(t.to_string())),
        finish,
    ))
}

/// Map an error object (`{"error": {...}}`), which can arrive as an HTTP body or mid-stream.
fn api_error(v: &Value) -> TranslateError {
    let code = v.pointer("/error/code");
    let status = match code {
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .unwrap_or(500),
        Some(Value::String(s)) => match s.as_str() {
            "invalid_api_key" | "invalid_authentication" => 401,
            "rate_limit_exceeded" | "insufficient_quota" => 429,
            "model_not_found" => 404,
            "context_length_exceeded" | "invalid_request_error" => 400,
            _ => 500,
        },
        _ => 500,
    };
    crate::http::service_error(status, &v.to_string(), Provider::OpenAi)
}

/// Accumulates a streamed completion.
#[derive(Debug, Default)]
pub(crate) struct StreamState {
    text: String,
    filter: ThinkFilter,
    finish_reason: Option<String>,
}

impl StreamState {
    pub fn on_event(&mut self, ev: &SseEvent) -> std::result::Result<Step, TranslateError> {
        let (chunk, finish) = parse_chunk(&ev.data)?;
        if finish.is_some() {
            self.finish_reason = finish;
        }
        match chunk {
            Chunk::Text(t) => {
                let visible = self.filter.push(&t);
                if visible.is_empty() {
                    return Ok(Step::Continue);
                }
                self.text.push_str(&visible);
                Ok(Step::Emit(visible))
            }
            Chunk::Done => Ok(Step::Stop),
            Chunk::Skip => Ok(Step::Continue),
        }
    }

    /// Release whatever the think filter still holds. Call once, after the stream ends.
    pub fn flush(&mut self) -> String {
        let tail = self.filter.finish();
        self.text.push_str(&tail);
        tail
    }

    pub fn finish(self) -> std::result::Result<String, TranslateError> {
        match self.finish_reason.as_deref() {
            Some("content_filter") => {
                return Err(TranslateError::Service {
                    status: 200,
                    message: "The provider's content filter blocked this text".into(),
                })
            }
            Some("length") => {
                log::warn!("openai: the translation hit the token limit and may be cut short")
            }
            _ => {}
        }
        let text = self.text.trim().to_string();
        if text.is_empty() {
            return Err(TranslateError::Parse("the model returned no text".into()));
        }
        Ok(text)
    }
}

/// Parse a complete (non-streamed) completion.
pub(crate) fn parse_completion_json(body: &str) -> std::result::Result<String, TranslateError> {
    let v: Value = serde_json::from_str(body).map_err(|_| parse_err("the response is not JSON"))?;
    if v.get("error").is_some_and(|e| !e.is_null()) {
        return Err(api_error(&v));
    }
    if v.pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        == Some("content_filter")
    {
        return Err(TranslateError::Service {
            status: 200,
            message: "The provider's content filter blocked this text".into(),
        });
    }
    let content = v
        .pointer("/choices/0/message/content")
        .ok_or_else(|| parse_err("the response has no message"))?;
    let raw: String = match content {
        Value::String(s) => s.clone(),
        // Some servers return content parts.
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect(),
        _ => String::new(),
    };
    let mut filter = ThinkFilter::default();
    let mut visible = filter.push(&raw);
    visible.push_str(&filter.finish());
    let text = visible.trim().to_string();
    if text.is_empty() {
        return Err(parse_err("the model returned no text"));
    }
    Ok(text)
}

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum FilterState {
    /// Still deciding whether the text starts with `<think>`.
    #[default]
    Start,
    /// Inside the block; everything is dropped.
    InThink,
    /// Just after `</think>`: drop whitespace until the real text begins.
    AfterThink,
    /// Plain text from here on.
    Pass,
}

/// Removes a leading `<think>...</think>` block from a stream of text deltas, however the
/// deltas are split. Only a block at the very start (after optional whitespace) counts, so a
/// translation that happens to mention the tag later is left alone.
#[derive(Debug, Default)]
pub(crate) struct ThinkFilter {
    state: FilterState,
    buf: String,
}

impl ThinkFilter {
    pub fn push(&mut self, s: &str) -> String {
        match self.state {
            FilterState::Pass => s.to_string(),
            FilterState::AfterThink => {
                let t = s.trim_start();
                if t.is_empty() {
                    String::new()
                } else {
                    self.state = FilterState::Pass;
                    t.to_string()
                }
            }
            FilterState::Start => {
                self.buf.push_str(s);
                self.resolve_start()
            }
            FilterState::InThink => {
                self.buf.push_str(s);
                self.resolve_think()
            }
        }
    }

    /// The stream ended. Text held back while deciding is released; an unterminated think
    /// block yields nothing.
    pub fn finish(&mut self) -> String {
        match self.state {
            FilterState::Start => {
                self.state = FilterState::Pass;
                let rest = std::mem::take(&mut self.buf);
                if rest.trim().is_empty() {
                    String::new()
                } else {
                    rest
                }
            }
            _ => String::new(),
        }
    }

    fn resolve_start(&mut self) -> String {
        let trimmed = self.buf.trim_start();
        if trimmed.is_empty() {
            return String::new();
        }
        if let Some(rest) = trimmed.strip_prefix(THINK_OPEN) {
            self.buf = rest.to_string();
            self.state = FilterState::InThink;
            return self.resolve_think();
        }
        if THINK_OPEN.starts_with(trimmed) {
            // A prefix of the tag, such as "<th": wait for more text.
            return String::new();
        }
        self.state = FilterState::Pass;
        std::mem::take(&mut self.buf)
    }

    fn resolve_think(&mut self) -> String {
        if let Some(i) = self.buf.find(THINK_CLOSE) {
            let rest = self.buf[i + THINK_CLOSE.len()..].trim_start().to_string();
            self.buf.clear();
            if rest.is_empty() {
                self.state = FilterState::AfterThink;
                return String::new();
            }
            self.state = FilterState::Pass;
            return rest;
        }
        // Keep just enough to recognise a closing tag that is split across deltas.
        let keep = THINK_CLOSE.len() - 1;
        if self.buf.len() > keep {
            let mut cut = self.buf.len() - keep;
            while !self.buf.is_char_boundary(cut) {
                cut -= 1;
            }
            self.buf.drain(..cut);
        }
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(data: &str) -> SseEvent {
        SseEvent {
            event: None,
            data: data.to_string(),
        }
    }

    fn run_stream(raw: &str) -> (std::result::Result<String, TranslateError>, Vec<String>) {
        let mut parser = crate::sse::SseParser::new();
        let mut events = parser.feed(raw.as_bytes());
        events.extend(parser.finish());
        let mut state = StreamState::default();
        let mut deltas = Vec::new();
        for e in &events {
            match state.on_event(e) {
                Ok(Step::Emit(t)) => deltas.push(t),
                Ok(Step::Continue) => {}
                Ok(Step::Stop) => break,
                Err(err) => return (Err(err), deltas),
            }
        }
        let tail = state.flush();
        if !tail.is_empty() {
            deltas.push(tail);
        }
        (state.finish(), deltas)
    }

    const OK_STREAM: &str = "\
data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"gpt-4.1-mini\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}

data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Bon\"},\"finish_reason\":null}]}

: OPENROUTER PROCESSING

data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"jour\\nle monde\"},\"finish_reason\":null}]}

data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}

data: {\"id\":\"chatcmpl-1\",\"choices\":[],\"usage\":{\"prompt_tokens\":300,\"completion_tokens\":6,\"total_tokens\":306}}

data: [DONE]

";

    #[test]
    fn streams_content_deltas() {
        let (res, deltas) = run_stream(OK_STREAM);
        assert_eq!(deltas, vec!["Bon", "jour\nle monde"]);
        assert_eq!(res.ok().as_deref(), Some("Bonjour\nle monde"));
    }

    #[test]
    fn stream_without_done_marker_is_accepted() {
        let raw = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n";
        let (res, _) = run_stream(raw);
        assert_eq!(res.ok().as_deref(), Some("Hi"));
    }

    #[test]
    fn reasoning_content_field_is_ignored() {
        let raw = "\
data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"let me think\",\"content\":null}}]}

data: {\"choices\":[{\"delta\":{\"content\":\"Answer\"}}]}

data: [DONE]

";
        let (res, deltas) = run_stream(raw);
        assert_eq!(deltas, vec!["Answer"]);
        assert_eq!(res.ok().as_deref(), Some("Answer"));
    }

    #[test]
    fn mid_stream_error_object_maps_to_a_short_message() {
        let raw = "\
data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}

data: {\"error\":{\"code\":502,\"message\":\"Provider returned error\"}}

";
        let (res, deltas) = run_stream(raw);
        assert_eq!(deltas, vec!["Hel"]);
        assert!(
            matches!(res, Err(TranslateError::Service { status: 502, ref message }) if message == "The service is unavailable")
        );
    }

    #[test]
    fn error_codes_map_to_statuses() {
        let e = |code: Value, msg: &str| {
            api_error(&json!({"error": {"code": code, "message": msg, "type": "t"}}))
        };
        assert!(
            matches!(e(json!("invalid_api_key"), "Incorrect API key provided: sk-xxx"),
            TranslateError::Service { status: 401, ref message } if message == "Invalid API key")
        );
        assert!(matches!(e(json!("rate_limit_exceeded"), "slow down"),
            TranslateError::Service { status: 429, ref message } if message == "Rate limited"));
        assert!(
            matches!(e(json!("insufficient_quota"), "You exceeded your current quota"),
            TranslateError::Service { status: 429, ref message } if message == "Quota exceeded")
        );
        assert!(matches!(
            e(json!(null), "boom"),
            TranslateError::Service { status: 500, .. }
        ));
    }

    #[test]
    fn content_filter_is_reported() {
        let raw = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"content_filter\"}]}\n\ndata: [DONE]\n\n";
        let (res, _) = run_stream(raw);
        assert!(matches!(
            res,
            Err(TranslateError::Service { status: 200, .. })
        ));
    }

    #[test]
    fn empty_answer_is_an_error() {
        let (res, _) = run_stream("data: [DONE]\n\n");
        assert!(matches!(res, Err(TranslateError::Parse(_))));
    }

    #[test]
    fn keepalive_junk_is_skipped() {
        let mut s = StreamState::default();
        assert!(matches!(s.on_event(&ev("")), Ok(Step::Continue)));
        assert!(matches!(
            s.on_event(&ev("{\"choices\":[]}")),
            Ok(Step::Continue)
        ));
        assert!(matches!(s.on_event(&ev("PROCESSING")), Ok(Step::Continue)));
    }

    #[test]
    fn think_block_is_removed_from_the_stream() {
        let raw = "\
data: {\"choices\":[{\"delta\":{\"content\":\"<think>\\nThe user wants\"}}]}

data: {\"choices\":[{\"delta\":{\"content\":\" French.\\n</thi\"}}]}

data: {\"choices\":[{\"delta\":{\"content\":\"nk>\\n\\nBonjour\"}}]}

data: {\"choices\":[{\"delta\":{\"content\":\" le monde\"},\"finish_reason\":\"stop\"}]}

data: [DONE]

";
        let (res, deltas) = run_stream(raw);
        assert_eq!(deltas, vec!["Bonjour", " le monde"]);
        assert_eq!(res.ok().as_deref(), Some("Bonjour le monde"));
    }

    #[test]
    fn plain_json_completion_is_parsed() {
        let body = r#"{"id":"c","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"Hola mundo"},"finish_reason":"stop"}],"usage":{"total_tokens":9}}"#;
        assert_eq!(
            parse_completion_json(body).ok().as_deref(),
            Some("Hola mundo")
        );
        let parts = r#"{"choices":[{"message":{"content":[{"type":"text","text":"A"},{"type":"text","text":"B"}]}}]}"#;
        assert_eq!(parse_completion_json(parts).ok().as_deref(), Some("AB"));
        let think = r#"{"choices":[{"message":{"content":"<think>x</think>\n\nHola"}}]}"#;
        assert_eq!(parse_completion_json(think).ok().as_deref(), Some("Hola"));
        assert!(matches!(
            parse_completion_json("oops"),
            Err(TranslateError::Parse(_))
        ));
        assert!(matches!(
            parse_completion_json(r#"{"choices":[]}"#),
            Err(TranslateError::Parse(_))
        ));
        assert!(matches!(
            parse_completion_json(r#"{"error":{"message":"bad","code":"invalid_api_key"}}"#),
            Err(TranslateError::Service { status: 401, .. })
        ));
    }

    // ---- ThinkFilter -----------------------------------------------------------------------

    fn filter_all(parts: &[&str]) -> String {
        let mut f = ThinkFilter::default();
        let mut out = String::new();
        for p in parts {
            out.push_str(&f.push(p));
        }
        out.push_str(&f.finish());
        out
    }

    #[test]
    fn think_filter_passes_normal_text_through() {
        assert_eq!(filter_all(&["Hello ", "world"]), "Hello world");
        assert_eq!(
            filter_all(&["  leading space kept"]),
            "  leading space kept"
        );
        // A tag later in the text is the translation's business.
        assert_eq!(filter_all(&["use the <think> tag"]), "use the <think> tag");
        assert_eq!(filter_all(&["<thanks> a lot"]), "<thanks> a lot");
        assert_eq!(filter_all(&[]), "");
        assert_eq!(filter_all(&["  \n"]), "");
    }

    #[test]
    fn think_filter_removes_a_leading_block_for_any_split() {
        let full = "<think>reasoning here</think>\n\nBonjour le monde";
        for i in 0..=full.len() {
            if !full.is_char_boundary(i) {
                continue;
            }
            assert_eq!(
                filter_all(&[&full[..i], &full[i..]]),
                "Bonjour le monde",
                "split at {i}"
            );
        }
        let bytes: Vec<String> = full.chars().map(|c| c.to_string()).collect();
        let parts: Vec<&str> = bytes.iter().map(String::as_str).collect();
        assert_eq!(filter_all(&parts), "Bonjour le monde");
    }

    #[test]
    fn think_filter_handles_multibyte_text_and_edge_cases() {
        let full = "  <think>思考中……\n还在想</think>你好，世界";
        let chars: Vec<String> = full.chars().map(|c| c.to_string()).collect();
        let parts: Vec<&str> = chars.iter().map(String::as_str).collect();
        assert_eq!(filter_all(&parts), "你好，世界");
        // The block ends the output: nothing is left to show.
        assert_eq!(filter_all(&["<think>only thoughts</think>"]), "");
        // Never closed: nothing is shown either.
        assert_eq!(filter_all(&["<think>cut off"]), "");
        // A held-back partial tag is released at the end.
        assert_eq!(filter_all(&["<th"]), "<th");
        assert_eq!(filter_all(&["<think"]), "<think");
    }

    // ---- request construction and URLs ---------------------------------------------------

    #[test]
    fn official_request_uses_max_completion_tokens_and_temperature_zero() {
        let b = request_body("gpt-4.1-mini", "SYS", "USR", "hello", true);
        assert_eq!(b["model"], "gpt-4.1-mini");
        assert_eq!(b["stream"], true);
        assert_eq!(b["temperature"], 0);
        assert_eq!(b["max_completion_tokens"], 256 + 6);
        assert!(b.get("max_tokens").is_none());
        assert_eq!(
            b["messages"][0],
            json!({"role": "system", "content": "SYS"})
        );
        assert_eq!(b["messages"][1], json!({"role": "user", "content": "USR"}));
    }

    #[test]
    fn official_reasoning_models_get_no_temperature_and_extra_room() {
        for m in ["gpt-5", "gpt-5-mini", "o3-mini", "o4-mini", "o1"] {
            let b = request_body(m, "S", "U", "hello", true);
            assert!(b.get("temperature").is_none(), "{m}");
            assert_eq!(b["max_completion_tokens"], 256 + 6 + 4096, "{m}");
        }
        // The same name on a third-party server is just a model name.
        let b = request_body("gpt-5", "S", "U", "hello", false);
        assert_eq!(b["temperature"], 0);
        assert_eq!(b["max_tokens"], 256 + 6);
    }

    #[test]
    fn compatible_servers_get_max_tokens() {
        let b = request_body("qwen3:8b", "S", "U", "hello", false);
        assert_eq!(b["max_tokens"], 256 + 6);
        assert!(b.get("max_completion_tokens").is_none());
        assert_eq!(b["temperature"], 0);
    }

    #[test]
    fn endpoint_urls() {
        assert_eq!(
            endpoint_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url("http://localhost:11434"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url("http://localhost:11434/"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url("https://openrouter.ai/api/v1"),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url("https://api.deepseek.com"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url("https://h/v1/chat/completions"),
            "https://h/v1/chat/completions"
        );
    }

    #[test]
    fn key_is_required_only_for_openai_itself() {
        let http = reqwest::Client::new();
        let mut cfg = crate::default_engines().remove(3);
        assert!(matches!(
            OpenAi::new("o".into(), "O".into(), &cfg, http.clone()),
            Err(TranslateError::Config(_))
        ));
        cfg.base_url = Some("https://api.openai.com/v1".into());
        assert!(OpenAi::new("o".into(), "O".into(), &cfg, http.clone()).is_err());
        cfg.base_url = Some("http://localhost:11434/v1".into());
        let o = OpenAi::new("o".into(), "O".into(), &cfg, http.clone())
            .unwrap_or_else(|e| panic!("Ollama needs no key: {e}"));
        assert!(!o.official);
        assert_eq!(o.model, "gpt-4.1-mini");
        cfg.base_url = None;
        cfg.api_key = Some("sk-x".into());
        let o = OpenAi::new("o".into(), "O".into(), &cfg, http)
            .unwrap_or_else(|e| panic!("builds: {e}"));
        assert!(o.official);
        assert_eq!(o.url, "https://api.openai.com/v1/chat/completions");
    }
}
