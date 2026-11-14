//! The catalog: a read-only index of packs on a web host the owner names
//! (`marketplace.index_url`, by default `index.json` of the `gil10101/mode_cat`
//! repository on GitHub, 06 §12). Mode fetches the index and the pack files it points
//! to and sends nothing: no account, no telemetry, no query string beyond
//! the URL as stored.
//!
//! Rules: the index URL is `https` (plain `http` only to a loopback host,
//! for tests); a pack URL is resolved against the index URL and must stay
//! on the index's host, so an index cannot send Mode to a third party; a
//! document larger than 2 MB is refused; one request waits at most 20 s.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::Url;

use super::counts_line;
use crate::connectors::http::{HttpClient, HttpError, HttpRequest};

/// `kind` of an index document.
pub const INDEX_KIND: &str = "mode-catalogue";
/// The index format this build reads.
pub const INDEX_FORMAT: i64 = 1;
/// An index larger than this is refused.
pub const MAX_INDEX_BYTES: usize = 2 * 1024 * 1024;
/// Entries per index.
pub const MAX_ENTRIES: usize = 500;
/// Per request.
pub const TIMEOUT: Duration = Duration::from_secs(20);
/// Where the index lives unless `marketplace.index_url` says otherwise.
pub const DEFAULT_INDEX_URL: &str =
    "https://raw.githubusercontent.com/gil10101/mode_cat/main/index.json";

/// One line of the index file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    /// The pack file: absolute, or relative to the index.
    pub url: String,
    #[serde(default)]
    pub agents: usize,
    #[serde(default)]
    pub automations: usize,
    #[serde(default)]
    pub themes: usize,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Up to six hex colors of a theme pack (bg, surface, fg, accent, ok,
    /// info), for the swatch on the catalog card.
    #[serde(default)]
    pub colors: Vec<String>,
    /// The accessories its agents wear (`avatar_variant`, 05 §11.2), up to
    /// six, for the bot marks on the catalog card.
    #[serde(default)]
    pub avatars: Vec<String>,
}

/// The index file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub format: i64,
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub packs: Vec<IndexEntry>,
}

/// One pack of the catalog, as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogueEntry {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub version: Option<String>,
    /// Resolved, absolute.
    pub url: String,
    pub agents: usize,
    pub automations: usize,
    pub themes: usize,
    /// "2 agents · 1 automation".
    pub counts: String,
    pub tags: Vec<String>,
    /// A theme pack's colors (hex), when the index gives them.
    pub colors: Vec<String>,
    /// Its agents' accessories, when the index gives them.
    pub avatars: Vec<String>,
}

/// The catalog as fetched (`catalogue_get`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalogue {
    pub url: String,
    /// The index's host, for the card ("raw.githubusercontent.com").
    pub host: String,
    pub name: Option<String>,
    pub fetched_at: String,
    pub packs: Vec<CatalogueEntry>,
}

fn loopback(u: &Url) -> bool {
    matches!(
        u.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("[::1]")
    )
}

/// A usable index URL: absolute, `https` (or `http` on loopback), a host.
pub fn check_url(raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("No catalog address is set.".into());
    }
    let u = Url::parse(raw).map_err(|e| format!("{raw} is not a URL: {e}."))?;
    if u.host_str().is_none() {
        return Err(format!("{raw} names no host."));
    }
    match u.scheme() {
        "https" => Ok(u),
        "http" if loopback(&u) => Ok(u),
        s => Err(format!("The catalog must be fetched over https, not {s}.")),
    }
}

/// A pack URL of the index: joined to the index URL, on the index's host.
pub fn resolve_pack_url(index: &Url, raw: &str) -> Result<Url, String> {
    let u = index
        .join(raw.trim())
        .map_err(|e| format!("{raw} is not a URL: {e}."))?;
    if u.scheme() != index.scheme() || u.host_str() != index.host_str() || u.port() != index.port()
    {
        return Err(format!(
            "{} is not on the catalog's host {}.",
            u,
            index.host_str().unwrap_or("")
        ));
    }
    Ok(u)
}

/// GET a document; a body over `max` bytes, a non-2xx status or a transport
/// failure is one sentence for the user.
pub fn fetch_text(http: &Arc<dyn HttpClient>, url: &Url, max: usize) -> Result<String, String> {
    let req = HttpRequest::get(url.as_str())
        .header("accept", "application/json")
        .timeout(TIMEOUT);
    let resp = match http.send(&req) {
        Ok(r) => r,
        Err(HttpError::Offline(_)) => {
            return Err("Mode is offline: the catalog could not be fetched.".into());
        }
        Err(HttpError::Timeout) => return Err("The catalog did not answer in time.".into()),
        Err(HttpError::Other(e)) => {
            return Err(format!("The catalog could not be fetched: {e}."));
        }
    };
    if !resp.ok() {
        return Err(match resp.status {
            404 => format!("Nothing is published at {}.", url),
            s => format!("{} answered {s}.", url.host_str().unwrap_or("the host")),
        });
    }
    if resp.body.len() > max {
        return Err(format!(
            "{} is larger than {} MB.",
            url,
            max / (1024 * 1024)
        ));
    }
    String::from_utf8(resp.body.clone()).map_err(|_| format!("{} is not text.", url))
}

/// Parse an index document.
pub fn parse_index(text: &str) -> Result<Index, String> {
    let idx: Index =
        serde_json::from_str(text).map_err(|e| format!("The index is not readable: {e}."))?;
    if idx.kind != INDEX_KIND {
        return Err(format!(
            "The index is a {} file, not a Mode catalog.",
            idx.kind
        ));
    }
    if idx.format > INDEX_FORMAT {
        return Err(format!(
            "This catalog needs a newer Mode (format {}; this Mode reads format {INDEX_FORMAT}).",
            idx.format
        ));
    }
    if idx.packs.len() > MAX_ENTRIES {
        return Err(format!("The index lists more than {MAX_ENTRIES} packs."));
    }
    Ok(idx)
}

/// An accessory name as the agent profiles keep it: short, `[a-z0-9_-]`.
fn variant_ok(a: &str) -> bool {
    !a.is_empty()
        && a.len() <= 32
        && a.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Fetch and read the index at `url`; entries with a bad slug or an
/// off-host URL are dropped, not fatal.
pub fn fetch(http: &Arc<dyn HttpClient>, url: &str, now: &str) -> Result<Catalogue, String> {
    let index_url = check_url(url)?;
    let text = fetch_text(http, &index_url, MAX_INDEX_BYTES)?;
    let idx = parse_index(&text)?;
    let mut packs = Vec::with_capacity(idx.packs.len());
    let mut seen = std::collections::BTreeSet::new();
    for e in idx.packs {
        if !super::slug_ok(&e.slug) || !seen.insert(e.slug.clone()) {
            continue;
        }
        let Ok(u) = resolve_pack_url(&index_url, &e.url) else {
            continue;
        };
        packs.push(CatalogueEntry {
            counts: counts_line(e.agents, e.automations, e.themes),
            slug: e.slug,
            name: e.name,
            description: e.description,
            author: e.author,
            version: e.version,
            url: u.to_string(),
            agents: e.agents,
            automations: e.automations,
            themes: e.themes,
            tags: e.tags,
            colors: e
                .colors
                .into_iter()
                .filter(|c| crate::themes::color::is_hex(c))
                .take(6)
                .collect(),
            avatars: e
                .avatars
                .into_iter()
                .filter(|a| variant_ok(a))
                .take(6)
                .collect(),
        });
    }
    Ok(Catalogue {
        url: index_url.to_string(),
        host: index_url.host_str().unwrap_or("").to_string(),
        name: idx.name,
        fetched_at: now.to_string(),
        packs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_https_and_packs_stay_on_the_host() {
        assert!(check_url("").is_err());
        assert!(check_url("ftp://x.y/i.json").is_err());
        assert!(check_url("http://example.com/i.json").is_err());
        assert!(check_url("http://127.0.0.1:9/i.json").is_ok());
        let idx = check_url("https://raw.githubusercontent.com/gil10101/mode_cat/main/index.json")
            .unwrap();
        assert_eq!(
            resolve_pack_url(&idx, "packs/reviewer.json")
                .unwrap()
                .as_str(),
            "https://raw.githubusercontent.com/gil10101/mode_cat/main/packs/reviewer.json"
        );
        assert_eq!(
            resolve_pack_url(&idx, "/other/x.json").unwrap().as_str(),
            "https://raw.githubusercontent.com/other/x.json"
        );
        assert!(resolve_pack_url(&idx, "https://evil.example/x.json").is_err());
        assert!(resolve_pack_url(&idx, "http://raw.githubusercontent.com/x.json").is_err());
    }

    #[test]
    fn the_index_is_checked() {
        assert!(parse_index("[]").is_err());
        assert!(
            parse_index(r#"{"format":1,"kind":"mode-pack"}"#)
                .unwrap_err()
                .contains("mode-pack")
        );
        assert!(
            parse_index(r#"{"format":2,"kind":"mode-catalogue"}"#)
                .unwrap_err()
                .contains("newer")
        );
        let i =
            parse_index(r#"{"format":1,"kind":"mode-catalogue","name":"Mode packs","packs":[]}"#)
                .unwrap();
        assert_eq!(i.name.as_deref(), Some("Mode packs"));
    }
}
