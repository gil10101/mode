//! A tiny HTTP client trait (08 §3.2 `http`) with two implementations:
//! [`ReqwestClient`] (reqwest on rustls with the ring provider, the OS root
//! store, the system proxy) and [`FixtureClient`], which replays recorded
//! request / response pairs from `fixtures/connectors/**` and is what every
//! adapter test runs against. [`RecordingClient`] wraps a real client and
//! writes the pairs it saw, redacted, in the fixture format.
//!
//! The trait is blocking: adapters run on the blocking pool, so a sync
//! interface keeps them plain functions. [`ReqwestClient`] drives the async
//! reqwest client on one dedicated runtime that lives for the process.
//!
//! Logging rule (06 §5): request logs carry the method, the host + path and
//! the status; `Authorization` and query values whose name contains
//! `token`, `code`, `key`, `secret` or `verifier` never appear
//! ([`redact_url`], [`redact_json`]).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bodies larger than this are refused (a Gmail `format=full` message with
/// attachments inline is the largest thing Mode fetches).
pub const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// Default per-request timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    Get,
    Post,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
        }
    }
}

/// One request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Option<Duration>,
}

impl HttpRequest {
    pub fn new(method: Method, url: impl Into<String>) -> HttpRequest {
        HttpRequest {
            method,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout: None,
        }
    }

    pub fn get(url: impl Into<String>) -> HttpRequest {
        HttpRequest::new(Method::Get, url)
    }

    pub fn post(url: impl Into<String>) -> HttpRequest {
        HttpRequest::new(Method::Post, url)
    }

    /// Append query parameters (percent-encoded).
    pub fn query<K: AsRef<str>, V: AsRef<str>>(mut self, pairs: &[(K, V)]) -> HttpRequest {
        if pairs.is_empty() {
            return self;
        }
        if let Ok(mut u) = url::Url::parse(&self.url) {
            {
                let mut q = u.query_pairs_mut();
                for (k, v) in pairs {
                    q.append_pair(k.as_ref(), v.as_ref());
                }
            }
            self.url = u.to_string();
        }
        self
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> HttpRequest {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    pub fn bearer(self, token: &str) -> HttpRequest {
        self.header("authorization", format!("Bearer {token}"))
    }

    /// An `application/x-www-form-urlencoded` body.
    pub fn form<K: AsRef<str>, V: AsRef<str>>(mut self, pairs: &[(K, V)]) -> HttpRequest {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs.iter().map(|(k, v)| (k.as_ref(), v.as_ref())))
            .finish();
        self.body = Some(body.into_bytes());
        self.header("content-type", "application/x-www-form-urlencoded")
    }

    /// A JSON body.
    pub fn json(mut self, value: &Value) -> HttpRequest {
        self.body = Some(value.to_string().into_bytes());
        self.header("content-type", "application/json")
    }

    pub fn timeout(mut self, d: Duration) -> HttpRequest {
        self.timeout = Some(d);
        self
    }

    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn body_text(&self) -> String {
        self.body
            .as_deref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default()
    }

    /// The query parameters of the URL.
    pub fn query_pairs(&self) -> Vec<(String, String)> {
        url::Url::parse(&self.url)
            .map(|u| u.query_pairs().into_owned().collect())
            .unwrap_or_default()
    }

    /// The form fields of an urlencoded body.
    pub fn form_pairs(&self) -> Vec<(String, String)> {
        url::form_urlencoded::parse(self.body.as_deref().unwrap_or_default())
            .into_owned()
            .collect()
    }

    /// `METHOD host/path` with secrets stripped, for logs.
    pub fn log_line(&self) -> String {
        format!("{} {}", self.method.as_str(), redact_url(&self.url))
    }
}

/// One response.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The body as JSON (`Null` when it is not JSON).
    pub fn json_value(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    /// `Retry-After` in seconds (delta-seconds form only).
    pub fn retry_after_secs(&self) -> Option<u64> {
        self.header("retry-after")
            .and_then(|v| v.trim().parse::<f64>().ok())
            .map(|s| s.max(0.0).ceil() as u64)
    }
}

/// A transport failure (no HTTP status).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HttpError {
    /// DNS, connect or TLS failure: the network is unreachable (08 §11
    /// counts these toward the offline heuristic).
    #[error("network unreachable: {0}")]
    Offline(String),
    #[error("request timed out")]
    Timeout,
    #[error("http: {0}")]
    Other(String),
}

/// The client every connector and AI backend uses.
pub trait HttpClient: Send + Sync {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError>;
}

// ----------------------------------------------------------------- redact

fn secret_param(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["token", "code", "key", "secret", "verifier", "password"]
        .iter()
        .any(|s| n.contains(s))
}

/// `scheme://host/path?a=1&token=[redacted]` (06 §5 "Logs").
pub fn redact_url(raw: &str) -> String {
    let Ok(mut u) = url::Url::parse(raw) else {
        return crate::redact::scrub(raw).into_owned();
    };
    let pairs: Vec<(String, String)> = u.query_pairs().into_owned().collect();
    if pairs.is_empty() {
        return u.to_string();
    }
    {
        let mut q = u.query_pairs_mut();
        q.clear();
        for (k, v) in &pairs {
            if secret_param(k) {
                q.append_pair(k, crate::redact::REDACTED);
            } else {
                q.append_pair(k, v);
            }
        }
    }
    u.to_string()
}

/// Replace the values of secret-named keys anywhere in a JSON value.
pub fn redact_json(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| {
                    let v = if secret_param(k) && !v.is_object() && !v.is_array() && !v.is_null() {
                        Value::String(crate::redact::REDACTED.into())
                    } else {
                        redact_json(v)
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(redact_json).collect()),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------- reqwest

static HTTP_RUNTIME: OnceLock<std::result::Result<tokio::runtime::Runtime, String>> =
    OnceLock::new();

fn runtime() -> Result<&'static tokio::runtime::Runtime, HttpError> {
    HTTP_RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("mode-http")
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| HttpError::Other(e.clone()))
}

/// reqwest on rustls (ring provider), the platform verifier and the system
/// proxy. Requests run on a dedicated runtime that is never dropped, so the
/// blocking `send` works from any thread that is not inside an async task.
#[derive(Clone)]
pub struct ReqwestClient {
    client: reqwest::Client,
}

impl std::fmt::Debug for ReqwestClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReqwestClient").finish_non_exhaustive()
    }
}

impl ReqwestClient {
    pub fn new() -> Result<ReqwestClient, HttpError> {
        // Another crate may have installed a provider first; either is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .user_agent(format!("Mode/{}", crate::VERSION))
            .connect_timeout(Duration::from_secs(10))
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(|e| HttpError::Other(e.to_string()))?;
        Ok(ReqwestClient { client })
    }
}

/// Large files (the dictation models, XC-37): streamed chunk by chunk, never
/// held in memory, with a long overall deadline instead of the API one.
impl crate::speech::models::Fetch for ReqwestClient {
    fn fetch(
        &self,
        url: &str,
        on_start: &mut dyn FnMut(Option<u64>),
        on_chunk: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
    ) -> std::result::Result<(), String> {
        let rt = runtime().map_err(|e| e.to_string())?;
        // The request and its timer are created inside the async block: `send()` starts the
        // timeout's `Sleep` when it is called, which needs the runtime's context, and
        // `block_on` only enters it when polling. Called from a plain thread (the model
        // download, XC-37), the eager form panicked with "no reactor running".
        let mut resp = rt
            .block_on(async {
                self.client
                    .get(url)
                    .timeout(Duration::from_secs(60 * 60))
                    .send()
                    .await
            })
            .map_err(|e| classify(e).to_string())?;
        if !resp.status().is_success() {
            return Err(format!("the server answered {}", resp.status().as_u16()));
        }
        on_start(resp.content_length());
        loop {
            match rt.block_on(async { resp.chunk().await }) {
                Ok(Some(bytes)) => on_chunk(&bytes).map_err(|e| e.to_string())?,
                Ok(None) => return Ok(()),
                Err(e) => return Err(classify(e).to_string()),
            }
        }
    }
}

fn classify(e: reqwest::Error) -> HttpError {
    if e.is_timeout() {
        HttpError::Timeout
    } else if e.is_connect() {
        HttpError::Offline("connect failed".into())
    } else {
        // The error's Display can carry the URL; strip it.
        HttpError::Other(crate::redact::scrub(&e.without_url().to_string()).into_owned())
    }
}

impl HttpClient for ReqwestClient {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let rt = runtime()?;
        let method = match req.method {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
        };
        let mut b = self.client.request(method, &req.url);
        for (k, v) in &req.headers {
            b = b.header(k.as_str(), v.as_str());
        }
        if let Some(body) = &req.body {
            b = b.body(body.clone());
        }
        if let Some(t) = req.timeout {
            b = b.timeout(t);
        }
        let line = req.log_line();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        rt.spawn(async move {
            let out = async {
                let resp = b.send().await.map_err(classify)?;
                let status = resp.status().as_u16();
                let headers: Vec<(String, String)> = resp
                    .headers()
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.as_str().to_string(),
                            v.to_str().unwrap_or_default().to_string(),
                        )
                    })
                    .collect();
                let body = resp.bytes().await.map_err(classify)?;
                if body.len() > MAX_BODY_BYTES {
                    return Err(HttpError::Other("response too large".into()));
                }
                Ok(HttpResponse {
                    status,
                    headers,
                    body: body.to_vec(),
                })
            }
            .await;
            let _ = tx.send(out);
        });
        let out = rx
            .recv()
            .map_err(|_| HttpError::Other("http runtime stopped".into()))?;
        match &out {
            Ok(r) => tracing::debug!(target: "mode::http", status = r.status, "{line}"),
            Err(e) => tracing::debug!(target: "mode::http", error = %e, "{line}"),
        }
        out
    }
}

// --------------------------------------------------------------- fixtures

/// The request half of a recorded pair. `url` is matched on scheme, host
/// and path; its query string and `query` both name parameters that must
/// be present with equal values (`*` matches any value). `body_contains`
/// lists substrings of the request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureRequest {
    pub method: Method,
    pub url: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub query: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body_contains: Vec<String>,
    /// Parameters that must be absent (e.g. `pageToken` on the first page).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub query_absent: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureResponse {
    pub status: u16,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// A JSON body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    /// A non-JSON body (multipart batch responses, HTML).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    /// Fail at the transport level instead (`offline`, `timeout`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_error: Option<String>,
}

/// One recorded exchange.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixturePair {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub request: FixtureRequest,
    pub response: FixtureResponse,
    /// Serve this pair for every matching request (never consumed).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub repeat: bool,
}

fn base_and_query(raw: &str) -> (String, Vec<(String, String)>) {
    match url::Url::parse(raw) {
        Ok(u) => {
            let q = u.query_pairs().into_owned().collect();
            let mut base = u.clone();
            base.set_query(None);
            base.set_fragment(None);
            (base.to_string(), q)
        }
        Err(_) => (raw.to_string(), Vec::new()),
    }
}

impl FixtureRequest {
    pub fn matches(&self, req: &HttpRequest) -> bool {
        if self.method != req.method {
            return false;
        }
        let (want_base, mut want_q) = base_and_query(&self.url);
        let (got_base, got_q) = base_and_query(&req.url);
        if want_base != got_base {
            return false;
        }
        want_q.extend(self.query.iter().map(|(k, v)| (k.clone(), v.clone())));
        for (k, v) in &want_q {
            let found = got_q
                .iter()
                .any(|(gk, gv)| gk == k && (v == "*" || gv == v));
            if !found {
                return false;
            }
        }
        if self
            .query_absent
            .iter()
            .any(|k| got_q.iter().any(|(gk, _)| gk == k))
        {
            return false;
        }
        let body = req.body_text();
        self.body_contains.iter().all(|s| body.contains(s.as_str()))
    }
}

impl FixtureResponse {
    pub fn to_response(&self) -> Result<HttpResponse, HttpError> {
        match self.transport_error.as_deref() {
            Some("offline") => return Err(HttpError::Offline("fixture".into())),
            Some("timeout") => return Err(HttpError::Timeout),
            Some(other) => return Err(HttpError::Other(other.into())),
            None => {}
        }
        let mut headers: Vec<(String, String)> = self
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let body = match (&self.body, &self.body_text) {
            (Some(v), _) => {
                if !headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                {
                    headers.push(("content-type".into(), "application/json".into()));
                }
                v.to_string().into_bytes()
            }
            (None, Some(t)) => t.clone().into_bytes(),
            (None, None) => Vec::new(),
        };
        Ok(HttpResponse {
            status: self.status,
            headers,
            body,
        })
    }
}

/// Replays recorded pairs; the first unconsumed matching pair answers.
/// An unmatched request gets status 599 with a body naming it, so a test
/// fails with a readable message rather than a hang.
#[derive(Debug, Default)]
pub struct FixtureClient {
    pairs: Mutex<Vec<(FixturePair, bool)>>,
    seen: Mutex<Vec<HttpRequest>>,
}

impl FixtureClient {
    pub fn new() -> FixtureClient {
        FixtureClient::default()
    }

    pub fn from_pairs(pairs: Vec<FixturePair>) -> FixtureClient {
        let c = FixtureClient::new();
        c.extend(pairs);
        c
    }

    /// Load one fixture file (a JSON array of pairs).
    pub fn load(path: &Path) -> std::io::Result<FixtureClient> {
        let c = FixtureClient::new();
        c.load_more(path)?;
        Ok(c)
    }

    /// Append the pairs of another file.
    pub fn load_more(&self, path: &Path) -> std::io::Result<()> {
        let text = std::fs::read_to_string(path)?;
        let pairs: Vec<FixturePair> = serde_json::from_str(&text).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{}: {e}", path.display()),
            )
        })?;
        self.extend(pairs);
        Ok(())
    }

    pub fn extend(&self, pairs: impl IntoIterator<Item = FixturePair>) {
        let mut p = self.pairs.lock().unwrap_or_else(|e| e.into_inner());
        p.extend(pairs.into_iter().map(|x| (x, false)));
    }

    pub fn push(&self, pair: FixturePair) {
        self.extend([pair]);
    }

    /// Every request received, in order.
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn request_count(&self) -> usize {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Non-repeating pairs not served yet.
    pub fn unused(&self) -> Vec<FixturePair> {
        self.pairs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(p, used)| !used && !p.repeat)
            .map(|(p, _)| p.clone())
            .collect()
    }
}

impl HttpClient for FixtureClient {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(req.clone());
        let mut pairs = self.pairs.lock().unwrap_or_else(|e| e.into_inner());
        for (pair, used) in pairs.iter_mut() {
            if (!*used || pair.repeat) && pair.request.matches(req) {
                *used = true;
                return pair.response.to_response();
            }
        }
        Ok(HttpResponse {
            status: 599,
            headers: vec![],
            body: format!("no fixture for {}", req.log_line()).into_bytes(),
        })
    }
}

/// Wraps a real client and records every exchange, redacted, in the
/// fixture format (`save` writes them out). Bearer tokens, codes, secrets
/// and token fields never reach the file.
pub struct RecordingClient {
    inner: Arc<dyn HttpClient>,
    recorded: Mutex<Vec<FixturePair>>,
}

impl RecordingClient {
    pub fn new(inner: Arc<dyn HttpClient>) -> RecordingClient {
        RecordingClient {
            inner,
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub fn pairs(&self) -> Vec<FixturePair> {
        self.recorded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(&self.pairs())?;
        std::fs::write(path, text + "\n")
    }

    fn record(req: &HttpRequest, resp: &HttpResponse) -> FixturePair {
        let (base, q) = base_and_query(&redact_url(&req.url));
        let body_json: Option<Value> = serde_json::from_slice(&resp.body).ok();
        let headers = resp
            .headers
            .iter()
            .filter(|(k, _)| {
                let k = k.to_ascii_lowercase();
                k == "retry-after" || k == "request-id" || k == "content-type"
            })
            .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
            .collect();
        FixturePair {
            note: None,
            request: FixtureRequest {
                method: req.method,
                url: base,
                query: q.into_iter().collect(),
                body_contains: Vec::new(),
                query_absent: Vec::new(),
            },
            response: FixtureResponse {
                status: resp.status,
                headers,
                body_text: if body_json.is_none() && !resp.body.is_empty() {
                    Some(crate::redact::scrub(&resp.text()).into_owned())
                } else {
                    None
                },
                body: body_json.map(|v| redact_json(&v)),
                transport_error: None,
            },
            repeat: false,
        }
    }
}

impl HttpClient for RecordingClient {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let out = self.inner.send(req);
        if let Ok(resp) = &out {
            self.recorded
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(Self::record(req, resp));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn query_and_form_encode() {
        let r = HttpRequest::get("https://example.com/a")
            .query(&[("q", "newer_than:14d"), ("labelIds", "INBOX")]);
        assert!(r.url.contains("q=newer_than%3A14d"));
        let r = HttpRequest::post("https://x/token").form(&[("code", "a b"), ("x", "1")]);
        assert_eq!(r.body_text(), "code=a+b&x=1");
        assert_eq!(
            r.form_pairs(),
            vec![("code".into(), "a b".into()), ("x".into(), "1".into())]
        );
    }

    #[test]
    fn urls_and_json_are_redacted() {
        let u = redact_url("https://oauth2.googleapis.com/revoke?token=abc&x=1");
        assert!(u.contains("token=%5Bredacted%5D"));
        assert!(!u.contains("abc"));
        let v = redact_json(
            &json!({"access_token": "t", "nested": {"refresh_token": "r", "ok": 1}, "list": [{"code": "c"}]}),
        );
        assert_eq!(v["access_token"], "[redacted]");
        assert_eq!(v["nested"]["refresh_token"], "[redacted]");
        assert_eq!(v["nested"]["ok"], 1);
        assert_eq!(v["list"][0]["code"], "[redacted]");
    }

    #[test]
    fn fixture_matching_consumes_in_order() {
        let pair = |url: &str, body: Value| FixturePair {
            note: None,
            request: FixtureRequest {
                method: Method::Get,
                url: url.into(),
                query: BTreeMap::new(),
                body_contains: vec![],
                query_absent: vec![],
            },
            response: FixtureResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: Some(body),
                body_text: None,
                transport_error: None,
            },
            repeat: false,
        };
        let c = FixtureClient::from_pairs(vec![
            pair("https://h/p?page=1", json!({"n": 1})),
            pair("https://h/p", json!({"n": 2})),
        ]);
        let r = c
            .send(&HttpRequest::get("https://h/p").query(&[("page", "1"), ("other", "x")]))
            .unwrap();
        assert_eq!(r.json_value()["n"], 1);
        let r = c.send(&HttpRequest::get("https://h/p")).unwrap();
        assert_eq!(r.json_value()["n"], 2);
        let r = c.send(&HttpRequest::get("https://h/p")).unwrap();
        assert_eq!(r.status, 599);
        assert!(c.unused().is_empty());
        assert_eq!(c.request_count(), 3);
    }

    #[test]
    fn recording_strips_secrets() {
        let inner = Arc::new(FixtureClient::from_pairs(vec![FixturePair {
            note: None,
            request: FixtureRequest {
                method: Method::Post,
                url: "https://oauth2.googleapis.com/token".into(),
                query: BTreeMap::new(),
                body_contains: vec![],
                query_absent: vec![],
            },
            response: FixtureResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: Some(json!({"access_token": "ya29.secret", "expires_in": 3599})),
                body_text: None,
                transport_error: None,
            },
            repeat: false,
        }]));
        let rec = RecordingClient::new(inner);
        rec.send(
            &HttpRequest::post("https://oauth2.googleapis.com/token?code=abc123xyz")
                .bearer("tok-SECRET-123")
                .form(&[("code", "abc")]),
        )
        .unwrap();
        let text = serde_json::to_string(&rec.pairs()).unwrap();
        assert!(!text.contains("ya29.secret"));
        assert!(!text.contains("abc123xyz"));
        assert!(!text.contains("tok-SECRET-123"));
    }

    #[test]
    fn retry_after_is_read() {
        let r = HttpResponse {
            status: 429,
            headers: vec![("Retry-After".into(), "31".into())],
            body: vec![],
        };
        assert_eq!(r.retry_after_secs(), Some(31));
    }
}
