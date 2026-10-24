//! The one seam to the network: a `Transport` takes a `Request`, returns a `Response`.
//!
//! Everything in this crate talks HTTP through `Arc<dyn Transport>`, so tests
//! hand in a closure (`|req| Ok(Response::json(200, ..))`) and never touch the network.
//! Non-2xx answers are returned, not raised: the trade code reads rate-limit
//! headers off 429s. `get` is the `urlopen` shape (error unless 2xx).

use anyhow::{Result, bail};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// An outgoing request. `Debug` hides header values: they may carry the POESESSID cookie.
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<_> = self.headers.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("Request").field("method", &self.method).field("url", &self.url).field("headers", &names).finish()
    }
}

/// Response headers, looked up case-insensitively (`Headers.get` in Python's `urllib`).
#[derive(Clone, Debug, Default)]
pub struct Headers(HashMap<String, String>);

impl Headers {
    pub fn new<K: AsRef<str>, V: Into<String>>(pairs: impl IntoIterator<Item = (K, V)>) -> Headers {
        Headers(pairs.into_iter().map(|(k, v)| (k.as_ref().to_ascii_lowercase(), v.into())).collect())
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(&name.to_ascii_lowercase()).map(String::as_str)
    }
}

#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Response {
        Response { status, headers: Headers::default(), body: body.into() }
    }

    /// A response with a JSON body (for fakes).
    pub fn json(status: u16, body: &serde_json::Value) -> Response {
        Response::new(status, body.to_string())
    }

    pub fn with_headers<K: AsRef<str>, V: Into<String>>(mut self, pairs: impl IntoIterator<Item = (K, V)>) -> Response {
        self.headers = Headers::new(pairs);
        self
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

pub trait Transport: Send + Sync {
    fn send(&self, req: &Request) -> Result<Response>;
}

/// Any closure is a transport, which is how tests fake the network.
impl<F: Fn(&Request) -> Result<Response> + Send + Sync> Transport for F {
    fn send(&self, req: &Request) -> Result<Response> {
        self(req)
    }
}

/// The real network, through ureq (honours `HTTPS_PROXY` and friends).
pub struct Ureq {
    agent: ureq::Agent,
}

impl Ureq {
    pub fn new() -> Ureq {
        let config = ureq::Agent::config_builder().http_status_as_error(false).build();
        Ureq { agent: config.into() }
    }
}

impl Default for Ureq {
    fn default() -> Self {
        Ureq::new()
    }
}

impl Transport for Ureq {
    fn send(&self, req: &Request) -> Result<Response> {
        let mut resp = match (req.method, &req.body) {
            (Method::Get, None) => {
                let mut b = self.agent.get(&req.url).config().timeout_global(Some(req.timeout)).http_status_as_error(false).build();
                for (k, v) in &req.headers {
                    b = b.header(k, v);
                }
                b.call()?
            }
            (_, body) => {
                let mut b = self.agent.post(&req.url).config().timeout_global(Some(req.timeout)).http_status_as_error(false).build();
                for (k, v) in &req.headers {
                    b = b.header(k, v);
                }
                b.send(body.as_deref().unwrap_or(&[]))?
            }
        };
        let status = resp.status().as_u16();
        let headers = Headers::new(resp.headers().iter().filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str(), v))));
        let body = resp.body_mut().with_config().limit(1 << 30).read_to_vec()?;
        Ok(Response { status, headers, body })
    }
}

/// The default transport: the real network.
pub fn real() -> Arc<dyn Transport> {
    Arc::new(Ureq::new())
}

/// `urlopen(Request(url, headers={"User-Agent": ..}), timeout=..).read()`: the body of a 2xx answer, else an error.
pub fn get(http: &dyn Transport, url: &str, user_agent: &str, timeout_secs: u64) -> Result<Vec<u8>> {
    let req = Request {
        method: Method::Get,
        url: url.to_string(),
        headers: vec![("User-Agent".into(), user_agent.into())],
        body: None,
        timeout: Duration::from_secs(timeout_secs),
    };
    let resp = http.send(&req)?;
    if !resp.ok() {
        bail!("HTTP {} for {url}", resp.status);
    }
    Ok(resp.body)
}

/// `get`, parsed as JSON.
pub fn get_json(http: &dyn Transport, url: &str, user_agent: &str, timeout_secs: u64) -> Result<serde_json::Value> {
    Ok(serde_json::from_slice(&get(http, url, user_agent, timeout_secs)?)?)
}
