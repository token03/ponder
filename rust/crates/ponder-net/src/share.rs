//! Share-site fetching for `pob.read_code`: pobb.in and pastebin links to their raw text.
//!
//! Only the URL-to-text part lives here; decoding the PoB code (base64, zlib, XML)
//! is the caller's.

use crate::http::{self, Transport};
use anyhow::Result;

/// `[\w-]+` filling a whole string
fn id(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

/// The `/raw` URL for a pobb.in or pastebin share link; `None` for anything else.
pub fn raw_url(source: &str) -> Option<String> {
    let rest = source.trim();
    let rest = rest.strip_prefix("https://").or_else(|| rest.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/')?;
    let path = path.strip_suffix('/').unwrap_or(path);
    match host {
        "pobb.in" if id(path) => Some(format!("https://pobb.in/{path}/raw")),
        "pastebin.com" => {
            let code = path.strip_prefix("raw/").unwrap_or(path);
            (!code.is_empty() && code.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| format!("https://pastebin.com/raw/{code}"))
        }
        _ => None,
    }
}

/// The PoB code behind a share link: `None` if `source` isn't one (a file path or the code itself).
pub fn fetch(http: &dyn Transport, source: &str) -> Result<Option<String>> {
    let Some(url) = raw_url(source) else { return Ok(None) };
    let body = http::get(http, &url, "ponder", 30)?;
    Ok(Some(String::from_utf8(body)?.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Request, Response};

    #[test]
    fn share_links_map_to_raw_urls() {
        assert_eq!(raw_url("https://pobb.in/Ab-c_9/").as_deref(), Some("https://pobb.in/Ab-c_9/raw"));
        assert_eq!(raw_url(" http://pobb.in/xyz ").as_deref(), Some("https://pobb.in/xyz/raw"));
        assert_eq!(raw_url("https://pastebin.com/raw/QwE12").as_deref(), Some("https://pastebin.com/raw/QwE12"));
        assert_eq!(raw_url("https://pastebin.com/QwE12/").as_deref(), Some("https://pastebin.com/raw/QwE12"));
        assert_eq!(raw_url("https://pobb.in/a/b"), None);
        assert_eq!(raw_url("builds/mine.txt"), None);
        assert_eq!(raw_url("eNrtV"), None);
    }

    #[test]
    fn fetch_returns_the_trimmed_text() {
        let http = |req: &Request| {
            assert_eq!(req.url, "https://pobb.in/xyz/raw");
            Ok(Response::new(200, "  code\n"))
        };
        assert_eq!(fetch(&http, "https://pobb.in/xyz").unwrap().as_deref(), Some("code"));
        assert_eq!(fetch(&http, "not a link").unwrap(), None);
    }
}
