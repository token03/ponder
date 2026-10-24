//! Python-compatible text helpers: `json.dumps`, `urllib.parse.quote`, `pathlib` suffix rules, `sha1`.
//!
//! Cache keys and trade query strings have to match what Python produces byte
//! for byte (the two ports share `.cache/`), so `serde_json`'s own writer
//! (UTF-8, no space after separators, ryu floats) is not enough.

use serde_json::{Number, Value};
use std::fmt::Write;

/// How `json.dumps` is called.
#[derive(Clone, Copy, Default)]
pub struct Dumps {
    /// `sort_keys=True`
    pub sort_keys: bool,
    /// `separators=(",", ":")`
    pub compact: bool,
    /// `indent=N`
    pub indent: Option<usize>,
}

/// `json.dumps(value)` with default separators and `ensure_ascii=True`.
pub fn dumps(v: &Value) -> String {
    dumps_with(v, Dumps::default())
}

/// `json.dumps(value, sort_keys=True)`.
pub fn dumps_sorted(v: &Value) -> String {
    dumps_with(v, Dumps { sort_keys: true, ..Dumps::default() })
}

/// `json.dumps(value, separators=(",", ":"))`.
pub fn dumps_compact(v: &Value) -> String {
    dumps_with(v, Dumps { compact: true, ..Dumps::default() })
}

pub fn dumps_with(v: &Value, o: Dumps) -> String {
    let mut out = String::new();
    write_value(&mut out, v, o, 0);
    out
}

fn write_value(out: &mut String, v: &Value, o: Dumps, depth: usize) {
    // With an indent Python's item separator is "," + newline, and the key separator ": ".
    let (item, key) = match (o.indent, o.compact) {
        (Some(_), _) => (",", ": "),
        (None, true) => (",", ":"),
        (None, false) => (", ", ": "),
    };
    let newline = |out: &mut String, d: usize| {
        if let Some(n) = o.indent {
            out.push('\n');
            out.push_str(&" ".repeat(n * d));
        }
    };
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&number(n)),
        Value::String(s) => string(out, s),
        Value::Array(a) if a.is_empty() => out.push_str("[]"),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(item);
                }
                newline(out, depth + 1);
                write_value(out, x, o, depth + 1);
            }
            newline(out, depth);
            out.push(']');
        }
        Value::Object(m) if m.is_empty() => out.push_str("{}"),
        Value::Object(m) => {
            let mut entries: Vec<_> = m.iter().collect();
            if o.sort_keys {
                entries.sort_by(|a, b| a.0.cmp(b.0));
            }
            out.push('{');
            for (i, (k, x)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push_str(item);
                }
                newline(out, depth + 1);
                string(out, k);
                out.push_str(key);
                write_value(out, x, o, depth + 1);
            }
            newline(out, depth);
            out.push('}');
        }
    }
}

fn number(n: &Number) -> String {
    match n.as_f64() {
        Some(f) if n.is_f64() => float_repr(f),
        _ => n.to_string(),
    }
}

/// Python's `repr(float)`: shortest round-trip digits, exponent form below 1e-4 and from 1e16.
pub fn float_repr(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let sci = format!("{:e}", f.abs()); // e.g. "1.2345e-5", "1e16"
    let (mant, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let sign = if f < 0.0 { "-" } else { "" };
    if (-4..16).contains(&exp) {
        if exp >= 0 {
            let int_len = exp as usize + 1;
            if digits.len() <= int_len {
                format!("{sign}{digits}{}.0", "0".repeat(int_len - digits.len()))
            } else {
                format!("{sign}{}.{}", &digits[..int_len], &digits[int_len..])
            }
        } else {
            format!("{sign}0.{}{digits}", "0".repeat((-exp - 1) as usize))
        }
    } else {
        let m = if digits.len() == 1 { digits } else { format!("{}.{}", &digits[..1], &digits[1..]) };
        format!("{sign}{m}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs())
    }
}

/// A JSON string literal as `json.dumps` writes it (`ensure_ascii=True`).
fn string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{u:04x}");
                }
            }
        }
    }
    out.push('"');
}

/// `urllib.parse.quote(s)`: percent-encode UTF-8 bytes except letters, digits, `_.-~` and `/`.
pub fn quote(s: &str) -> String {
    quote_with(s, "/", false)
}

/// `urllib.parse.quote_plus(s)` (what `urlencode` uses): like `quote` with no safe characters and `+` for spaces.
pub fn quote_plus(s: &str) -> String {
    quote_with(s, "", true)
}

fn quote_with(s: &str, safe: &str, plus: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) || safe.as_bytes().contains(&b) {
            out.push(b as char);
        } else if plus && b == b' ' {
            out.push('+');
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// `urllib.parse.urlencode(pairs)`.
pub fn urlencode(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", quote_plus(k), quote_plus(v))).collect::<Vec<_>>().join("&")
}

/// pathlib's suffix of a file name: from the last dot, unless it is the first or last character.
fn suffix_at(name: &str) -> Option<usize> {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => Some(i),
        _ => None,
    }
}

/// `PurePath(name).with_suffix(new)`: replaces the last suffix, or appends when there is none.
pub fn with_suffix(name: &str, new: &str) -> String {
    format!("{}{new}", &name[..suffix_at(name).unwrap_or(name.len())])
}

/// SHA-1 as lowercase hex (`hashlib.sha1(data).hexdigest()`).
pub fn sha1_hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, c) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

/// Python truthiness of a JSON value (`if value:`).
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sha1_known_vectors() {
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(&[b'a'; 1000]), "291e9a6c66994949b57ba5e650361e98fc36b1ba");
    }

    #[test]
    fn floats_print_like_python() {
        for (f, s) in [(1.0, "1.0"), (0.1, "0.1"), (1e16, "1e+16"), (1.5e16, "1.5e+16"), (123456789012345.0, "123456789012345.0"), (1e-5, "1e-05"), (0.0001, "0.0001"), (-2.5, "-2.5"), (1e22, "1e+22"), (1234.5678, "1234.5678")] {
            assert_eq!(float_repr(f), s);
        }
    }

    #[test]
    fn dumps_escapes_and_separators() {
        let v = json!({"b": [1, 2.0, "é\u{1F600}\"\n"], "a": {}, "c": []});
        assert_eq!(dumps(&v), r#"{"b": [1, 2.0, "\u00e9\ud83d\ude00\"\n"], "a": {}, "c": []}"#);
        assert_eq!(dumps_sorted(&v), r#"{"a": {}, "b": [1, 2.0, "\u00e9\ud83d\ude00\"\n"], "c": []}"#);
        assert_eq!(dumps_compact(&json!({"a": [1, 2]})), r#"{"a":[1,2]}"#);
        assert_eq!(dumps_with(&json!({"a": {"b": 1}, "c": []}), Dumps { indent: Some(1), ..Dumps::default() }), "{\n \"a\": {\n  \"b\": 1\n },\n \"c\": []\n}");
    }

    #[test]
    fn quoting_and_suffixes() {
        assert_eq!(quote("Settlers of Kalguur/x y~é"), "Settlers%20of%20Kalguur/x%20y~%C3%A9");
        assert_eq!(quote_plus("a b&c"), "a+b%26c");
        assert_eq!(with_suffix("a.b", ".lite"), "a.lite");
        assert_eq!(with_suffix("a", ".lite"), "a.lite");
        assert_eq!(with_suffix("a.lite", ""), "a");
        assert_eq!(with_suffix(".x", ".lite"), ".x.lite");
        assert_eq!(with_suffix("a.", ".lite"), "a..lite");
    }
}
