//! Engines against a local HTTP mock: request shape, streaming over a real socket, error
//! mapping, retries, the Bing token cache, the cache wrapper, and cancellation.

mod common;

use std::time::{Duration, Instant};

use common::{Mock, Recorded, Reply};
use glint_translate::{
    build, build_cached, default_engines, http_client, translate_all, Cache, EngineConfig,
    EngineEvent, TranslateError, TranslateRequest, Translator,
};
use md5::{Digest as _, Md5};
use sha2::Sha256;

fn cfg(kind: &str, base: &str) -> EngineConfig {
    let mut c = default_engines()
        .into_iter()
        .find(|e| e.kind == kind)
        .expect("kind exists");
    c.enabled = true;
    c.base_url = Some(base.to_string());
    c
}

fn engine(c: &EngineConfig) -> Box<dyn Translator> {
    build(c, &http_client()).expect("engine builds")
}

/// Run one translation and also collect the deltas it streamed.
async fn run(
    t: &dyn Translator,
    req: &TranslateRequest,
) -> (Result<String, TranslateError>, Vec<String>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let res = t.translate(req, tx).await;
    let mut deltas = Vec::new();
    while let Ok(d) = rx.try_recv() {
        deltas.push(d);
    }
    (res, deltas)
}

fn req(text: &str, source: &str, target: &str) -> TranslateRequest {
    TranslateRequest::new(text, source, target)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---- Anthropic ------------------------------------------------------------------------------

fn anthropic_sse(deltas: &[&str]) -> String {
    let mut s = String::from(
        "event: message_start\r\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-haiku-5-5\",\"usage\":{\"input_tokens\":9,\"output_tokens\":1}}}\r\n\r\n\
         event: content_block_start\r\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\r\n\r\n\
         event: ping\r\ndata: {\"type\": \"ping\"}\r\n\r\n",
    );
    for d in deltas {
        let data = serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":d}});
        s.push_str(&format!(
            "event: content_block_delta\r\ndata: {data}\r\n\r\n"
        ));
    }
    s.push_str(
        "event: content_block_stop\r\ndata: {\"type\":\"content_block_stop\",\"index\":0}\r\n\r\n\
         event: message_delta\r\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":7}}\r\n\r\n\
         event: message_stop\r\ndata: {\"type\":\"message_stop\"}\r\n\r\n",
    );
    s
}

fn anthropic_cfg(base: &str) -> EngineConfig {
    let mut c = cfg("anthropic", base);
    c.api_key = Some("sk-ant-test".into());
    c
}

#[tokio::test]
async fn anthropic_streams_over_the_wire_and_sends_the_documented_request() {
    let stream = anthropic_sse(&["你好，", "世界。\n", "第二行"]);
    let mock = Mock::start(move |_, _| {
        // Cut mid-line and mid-character, so chunk boundaries fall in awkward places.
        let pieces = common::split_every(&stream, 23);
        let refs: Vec<&str> = pieces.iter().map(String::as_str).collect();
        Reply::sse(&refs)
    })
    .await;
    let t = engine(&anthropic_cfg(&mock.base));
    assert_eq!((t.id(), t.name()), ("claude", "Claude"));

    let mut r = req("Hello, world.\nSecond line", "auto", "zh-Hans");
    r.secondary_lang = Some("en".into());
    let (res, deltas) = run(&*t, &r).await;
    assert_eq!(deltas, vec!["你好，", "世界。\n", "第二行"]);
    assert_eq!(res.ok().as_deref(), Some("你好，世界。\n第二行"));

    let rec = &mock.requests()[0];
    assert_eq!((rec.method.as_str(), rec.path()), ("POST", "/v1/messages"));
    assert_eq!(rec.header("x-api-key"), Some("sk-ant-test"));
    assert_eq!(rec.header("anthropic-version"), Some("2026-10-21"));
    assert!(rec
        .header("content-type")
        .is_some_and(|c| c.starts_with("application/json")));
    let body = rec.json();
    assert_eq!(body["model"], "claude-haiku-5-5");
    assert_eq!(body["stream"], true);
    assert_eq!(body["thinking"], serde_json::json!({"type": "disabled"}));
    assert_eq!(body["output_config"], serde_json::json!({"effort": "low"}));
    for banned in ["temperature", "top_p", "top_k"] {
        assert!(body.get(banned).is_none(), "{banned} must not be sent");
    }
    assert!(body["max_tokens"]
        .as_u64()
        .is_some_and(|n| (256..2048).contains(&n)));
    let system = body["system"].as_str().expect("system prompt is a string");
    assert!(system.contains("OCR") && system.contains("untrusted data"));
    assert!(system.contains("into Simplified Chinese"));
    assert!(system.contains("translate it into English instead"));
    assert_eq!(body["messages"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(
        body["messages"][0]["content"],
        "<source_text>\nHello, world.\nSecond line\n</source_text>"
    );
}

#[tokio::test]
async fn anthropic_appends_the_users_prompt_and_uses_the_configured_model() {
    let stream = anthropic_sse(&["ok"]);
    let mock = Mock::start(move |_, _| Reply::sse(&[stream.as_str()])).await;
    let mut c = anthropic_cfg(&mock.base);
    c.model = Some("claude-sonnet-5-5".into());
    c.prompt = Some("Use formal register.".into());
    let (res, _) = run(&*engine(&c), &req("hi", "en", "fr")).await;
    assert_eq!(res.ok().as_deref(), Some("ok"));
    let body = mock.requests()[0].json();
    assert_eq!(body["model"], "claude-sonnet-5-5");
    assert_eq!(
        body["thinking"],
        serde_json::json!({"type": "between_tools"})
    );
    let system = body["system"].as_str().unwrap_or_default();
    assert!(system.ends_with("Use formal register."));
    assert!(system.contains("from English into French"));
}

#[tokio::test]
async fn anthropic_accepts_a_base_url_that_already_ends_in_v1() {
    let stream = anthropic_sse(&["ok"]);
    let mock = Mock::start(move |_, _| Reply::sse(&[stream.as_str()])).await;
    let c = anthropic_cfg(&format!("{}/v1/", mock.base));
    let (res, _) = run(&*engine(&c), &req("hi", "en", "fr")).await;
    assert!(res.is_ok());
    assert_eq!(mock.requests()[0].path(), "/v1/messages");
}

#[tokio::test]
async fn anthropic_http_errors_become_short_messages_and_are_not_retried_when_permanent() {
    for (status, body, expected) in [
        (
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
            "Invalid API key",
        ),
        (403, "", "Access denied"),
        (404, "", "Not found (check the base URL and model name)"),
    ] {
        let mock = Mock::start(move |_, _| Reply::json(status, body)).await;
        let (res, deltas) = run(&*engine(&anthropic_cfg(&mock.base)), &req("hi", "en", "fr")).await;
        match res {
            Err(TranslateError::Service { status: s, message }) => {
                assert_eq!(s, status);
                assert_eq!(message, expected);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(deltas.is_empty());
        assert_eq!(mock.count(), 1, "{status} must not be retried");
    }
}

#[tokio::test]
async fn bad_request_shows_the_services_reason_but_not_the_whole_body() {
    let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"model: claude-nope is not a valid model"},"request_id":"req_1"}"#;
    let mock = Mock::start(move |_, _| Reply::json(400, body)).await;
    let (res, _) = run(&*engine(&anthropic_cfg(&mock.base)), &req("hi", "en", "fr")).await;
    match res {
        Err(TranslateError::Service {
            status: 400,
            message,
        }) => {
            assert_eq!(
                message,
                "Bad request: model: claude-nope is not a valid model"
            );
            assert!(!message.contains("request_id"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn a_503_is_retried_once_and_then_succeeds() {
    let stream = anthropic_sse(&["done"]);
    let mock = Mock::start(move |_, i| {
        if i == 0 {
            Reply::json(
                503,
                r#"{"type":"error","error":{"type":"api_error","message":"down"}}"#,
            )
        } else {
            Reply::sse(&[stream.as_str()])
        }
    })
    .await;
    let started = Instant::now();
    let (res, deltas) = run(&*engine(&anthropic_cfg(&mock.base)), &req("hi", "en", "fr")).await;
    assert_eq!(res.ok().as_deref(), Some("done"));
    assert_eq!(deltas, vec!["done"]);
    assert_eq!(mock.count(), 2);
    assert!(
        started.elapsed() >= Duration::from_millis(350),
        "a short pause precedes the retry"
    );
}

#[tokio::test]
async fn a_429_is_not_retried_and_carries_retry_after() {
    let mock = Mock::start(|_, _| Reply::json(429, "{}").with_header("Retry-After", "7")).await;
    let started = Instant::now();
    let (res, _) = run(&*engine(&anthropic_cfg(&mock.base)), &req("hi", "en", "fr")).await;
    match res {
        Err(e @ TranslateError::RateLimited { .. }) => {
            assert_eq!(e.code(), "rate_limited");
            assert_eq!(e.retry_after(), Some(Duration::from_secs(7)));
            assert_eq!(e.user_message(), "Rate limited");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        mock.count(),
        1,
        "asking again right away would only extend the block"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "no waiting before giving up"
    );
}

#[tokio::test]
async fn a_rate_limited_engine_cools_down_and_recovers() {
    let stream = anthropic_sse(&["ok"]);
    let mock = Mock::start(move |_, i| {
        if i == 0 {
            Reply::json(429, "{}").with_header("Retry-After", "1")
        } else {
            Reply::sse(&[stream.as_str()])
        }
    })
    .await;
    let cache = Cache::default();
    let t = build_cached(&anthropic_cfg(&mock.base), &http_client(), &cache).expect("builds");

    let (res, _) = run(&*t, &req("one", "en", "fr")).await;
    assert!(
        matches!(&res, Err(e) if e.code() == "rate_limited" && e.retry_after() == Some(Duration::from_secs(1))),
        "{res:?}"
    );

    // During the cool-down the engine is skipped without a request.
    let (res, _) = run(&*t, &req("two", "en", "fr")).await;
    match &res {
        Err(e) => {
            assert_eq!(e.code(), "rate_limited");
            assert!(e.retry_after().is_some_and(|d| d <= Duration::from_secs(1)));
        }
        Ok(_) => panic!("should still be cooling down"),
    }
    assert_eq!(mock.count(), 1);

    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let (res, _) = run(&*t, &req("three", "en", "fr")).await;
    assert_eq!(res.ok().as_deref(), Some("ok"));
    assert_eq!(mock.count(), 2);
}

#[tokio::test]
async fn at_most_one_retry_even_if_the_service_keeps_failing() {
    let mock = Mock::start(|_, _| Reply::json(503, "{}")).await;
    let (res, _) = run(&*engine(&anthropic_cfg(&mock.base)), &req("hi", "en", "fr")).await;
    match res {
        Err(TranslateError::Service {
            status: 503,
            message,
        }) => assert_eq!(message, "The service is unavailable"),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(mock.count(), 2);

    let mock = Mock::start(|_, _| Reply::json(429, "{}")).await;
    let (res, _) = run(&*engine(&anthropic_cfg(&mock.base)), &req("hi", "en", "fr")).await;
    assert!(
        matches!(res, Err(TranslateError::RateLimited { ref message, retry_after: None }) if message == "Rate limited")
    );
    assert_eq!(mock.count(), 1, "a 429 is not retried");
}

#[tokio::test]
async fn an_overloaded_error_before_any_output_is_retried() {
    let good = anthropic_sse(&["fine"]);
    let mock = Mock::start(move |_, i| {
        if i == 0 {
            Reply::sse(&["event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n"])
        } else {
            Reply::sse(&[good.as_str()])
        }
    })
    .await;
    let (res, deltas) = run(&*engine(&anthropic_cfg(&mock.base)), &req("hi", "en", "fr")).await;
    assert_eq!(res.ok().as_deref(), Some("fine"));
    assert_eq!(deltas, vec!["fine"]);
    assert_eq!(mock.count(), 2);
}

#[tokio::test]
async fn no_retry_once_partial_output_has_streamed() {
    let mock = Mock::start(|_, _| {
        Reply::sse(&[
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n",
            "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n",
        ])
    })
    .await;
    let (res, deltas) = run(&*engine(&anthropic_cfg(&mock.base)), &req("hi", "en", "fr")).await;
    assert!(matches!(
        res,
        Err(TranslateError::Service { status: 529, .. })
    ));
    assert_eq!(deltas, vec!["Hel"]);
    assert_eq!(
        mock.count(),
        1,
        "the partial text is already on screen, so no second attempt"
    );
}

#[tokio::test]
async fn a_refusal_is_an_error_and_is_not_retried() {
    let mock = Mock::start(|_, _| {
        Reply::sse(&["event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"])
    })
    .await;
    let (res, _) = run(&*engine(&anthropic_cfg(&mock.base)), &req("hi", "en", "fr")).await;
    assert!(
        matches!(res, Err(TranslateError::Service { status: 200, ref message }) if message.contains("declined"))
    );
    assert_eq!(mock.count(), 1);
}

#[tokio::test]
async fn a_server_that_ignores_stream_true_still_works() {
    let mock = Mock::start(|_, _| {
        Reply::json(200, r#"{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"text","text":"Bonjour"}],"stop_reason":"end_turn"}"#)
    })
    .await;
    let (res, deltas) = run(&*engine(&anthropic_cfg(&mock.base)), &req("hi", "en", "fr")).await;
    assert_eq!(res.ok().as_deref(), Some("Bonjour"));
    assert_eq!(deltas, vec!["Bonjour"]);
}

#[tokio::test]
async fn connection_failures_are_retried_once_then_reported_plainly() {
    // Bind and drop a listener to get a port nothing listens on.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        l.local_addr().expect("addr").port()
    };
    let started = Instant::now();
    let (res, _) = run(
        &*engine(&anthropic_cfg(&format!("http://127.0.0.1:{port}"))),
        &req("hi", "en", "fr"),
    )
    .await;
    match res {
        Err(TranslateError::Network(m)) => assert_eq!(m, "Could not connect to the server"),
        other => panic!("unexpected {other:?}"),
    }
    assert!(
        started.elapsed() >= Duration::from_millis(350),
        "one retry happened"
    );
}

// ---- OpenAI-compatible ----------------------------------------------------------------------

fn openai_sse(deltas: &[&str]) -> String {
    let mut s = String::from(
        "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}\n\n",
    );
    for d in deltas {
        let data = serde_json::json!({"id":"c1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":d},"finish_reason":null}]});
        s.push_str(&format!("data: {data}\n\n"));
    }
    s.push_str("data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n");
    s
}

#[tokio::test]
async fn openai_compatible_streaming_and_request_shape() {
    let stream = openai_sse(&["Bon", "jour"]);
    let mock = Mock::start(move |_, _| {
        let pieces = common::split_every(&stream, 17);
        let refs: Vec<&str> = pieces.iter().map(String::as_str).collect();
        Reply::sse(&refs)
    })
    .await;
    // A base URL with no path, as people type for Ollama: /v1 is added.
    let mut c = cfg("openai", &mock.base);
    c.model = Some("qwen3:8b".into());
    c.api_key = Some("sk-local".into());
    c.prompt = Some("Be concise.".into());
    let t = engine(&c);
    assert_eq!((t.id(), t.name()), ("openai", "OpenAI"));
    let (res, deltas) = run(&*t, &req("Hello", "auto", "fr")).await;
    assert_eq!(deltas, vec!["Bon", "jour"]);
    assert_eq!(res.ok().as_deref(), Some("Bonjour"));

    let rec = &mock.requests()[0];
    assert_eq!(rec.path(), "/v1/chat/completions");
    assert_eq!(rec.header("authorization"), Some("Bearer sk-local"));
    let body = rec.json();
    assert_eq!(body["model"], "qwen3:8b");
    assert_eq!(body["stream"], true);
    assert_eq!(body["temperature"], 0);
    assert!(body["max_tokens"].as_u64().is_some());
    assert!(body.get("max_completion_tokens").is_none());
    assert_eq!(body["messages"][0]["role"], "system");
    assert!(body["messages"][0]["content"]
        .as_str()
        .is_some_and(|s| s.ends_with("Be concise.")));
    assert_eq!(
        body["messages"][1]["content"],
        "<source_text>\nHello\n</source_text>"
    );
}

#[tokio::test]
async fn openai_without_a_key_sends_no_authorization_header() {
    let stream = openai_sse(&["ok"]);
    let mock = Mock::start(move |_, _| Reply::sse(&[stream.as_str()])).await;
    let (res, _) = run(
        &*engine(&cfg("openai", &format!("{}/v1", mock.base))),
        &req("hi", "en", "fr"),
    )
    .await;
    assert!(res.is_ok());
    assert!(mock.requests()[0].header("authorization").is_none());
}

#[tokio::test]
async fn openai_hides_a_think_block_from_a_local_reasoning_model() {
    let stream = openai_sse(&[
        "<think>\nUser wants French",
        ".\n</thi",
        "nk>\n\nBonjour",
        " le monde",
    ]);
    let mock = Mock::start(move |_, _| Reply::sse(&[stream.as_str()])).await;
    let (res, deltas) = run(&*engine(&cfg("openai", &mock.base)), &req("hi", "en", "fr")).await;
    assert_eq!(deltas.concat(), "Bonjour le monde");
    assert_eq!(res.ok().as_deref(), Some("Bonjour le monde"));
}

#[tokio::test]
async fn openai_errors_are_short() {
    let mock = Mock::start(|_, _| {
        Reply::json(429, r#"{"error":{"message":"You exceeded your current quota","type":"insufficient_quota","code":"insufficient_quota"}}"#)
    })
    .await;
    let (res, _) = run(&*engine(&cfg("openai", &mock.base)), &req("hi", "en", "fr")).await;
    assert!(
        matches!(res, Err(TranslateError::Service { status: 429, ref message }) if message == "Quota exceeded")
    );
    assert_eq!(mock.count(), 1, "an exhausted quota is not retried");

    let mock = Mock::start(|_, _| Reply::json(401, r#"{"error":{"message":"Incorrect API key provided: sk-abc","code":"invalid_api_key"}}"#)).await;
    let (res, _) = run(&*engine(&cfg("openai", &mock.base)), &req("hi", "en", "fr")).await;
    match res {
        Err(TranslateError::Service {
            status: 401,
            message,
        }) => assert_eq!(message, "Invalid API key"),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn openai_plain_json_fallback() {
    let mock = Mock::start(|_, _| {
        Reply::json(200, r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"Hola"},"finish_reason":"stop"}]}"#)
    })
    .await;
    let (res, deltas) = run(&*engine(&cfg("openai", &mock.base)), &req("hi", "en", "es")).await;
    assert_eq!(res.ok().as_deref(), Some("Hola"));
    assert_eq!(deltas, vec!["Hola"]);
}

// ---- Google ---------------------------------------------------------------------------------

// A real response, recorded from translate.googleapis.com.
const GOOGLE_MULTILINE: &str = r#"[[["你好世界。\n","Hello, world.\n",null,null,10,null,null,null,[[null,true]]],["这是第二行。","This is the second line. ",null,null,3,null,null,[[]],[[["6ffafab0da7e640be86ac09d0d5e539c","en_zh_2023q1.md"]]]],["请保留它。\n\n","Please keep it.\n\n",null,null,3,null,null,[[]],[[["6ffafab0da7e640be86ac09d0d5e539c","en_zh_2023q1.md"],true]]],["- 第一项\n","- item one\n",null,null,3,null,null,[[]],[[["6ffafab0da7e640be86ac09d0d5e539c","en_zh_2023q1.md"],true]]],["- 第二项","- item two",null,null,3,null,null,[[]],[[["6ffafab0da7e640be86ac09d0d5e539c","en_zh_2023q1.md"]]]]],null,"en",null,null,null,1,[],[["en"],null,[1],["en"]]]"#;

#[tokio::test]
async fn google_free_endpoint_request_and_result() {
    let mock = Mock::start(|_, _| Reply::json(200, GOOGLE_MULTILINE)).await;
    let t = engine(&cfg("google", &mock.base));
    let text = "Hello, world.\nThis is the second line. Please keep it.\n\n- item one\n- item two";
    let (res, deltas) = run(&*t, &req(text, "auto", "zh-Hans")).await;
    let expected = "你好世界。\n这是第二行。请保留它。\n\n- 第一项\n- 第二项";
    assert_eq!(res.ok().as_deref(), Some(expected));
    assert_eq!(
        deltas,
        vec![expected.to_string()],
        "non-streaming engines send one delta"
    );

    let rec = &mock.requests()[0];
    assert_eq!(
        (rec.method.as_str(), rec.path()),
        ("POST", "/translate_a/single")
    );
    assert_eq!(rec.query_value("client").as_deref(), Some("gtx"));
    assert_eq!(rec.query_value("sl").as_deref(), Some("auto"));
    assert_eq!(rec.query_value("tl").as_deref(), Some("zh-CN"));
    assert_eq!(rec.query_value("dt").as_deref(), Some("t"));
    assert_eq!(
        rec.form_value("q"),
        text,
        "the text travels in the body, newlines intact"
    );
    assert!(
        rec.header("user-agent")
            .is_some_and(|ua| ua.starts_with("Mozilla/5.0")),
        "free endpoint should send a browser user-agent"
    );
}

#[tokio::test]
async fn google_flips_to_the_secondary_language_when_the_text_is_already_in_the_target() {
    let mock = Mock::start(|_, i| {
        if i == 0 {
            // The text turned out to be Chinese.
            Reply::json(
                200,
                r#"[[["你好","你好",null,null,5]],null,"zh-CN",null,null,null,1,[]]"#,
            )
        } else {
            Reply::json(
                200,
                r#"[[["Hello","你好",null,null,10]],null,"zh-CN",null,null,null,1,[]]"#,
            )
        }
    })
    .await;
    let mut r = req("你好", "auto", "zh-Hans");
    r.secondary_lang = Some("en".into());
    let (res, deltas) = run(&*engine(&cfg("google", &mock.base)), &r).await;
    assert_eq!(res.ok().as_deref(), Some("Hello"));
    assert_eq!(deltas, vec!["Hello"]);
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].query_value("tl").as_deref(), Some("zh-CN"));
    assert_eq!(reqs[1].query_value("tl").as_deref(), Some("en"));
}

#[tokio::test]
async fn google_official_api_when_a_key_is_set() {
    let mock = Mock::start(|_, _| {
        Reply::json(200, r#"{"data":{"translations":[{"translatedText":"Bonjour","detectedSourceLanguage":"en"}]}}"#)
    })
    .await;
    let mut c = cfg("google", &mock.base);
    c.api_key = Some("AIza-test".into());
    let (res, _) = run(&*engine(&c), &req("Hello", "en", "fr")).await;
    assert_eq!(res.ok().as_deref(), Some("Bonjour"));
    let rec = &mock.requests()[0];
    assert_eq!(rec.path(), "/language/translate/v2");
    assert_eq!(rec.header("x-goog-api-key"), Some("AIza-test"));
    let body = rec.json();
    assert_eq!(body["q"], "Hello");
    assert_eq!(body["source"], "en");
    assert_eq!(body["target"], "fr");
    assert_eq!(body["format"], "text");
    assert!(
        !rec.target.contains("AIza"),
        "the key must not appear in the URL"
    );
}

#[tokio::test]
async fn unsupported_languages_fail_before_any_request() {
    let mock = Mock::start(|_, _| Reply::json(200, "[]")).await;
    let (res, _) = run(
        &*engine(&cfg("google", &mock.base)),
        &req("hi", "auto", "tlh"),
    )
    .await;
    assert!(matches!(res, Err(TranslateError::Config(ref m)) if m.contains("tlh")));
    let (res, _) = run(
        &*engine(&cfg("google", &mock.base)),
        &req("hi", "auto", "auto"),
    )
    .await;
    assert!(matches!(res, Err(TranslateError::Config(_))));
    assert_eq!(mock.count(), 0);
}

// ---- Bing -----------------------------------------------------------------------------------

const FAKE_JWT: &str = "eyJhbGciOiJFUzI1NiIsImtpZCI6ImtleTEifQ.eyJzdWIiOiJ4In0.c2lnbmF0dXJl";

fn bing_handler(rec: &Recorded, _i: usize) -> Reply {
    if rec.path() == "/translate/auth" {
        Reply::text(200, FAKE_JWT)
    } else {
        Reply::json(
            200,
            r#"[{"detectedLanguage":{"language":"en","score":1.0},"translations":[{"text":"你好","to":"zh-Hans"}]}]"#,
        )
    }
}

#[tokio::test]
async fn bing_fetches_a_token_once_and_reuses_it() {
    let mock = Mock::start(bing_handler).await;
    let t = engine(&cfg("bing", &mock.base));
    for _ in 0..3 {
        let (res, deltas) = run(&*t, &req("Hello", "auto", "zh-Hans")).await;
        assert_eq!(res.ok().as_deref(), Some("你好"));
        assert_eq!(deltas, vec!["你好"]);
    }
    let reqs = mock.requests();
    assert_eq!(
        reqs.iter()
            .filter(|r| r.path() == "/translate/auth")
            .count(),
        1,
        "token is cached"
    );
    let translate: Vec<_> = reqs.iter().filter(|r| r.path() == "/translate").collect();
    assert_eq!(translate.len(), 3);
    let first = translate[0];
    assert_eq!(first.method, "POST");
    let bearer = format!("Bearer {FAKE_JWT}");
    assert_eq!(first.header("authorization"), Some(bearer.as_str()));
    assert_eq!(first.query_value("api-version").as_deref(), Some("3.0"));
    assert_eq!(first.query_value("to").as_deref(), Some("zh-Hans"));
    assert!(first.query_value("from").is_none(), "auto means no `from`");
    assert_eq!(first.json(), serde_json::json!([{"Text": "Hello"}]));
}

#[tokio::test]
async fn bing_concurrent_requests_share_one_token_fetch() {
    let mock = Mock::start(bing_handler).await;
    let t = std::sync::Arc::new(engine(&cfg("bing", &mock.base)));
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let t = t.clone();
        tasks.push(tokio::spawn(async move {
            run(&**t, &req("Hello", "en", "zh-Hans")).await.0.is_ok()
        }));
    }
    for task in tasks {
        assert!(task.await.unwrap_or(false));
    }
    assert_eq!(
        mock.requests()
            .iter()
            .filter(|r| r.path() == "/translate/auth")
            .count(),
        1
    );
}

#[tokio::test]
async fn bing_refreshes_a_rejected_token_and_retries() {
    let mock = Mock::start(|rec, i| {
        if rec.path() == "/translate/auth" {
            Reply::text(
                200,
                if i == 0 {
                    FAKE_JWT
                } else {
                    "eyJ0b2tlbi1udW1iZXItdHdvLWlzLWxvbmctZW5vdWdo.e30.c2ln"
                },
            )
        } else if rec.header("authorization") == Some(&format!("Bearer {FAKE_JWT}")) {
            Reply::json(401, r#"{"error":{"code":401000,"message":"expired"}}"#)
        } else {
            Reply::json(200, r#"[{"translations":[{"text":"Hola","to":"es"}]}]"#)
        }
    })
    .await;
    let (res, _) = run(
        &*engine(&cfg("bing", &mock.base)),
        &req("Hello", "en", "es"),
    )
    .await;
    assert_eq!(res.ok().as_deref(), Some("Hola"));
    let reqs = mock.requests();
    assert_eq!(
        reqs.iter()
            .filter(|r| r.path() == "/translate/auth")
            .count(),
        2,
        "a new token was fetched"
    );
    assert_eq!(reqs.iter().filter(|r| r.path() == "/translate").count(), 2);
    assert_eq!(
        reqs.iter()
            .find(|r| r.path() == "/translate")
            .and_then(|r| r.query_value("from"))
            .as_deref(),
        Some("en")
    );
}

#[tokio::test]
async fn bing_with_a_key_uses_the_azure_headers() {
    let mock =
        Mock::start(|_, _| Reply::json(200, r#"[{"translations":[{"text":"Hallo","to":"de"}]}]"#))
            .await;
    let mut c = cfg("bing", &mock.base);
    c.api_key = Some("azure-key".into());
    c.api_secret = Some("westeurope".into());
    let (res, _) = run(&*engine(&c), &req("Hello", "en", "de")).await;
    assert_eq!(res.ok().as_deref(), Some("Hallo"));
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1, "no Edge token is fetched with a key");
    assert_eq!(
        reqs[0].header("ocp-apim-subscription-key"),
        Some("azure-key")
    );
    assert_eq!(
        reqs[0].header("ocp-apim-subscription-region"),
        Some("westeurope")
    );
    assert!(reqs[0].header("authorization").is_none());
}

// ---- DeepL ----------------------------------------------------------------------------------

#[tokio::test]
async fn deepl_request_shape_and_errors() {
    let mock = Mock::start(|_, _| {
        Reply::json(
            200,
            r#"{"translations":[{"detected_source_language":"EN","text":"Hallo, Welt!"}]}"#,
        )
    })
    .await;
    let mut c = cfg("deepl", &mock.base);
    c.api_key = Some("secret-key:fx".into());
    let (res, _) = run(&*engine(&c), &req("Hello, world!", "auto", "de")).await;
    assert_eq!(res.ok().as_deref(), Some("Hallo, Welt!"));
    let rec = &mock.requests()[0];
    assert_eq!(rec.path(), "/v2/translate");
    assert_eq!(
        rec.header("authorization"),
        Some("DeepL-Auth-Key secret-key:fx")
    );
    assert_eq!(
        rec.json(),
        serde_json::json!({"text": ["Hello, world!"], "target_lang": "DE"})
    );

    let (res, _) = run(&*engine(&c), &req("Hello", "en", "pt")).await;
    assert!(res.is_ok());
    let body = mock.requests()[1].json();
    assert_eq!(body["source_lang"], "EN");
    assert_eq!(body["target_lang"], "PT-BR");

    for (status, expected) in [(403, "Invalid API key"), (456, "Quota exceeded")] {
        let mock = Mock::start(move |_, _| Reply::json(status, r#"{"message":"whatever"}"#)).await;
        let mut c = cfg("deepl", &mock.base);
        c.api_key = Some("k".into());
        let (res, _) = run(&*engine(&c), &req("Hello", "en", "de")).await;
        match res {
            Err(TranslateError::Service { status: s, message }) => {
                assert_eq!((s, message.as_str()), (status, expected))
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(mock.count(), 1);
    }
}

// ---- Youdao and Baidu: the signature on the wire ----------------------------------------------

#[tokio::test]
async fn youdao_sends_a_valid_v3_signature() {
    let mock = Mock::start(|_, _| {
        Reply::json(
            200,
            r#"{"errorCode":"0","query":"x","translation":["你好，世界"],"l":"en2zh-CHS"}"#,
        )
    })
    .await;
    let mut c = cfg("youdao", &mock.base);
    c.api_key = Some("my-app-key".into());
    c.api_secret = Some("my-app-secret".into());
    // 26 characters, so the truncation rule applies.
    let text = "Hello world, this is long.";
    assert_eq!(text.chars().count(), 26);
    let (res, deltas) = run(&*engine(&c), &req(text, "en", "zh-Hans")).await;
    assert_eq!(res.ok().as_deref(), Some("你好，世界"));
    assert_eq!(deltas, vec!["你好，世界"]);

    let rec = &mock.requests()[0];
    assert_eq!((rec.method.as_str(), rec.path()), ("POST", "/api"));
    for (k, v) in [
        ("q", text),
        ("from", "en"),
        ("to", "zh-CHS"),
        ("appKey", "my-app-key"),
        ("signType", "v3"),
    ] {
        assert_eq!(rec.form_value(k), v, "field {k}");
    }
    let (salt, curtime, sign) = (
        rec.form_value("salt"),
        rec.form_value("curtime"),
        rec.form_value("sign"),
    );
    assert!(!salt.is_empty());
    assert!(curtime.parse::<u64>().is_ok_and(|t| t > 1_700_000_000));
    // Recompute independently: appKey + head10 + length + tail10 + salt + curtime + secret.
    let input = format!("{}{}{}", &text[..10], 26, &text[16..]);
    let expected = hex(&Sha256::digest(
        format!("my-app-key{input}{salt}{curtime}my-app-secret").as_bytes(),
    ));
    assert_eq!(sign, expected);
}

#[tokio::test]
async fn youdao_error_codes_are_translated() {
    let mock = Mock::start(|_, _| Reply::json(200, r#"{"errorCode":"202"}"#)).await;
    let mut c = cfg("youdao", &mock.base);
    c.api_key = Some("k".into());
    c.api_secret = Some("s".into());
    let (res, _) = run(&*engine(&c), &req("hi", "en", "zh-Hans")).await;
    match res {
        Err(TranslateError::Service {
            status: 401,
            message,
        }) => assert!(message.contains("app secret")),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(mock.count(), 1);
}

#[tokio::test]
async fn baidu_sends_a_valid_md5_signature_and_joins_lines() {
    let mock = Mock::start(|_, _| {
        Reply::json(
            200,
            r#"{"from":"en","to":"zh","trans_result":[{"src":"Hello","dst":"你好"},{"src":"World","dst":"世界"}]}"#,
        )
    })
    .await;
    let mut c = cfg("baidu", &mock.base);
    c.api_key = Some("20151113000005349".into());
    c.api_secret = Some("osubCEzlGjzvw8qdQc41".into());
    let (res, _) = run(&*engine(&c), &req("Hello\nWorld", "auto", "zh-Hans")).await;
    assert_eq!(res.ok().as_deref(), Some("你好\n世界"));

    let rec = &mock.requests()[0];
    assert_eq!(
        (rec.method.as_str(), rec.path()),
        ("POST", "/api/trans/vip/translate")
    );
    for (k, v) in [
        ("q", "Hello\nWorld"),
        ("from", "auto"),
        ("to", "zh"),
        ("appid", "20151113000005349"),
    ] {
        assert_eq!(rec.form_value(k), v, "field {k}");
    }
    let salt = rec.form_value("salt");
    let expected = hex(&Md5::digest(
        format!("20151113000005349Hello\nWorld{salt}osubCEzlGjzvw8qdQc41").as_bytes(),
    ));
    assert_eq!(rec.form_value("sign"), expected);
}

#[tokio::test]
async fn baidu_error_codes_are_translated() {
    let mock = Mock::start(|_, _| {
        Reply::json(
            200,
            r#"{"error_code":"54003","error_msg":"Invalid Access Limit"}"#,
        )
    })
    .await;
    let mut c = cfg("baidu", &mock.base);
    c.api_key = Some("id".into());
    c.api_secret = Some("s".into());
    let (res, _) = run(&*engine(&c), &req("hi", "en", "zh-Hans")).await;
    assert!(
        matches!(res, Err(TranslateError::Service { status: 429, ref message }) if message == "Rate limited")
    );
}

// ---- Cache, translate_all, cancellation ----------------------------------------------------------

#[tokio::test]
async fn build_cached_answers_repeats_without_a_second_request() {
    let mock = Mock::start(|_, _| Reply::json(200, GOOGLE_MULTILINE)).await;
    let cache = Cache::default();
    let c = cfg("google", &mock.base);
    let t = build_cached(&c, &http_client(), &cache).expect("builds");
    let r = req("Hello, world.", "auto", "zh-Hans");
    let (first, d1) = run(&*t, &r).await;
    let (second, d2) = run(&*t, &r).await;
    assert_eq!(first.ok(), second.ok());
    assert_eq!(d1, d2);
    assert_eq!(mock.count(), 1);
    assert_eq!(cache.len(), 1);

    // Another engine instance sharing the cache and config also hits.
    let t2 = build_cached(&c, &http_client(), &cache).expect("builds");
    assert!(run(&*t2, &r).await.0.is_ok());
    assert_eq!(mock.count(), 1);

    // A different target language is a different entry.
    assert!(run(&*t, &req("Hello, world.", "auto", "ja"))
        .await
        .0
        .is_ok());
    assert_eq!(mock.count(), 2);
}

#[tokio::test]
async fn translate_all_runs_engines_together_and_reports_each_one() {
    let claude_stream = anthropic_sse(&["Bon", "jour"]);
    let claude = Mock::start(move |_, _| Reply::sse(&[claude_stream.as_str()])).await;
    let google =
        Mock::start(|_, _| Reply::json(200, r#"[[["Bonjour","Hello",null,null,10]],null,"en"]"#))
            .await;
    let broken = Mock::start(|_, _| Reply::json(401, "{}")).await;

    let mut broken_cfg = anthropic_cfg(&broken.base);
    broken_cfg.id = "claude-broken".into();
    let engines: Vec<Box<dyn Translator>> = vec![
        engine(&anthropic_cfg(&claude.base)),
        engine(&cfg("google", &google.base)),
        engine(&broken_cfg),
    ];

    let events = std::sync::Mutex::new(Vec::<EngineEvent>::new());
    translate_all(&engines, &req("Hello", "en", "fr"), |e| {
        events.lock().unwrap_or_else(|p| p.into_inner()).push(e)
    })
    .await;
    let events = events.into_inner().unwrap_or_default();
    let of =
        |id: &str| -> Vec<&EngineEvent> { events.iter().filter(|e| e.engine_id() == id).collect() };

    let c = of("claude");
    assert!(matches!(c[0], EngineEvent::Start { engine_name, .. } if engine_name == "Claude"));
    let streamed: String = c
        .iter()
        .filter_map(|e| {
            if let EngineEvent::Delta { delta, .. } = e {
                Some(delta.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(streamed, "Bonjour");
    assert!(matches!(c.last(), Some(EngineEvent::Done { text, .. }) if text == "Bonjour"));

    let g = of("google");
    assert_eq!(g.len(), 3, "start, one delta, done: {g:?}");
    assert!(matches!(g[2], EngineEvent::Done { text, .. } if text == "Bonjour"));

    let b = of("claude-broken");
    assert!(
        matches!(b.last(), Some(EngineEvent::Error { message, .. }) if message == "Invalid API key")
    );
}

#[tokio::test]
async fn dropping_translate_all_closes_the_connections() {
    let first = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Bon\"}}\n\n";
    let mock = Mock::start(move |_, _| Reply::sse_hanging(&[first])).await;
    let engines: Vec<Box<dyn Translator>> = vec![engine(&anthropic_cfg(&mock.base))];

    let got_delta = std::sync::atomic::AtomicBool::new(false);
    let request = req("Hello", "en", "fr");
    let work = translate_all(&engines, &request, |e| {
        if matches!(e, EngineEvent::Delta { .. }) {
            got_delta.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    });
    // Give the request time to start and stream its first delta, then drop the future.
    let res = tokio::time::timeout(Duration::from_millis(600), work).await;
    assert!(
        res.is_err(),
        "the engine is still waiting for the rest of the stream"
    );
    assert!(
        got_delta.load(std::sync::atomic::Ordering::SeqCst),
        "the first delta did arrive"
    );

    // The server sees the client hang up shortly after the drop.
    let deadline = Instant::now() + Duration::from_secs(3);
    while mock.disconnects() == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        mock.disconnects(),
        1,
        "dropping the future must close the in-flight request"
    );
    assert_eq!(mock.count(), 1, "and nothing is retried");
}
