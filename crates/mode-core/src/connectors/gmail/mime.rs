//! Gmail message parsing (06 §6.6): headers from `payload.headers`, address
//! lists, the plain-text body (a `text/plain` part, else HTML reduced to
//! text), attachments by `filename`, Gmail's HTML-escaped snippet, and the
//! label hints (`list`, `bulk`, `auto`) from `List-Id`,
//! `List-Unsubscribe`, `Precedence` and `Auto-Submitted`.

use base64::Engine;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use serde_json::Value;

/// A header value by name (case-insensitive).
pub fn header<'a>(payload: &'a Value, name: &str) -> Option<&'a str> {
    payload
        .get("headers")?
        .as_array()?
        .iter()
        .find(|h| {
            h.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })?
        .get("value")?
        .as_str()
}

/// Split an address list on commas outside quotes and angle brackets.
pub fn split_addresses(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut cur, mut quoted, mut angle) = (String::new(), false, false);
    for c in raw.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            '<' if !quoted => {
                angle = true;
                cur.push(c);
            }
            '>' if !quoted => {
                angle = false;
                cur.push(c);
            }
            ',' if !quoted && !angle => {
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_string());
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// `"Ana Ruiz" <Ana@X.com>` → (`Ana Ruiz`, `ana@x.com`).
pub fn parse_address(raw: &str) -> (String, String) {
    let raw = raw.trim();
    if let (Some(lt), Some(gt)) = (raw.rfind('<'), raw.rfind('>'))
        && lt < gt
    {
        let addr = raw[lt + 1..gt].trim().to_lowercase();
        let name = raw[..lt]
            .trim()
            .trim_matches('"')
            .replace("\\\"", "\"")
            .trim()
            .to_string();
        let name = if name.is_empty() { addr.clone() } else { name };
        return (name, addr);
    }
    let addr = raw.trim_matches(|c| c == '<' || c == '>').to_lowercase();
    (addr.clone(), addr)
}

/// "Re:" / "Fwd:" / "Fw:" stripped (repeatedly) for the thread title.
pub fn thread_title(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        let cut = ["re:", "fwd:", "fw:", "aw:", "wg:"]
            .iter()
            .find(|p| lower.starts_with(*p))
            .map(|p| p.len());
        match cut {
            Some(n) => s = s[n..].trim_start(),
            None => break,
        }
    }
    if s.is_empty() {
        "(no subject)".into()
    } else {
        s.to_string()
    }
}

/// Decode the few HTML entities Gmail puts in `snippet`.
pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let end = tail.find(';').filter(|e| *e <= 10);
        let decoded = end.and_then(|e| {
            let ent = &tail[1..e];
            let ch = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                    u32::from_str_radix(&ent[2..], 16)
                        .ok()
                        .and_then(char::from_u32)
                }
                _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            ch.map(|c| (c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &tail[e + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// HTML → text: tags dropped, block tags become line breaks, scripts and
/// styles removed, entities decoded.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let close = lower[i..]
                .find('>')
                .map(|e| i + e + 1)
                .unwrap_or(bytes.len());
            let tag = &lower[i..close];
            if let Some(skip) = ["<script", "<style", "<head"]
                .iter()
                .find(|s| tag.starts_with(**s))
            {
                let end_tag = format!("</{}", &skip[1..]);
                let end = lower[close..]
                    .find(&end_tag)
                    .map(|e| close + e)
                    .unwrap_or(bytes.len());
                i = lower[end..]
                    .find('>')
                    .map(|e| end + e + 1)
                    .unwrap_or(bytes.len());
                continue;
            }
            if [
                "<br", "<p", "</p", "<div", "</div", "<li", "<tr", "<h1", "<h2", "<h3", "</h",
            ]
            .iter()
            .any(|t| tag.starts_with(t))
            {
                out.push('\n');
            }
            i = close;
            continue;
        }
        let next = lower[i..].find('<').map(|e| i + e).unwrap_or(bytes.len());
        out.push_str(&html[i..next]);
        i = next;
    }
    let text = decode_entities(&out);
    text.lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_b64(data: &str) -> Option<Vec<u8>> {
    let d = data.trim();
    URL_SAFE_NO_PAD
        .decode(d.trim_end_matches('='))
        .or_else(|_| URL_SAFE.decode(d))
        .ok()
}

/// The body text and whether any part carries a file name.
pub fn body_text(payload: &Value) -> (Option<String>, bool) {
    let mut plain = None;
    let mut html = None;
    let mut attachments = false;
    walk(payload, &mut plain, &mut html, &mut attachments);
    let text = plain.or_else(|| html.map(|h| html_to_text(&h)));
    (text, attachments)
}

fn walk(part: &Value, plain: &mut Option<String>, html: &mut Option<String>, att: &mut bool) {
    let mime = part
        .get("mimeType")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let filename = part.get("filename").and_then(Value::as_str).unwrap_or("");
    if !filename.is_empty() {
        *att = true;
    } else if let Some(data) = part.pointer("/body/data").and_then(Value::as_str)
        && let Some(bytes) = decode_b64(data)
    {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if mime == "text/plain" && plain.is_none() {
            *plain = Some(text);
        } else if mime == "text/html" && html.is_none() {
            *html = Some(text);
        }
    }
    if let Some(parts) = part.get("parts").and_then(Value::as_array) {
        for p in parts {
            walk(p, plain, html, att);
        }
    }
}

/// The `list` / `bulk` / `auto` hints (06 §6.6).
pub fn hint_labels(payload: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if header(payload, "List-Id").is_some() || header(payload, "List-Unsubscribe").is_some() {
        out.push("list".to_string());
    }
    if header(payload, "Precedence").is_some_and(|p| {
        matches!(
            p.trim().to_ascii_lowercase().as_str(),
            "bulk" | "list" | "junk"
        )
    }) {
        out.push("bulk".to_string());
    }
    if header(payload, "Auto-Submitted").is_some_and(|v| !v.trim().eq_ignore_ascii_case("no")) {
        out.push("auto".to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn addresses() {
        assert_eq!(
            parse_address("\"Ruiz, Ana\" <Ana@Example.com>"),
            ("Ruiz, Ana".into(), "ana@example.com".into())
        );
        assert_eq!(
            parse_address("bob@x.com"),
            ("bob@x.com".into(), "bob@x.com".into())
        );
        assert_eq!(
            split_addresses("\"Ruiz, Ana\" <a@x.com>, b@x.com,<c@x.com>"),
            vec!["\"Ruiz, Ana\" <a@x.com>", "b@x.com", "<c@x.com>"]
        );
    }

    #[test]
    fn titles_and_entities() {
        assert_eq!(thread_title("Re: Fwd: RE: Contract v3"), "Contract v3");
        assert_eq!(thread_title("Re:"), "(no subject)");
        assert_eq!(
            decode_entities("It&#39;s &amp; &lt;b&gt; &#x41;"),
            "It's & <b> A"
        );
        assert_eq!(decode_entities("a & b"), "a & b");
    }

    #[test]
    fn html_reduces_to_text() {
        let t = html_to_text(
            "<html><head><style>p{}</style></head><body><p>Hello&nbsp;<b>there</b></p><div>line two</div><script>x()</script></body></html>",
        );
        assert_eq!(t, "Hello there\nline two");
    }

    #[test]
    fn bodies_and_attachments() {
        let data = URL_SAFE_NO_PAD.encode("Plain body text");
        let p = json!({"mimeType":"multipart/mixed","parts":[
            {"mimeType":"multipart/alternative","parts":[
                {"mimeType":"text/plain","body":{"data": data}},
                {"mimeType":"text/html","body":{"data": URL_SAFE_NO_PAD.encode("<p>HTML</p>")}}]},
            {"mimeType":"application/pdf","filename":"x.pdf","body":{"attachmentId":"a1"}}]});
        assert_eq!(body_text(&p), (Some("Plain body text".into()), true));
        let only_html =
            json!({"mimeType":"text/html","body":{"data": URL_SAFE_NO_PAD.encode("<p>Hi</p>")}});
        assert_eq!(body_text(&only_html).0.as_deref(), Some("Hi"));
    }

    #[test]
    fn hints() {
        let p = json!({"headers":[{"name":"List-Unsubscribe","value":"<mailto:x>"},{"name":"Precedence","value":"bulk"},{"name":"Auto-Submitted","value":"auto-generated"}]});
        assert_eq!(hint_labels(&p), vec!["list", "bulk", "auto"]);
        let q = json!({"headers":[{"name":"Auto-Submitted","value":"no"}]});
        assert!(hint_labels(&q).is_empty());
    }
}
