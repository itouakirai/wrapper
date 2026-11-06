//! Apple catalog probes used to route a request to an account whose
//! storefront can actually serve the item.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use reqwest::Client;

const WEB_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";
const DAY: Duration = Duration::from_secs(24 * 3600);

/// Small TTL map with a hard size cap (oldest entries are dropped first).
pub struct TtlCache<K, V> {
    map: Mutex<HashMap<K, (V, Instant)>>,
    ttl: Duration,
    cap: usize,
}

impl<K: Eq + Hash + Clone, V: Clone> TtlCache<K, V> {
    pub fn new(ttl: Duration, cap: usize) -> Self {
        Self { map: Mutex::new(HashMap::new()), ttl, cap }
    }

    pub fn get(&self, k: &K) -> Option<V> {
        let map = self.map.lock().unwrap();
        map.get(k).filter(|(_, t)| t.elapsed() < self.ttl).map(|(v, _)| v.clone())
    }

    pub fn insert(&self, k: K, v: V) {
        let mut map = self.map.lock().unwrap();
        if map.len() >= self.cap {
            map.retain(|_, (_, t)| t.elapsed() < self.ttl);
            if map.len() >= self.cap {
                // Still full of live entries: drop the oldest tenth.
                let mut ages: Vec<_> = map.iter().map(|(k, (_, t))| (*t, k.clone())).collect();
                ages.sort_by_key(|(t, _)| *t);
                for (_, k) in ages.into_iter().take(self.cap / 10 + 1) {
                    map.remove(&k);
                }
            }
        }
        map.insert(k, (v, Instant::now()));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Song,
    MusicVideo,
}

/// What one catalog lookup says about an item in one storefront.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CatalogItem {
    /// The storefront's catalog has the item.
    pub available: bool,
    /// `data[0].attributes.hasLyrics` of the same response (songs only).
    pub has_lyrics: bool,
}

/// Reads a catalog `songs/{id}` or `music-videos/{id}` response.
fn parse_catalog_item(v: &serde_json::Value) -> CatalogItem {
    if v.get("errors").is_some() {
        return CatalogItem::default();
    }
    match v.get("data").and_then(|d| d.as_array()).and_then(|d| d.first()) {
        Some(item) => CatalogItem {
            available: true,
            has_lyrics: item.pointer("/attributes/hasLyrics").and_then(|h| h.as_bool()).unwrap_or(false),
        },
        None => CatalogItem::default(),
    }
}

pub struct AppleClient {
    http: Client,
    token: tokio::sync::Mutex<Option<(String, Instant)>>,
    last_failure: Mutex<Option<Instant>>,
    catalog: TtlCache<(Kind, String, String), CatalogItem>,
    reach_url: Mutex<String>,
}

/// The FairPlay key server lite talks to; see [`AppleClient::reachable`].
const KEY_SERVER_URL: &str = "https://play.itunes.apple.com/WebObjects/MZPlay.woa/music/fps";

static INDEX_JS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/assets/index[~-][^/\x22']+\.js").unwrap());
static JWT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"eyJ[A-Za-z0-9_=-]+\.[A-Za-z0-9_=-]+\.[A-Za-z0-9_=-]+").unwrap());

impl AppleClient {
    pub fn new(proxy: Option<&str>) -> anyhow::Result<Self> {
        let mut b = Client::builder().timeout(Duration::from_secs(15)).user_agent(WEB_UA);
        if let Some(p) = proxy.filter(|p| !p.is_empty()) {
            let p = if p.contains("://") { p.to_string() } else { format!("http://{p}") };
            b = b.proxy(reqwest::Proxy::all(p)?);
        }
        Ok(Self {
            http: b.build()?,
            token: tokio::sync::Mutex::new(None),
            last_failure: Mutex::new(None),
            catalog: TtlCache::new(DAY, 200_000),
            reach_url: Mutex::new(KEY_SERVER_URL.to_string()),
        })
    }

    /// Whether Apple's FairPlay key server answers at all, through the same
    /// proxy as lite. Any HTTP status counts (the bare endpoint rejects a
    /// plain GET); only a transport failure (DNS, connect, TLS, proxy,
    /// timeout) does not. This is what tells "the network is down" apart from
    /// "lite is broken" when a key request fails.
    pub async fn reachable(&self) -> bool {
        let url = self.reach_url.lock().unwrap().clone();
        self.http.get(url).timeout(Duration::from_secs(10)).send().await.is_ok()
    }

    #[cfg(test)]
    pub(crate) fn set_reach_url(&self, url: &str) {
        *self.reach_url.lock().unwrap() = url.to_string();
    }

    /// The AMPWebPlay bearer token embedded in music.apple.com's JS bundle.
    pub async fn web_token(&self) -> anyhow::Result<String> {
        let mut guard = self.token.lock().await;
        if let Some((t, at)) = guard.as_ref() {
            if at.elapsed() < DAY {
                return Ok(t.clone());
            }
        }
        if let Some(at) = *self.last_failure.lock().unwrap() {
            if at.elapsed() < Duration::from_secs(60) {
                anyhow::bail!("web token unavailable (recent scrape failed)");
            }
        }
        match self.scrape_token().await {
            Ok(t) => {
                *guard = Some((t.clone(), Instant::now()));
                Ok(t)
            }
            Err(e) => {
                *self.last_failure.lock().unwrap() = Some(Instant::now());
                // Keep serving a stale token rather than nothing.
                if let Some((t, _)) = guard.as_ref() {
                    return Ok(t.clone());
                }
                Err(e)
            }
        }
    }

    async fn scrape_token(&self) -> anyhow::Result<String> {
        let home = self.http.get("https://music.apple.com").send().await?.text().await?;
        let js = INDEX_JS.find(&home).ok_or_else(|| anyhow::anyhow!("index js not found"))?.as_str();
        let body = self.http.get(format!("https://music.apple.com{js}")).send().await?.text().await?;
        let token = JWT.find(&body).ok_or_else(|| anyhow::anyhow!("token not found in {js}"))?;
        Ok(token.as_str().to_string())
    }

    /// What the `region` catalog says about `adam_id`: whether it exists there
    /// and, for songs, whether it has lyrics. Both come from one response, so
    /// routing a lyrics request costs no extra Apple request; `language` and
    /// `script` do not affect either answer.
    pub async fn lookup(&self, kind: Kind, region: &str, adam_id: &str) -> anyhow::Result<CatalogItem> {
        if adam_id == "0" {
            return Ok(CatalogItem { available: true, has_lyrics: false });
        }
        let key = (kind, region.to_string(), adam_id.to_string());
        if let Some(v) = self.catalog.get(&key) {
            return Ok(v);
        }
        let path = match kind {
            Kind::Song => "songs",
            Kind::MusicVideo => "music-videos",
        };
        let token = self.web_token().await?;
        let resp = self
            .http
            .get(format!("https://amp-api.music.apple.com/v1/catalog/{region}/{path}/{adam_id}"))
            .bearer_auth(token)
            .header("Origin", "https://music.apple.com")
            .send()
            .await?;
        let status = resp.status();
        if status.is_server_error() || status.as_u16() == 401 || status.as_u16() == 429 {
            anyhow::bail!("catalog probe returned {status}");
        }
        let v: serde_json::Value = resp.json().await.unwrap_or_default();
        let item = parse_catalog_item(&v);
        self.catalog.insert(key, item);
        Ok(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_catalog_items() {
        use serde_json::json;
        let item = |available, has_lyrics| CatalogItem { available, has_lyrics };
        for (name, body, want) in [
            ("song with lyrics", json!({"data": [{"attributes": {"hasLyrics": true}}]}), item(true, true)),
            ("song without lyrics", json!({"data": [{"attributes": {"hasLyrics": false}}]}), item(true, false)),
            ("field missing", json!({"data": [{"attributes": {"name": "x"}}]}), item(true, false)),
            ("not in the catalog", json!({"data": []}), item(false, false)),
            ("apple error", json!({"errors": [{"status": "404"}]}), item(false, false)),
            ("not an object", json!(null), item(false, false)),
        ] {
            assert_eq!(parse_catalog_item(&body), want, "{name}");
        }
    }

    #[test]
    fn ttl_cache_caps_size() {
        let c = TtlCache::new(Duration::from_secs(60), 10);
        for i in 0..25 {
            c.insert(i, i);
        }
        assert!(c.map.lock().unwrap().len() <= 10);
        assert_eq!(c.get(&24), Some(24));
    }

    #[test]
    fn regexes_match_bundle_shapes() {
        assert!(INDEX_JS.is_match(r#"<script src="/assets/index~a1b2c3.js">"#));
        assert!(INDEX_JS.is_match(r#"<script src="/assets/index-DyQ9.js">"#));
        assert!(JWT.is_match("x=\"eyJhbGciOi.eyJpc3Mi.sig-_=\""));
    }
}
