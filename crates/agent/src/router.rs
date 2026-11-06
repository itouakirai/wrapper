//! Picks an account able to serve a request and forwards it to its lite
//! instance, retrying on another account when the failure may be
//! instance-specific.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use rand::seq::SliceRandom;
use reqwest::Method;

use crate::agent::{Agent, Instance};
use crate::api::{envelope_code, InstanceState, CODE_OK};
use crate::apple::{CatalogItem, Kind};

const MAX_ATTEMPTS: usize = 3;
const RETRY_BUDGET: Duration = Duration::from_secs(45);
pub(crate) const SLOT_WAIT: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub enum RouteError {
    NoInstance,
    Upstream(String),
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouteError::NoInstance => f.write_str("no available instance"),
            RouteError::Upstream(e) => write!(f, "upstream error: {e}"),
        }
    }
}

pub struct Forwarded {
    pub body: Bytes,
    pub code: i64,
    /// Account that answered.
    pub account: String,
}

/// Decrements the in-flight counter on drop.
pub(crate) struct InFlight<'a>(pub(crate) &'a Instance);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Whether a lite business failure is worth retrying on another account.
/// Malformed requests and "no lyrics in that language" are the same
/// everywhere; anything else may be instance-specific.
pub fn should_retry(path: &str, code: i64) -> bool {
    !(code == 400 || (path == "/lyrics" && code == 404))
}

impl Agent {
    /// Running accounts that are not temporarily excluded.
    pub fn serving(&self) -> Vec<Arc<Instance>> {
        self.instances()
            .into_iter()
            .filter(|i| i.state() == InstanceState::Running && i.port().is_some() && !i.circuit_open())
            .collect()
    }

    /// Accounts whose storefront can serve `adam_id`, least busy first. With
    /// `lyrics`, only storefronts whose catalog reports lyrics for the song are
    /// kept when there are any (see `filter_by_catalog`).
    pub async fn candidates(&self, adam_id: &str, lyrics: bool) -> Vec<Arc<Instance>> {
        let mut serving = self.serving();
        serving.shuffle(&mut rand::thread_rng());
        let regions: HashSet<String> = serving.iter().map(|i| i.region()).collect();
        // With a single storefront there is nothing to choose between.
        let mut picked = if regions.len() <= 1 || adam_id == "0" {
            serving
        } else {
            self.filter_by_catalog(serving, &regions, adam_id, lyrics).await
        };
        picked.sort_by_key(|i| i.in_flight.load(Ordering::Relaxed));
        picked
    }

    /// Keeps the accounts whose storefront has the item (songs, then music
    /// videos). For a lyrics request the storefronts whose catalog says
    /// `hasLyrics` are preferred; when none does (or Apple stops sending the
    /// field) every storefront with the song stays, so an account still
    /// answers with the real result. `language` and `script` play no part in
    /// routing: they are forwarded to lite untouched.
    async fn filter_by_catalog(
        &self,
        serving: Vec<Arc<Instance>>,
        regions: &HashSet<String>,
        adam_id: &str,
        lyrics: bool,
    ) -> Vec<Arc<Instance>> {
        for kind in [Kind::Song, Kind::MusicVideo] {
            let probes = regions.iter().map(|r| async move { (r.clone(), self.apple.lookup(kind, r, adam_id).await) });
            let results: Vec<_> = futures_util::future::join_all(probes).await;
            let errors = results.iter().filter(|(_, r)| r.is_err()).count();
            let items: Vec<(String, CatalogItem)> =
                results.into_iter().filter_map(|(r, v)| v.ok().map(|v| (r, v))).collect();
            let regions_where = |pred: fn(&CatalogItem) -> bool| -> HashSet<String> {
                items.iter().filter(|(_, i)| pred(i)).map(|(r, _)| r.clone()).collect()
            };
            let mut ok = regions_where(|i| i.available);
            if lyrics {
                let with_lyrics = regions_where(|i| i.has_lyrics);
                if !with_lyrics.is_empty() {
                    ok = with_lyrics;
                }
            }
            if !ok.is_empty() {
                return serving.into_iter().filter(|i| ok.contains(&i.region())).collect();
            }
            if errors == regions.len() {
                // Apple probing is unavailable (no network to amp-api, token
                // scrape failing...): fall back to trying everyone.
                tracing::debug!("catalog probes failed for {adam_id}; trying every account");
                return serving;
            }
        }
        Vec::new()
    }

    /// Sends one request to one lite instance.
    pub async fn forward(
        &self,
        inst: &Instance,
        method: Method,
        path: &str,
        query: Option<&str>,
        body: Bytes,
    ) -> Result<Forwarded, String> {
        let port = inst.port().ok_or("instance is not running")?;
        let _permit = tokio::time::timeout(SLOT_WAIT, inst.sem.acquire())
            .await
            .map_err(|_| "instance busy".to_string())?
            .map_err(|_| "instance closed".to_string())?;
        inst.in_flight.fetch_add(1, Ordering::Relaxed);
        let _guard = InFlight(inst);

        let mut url = format!("http://127.0.0.1:{port}{path}");
        if let Some(q) = query.filter(|q| !q.is_empty()) {
            url.push('?');
            url.push_str(q);
        }
        let mut req = self.upstream.request(method, url);
        if !body.is_empty() {
            req = req.header("Content-Type", "application/json").body(body);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let body = resp.bytes().await.map_err(|e| e.to_string())?;
        let code = envelope_code(&body);
        if code == CODE_OK && path == "/key" {
            // Real traffic proves the key path works: no probe needed.
            inst.note_key_ok();
        }
        Ok(Forwarded { body, code, account: inst.id.clone() })
    }

    /// Forwards to the given candidates in order with the retry policy. Every
    /// attempt is counted against the account that made it.
    pub async fn forward_with_retry(
        &self,
        candidates: Vec<Arc<Instance>>,
        method: Method,
        path: &str,
        query: Option<&str>,
        body: Bytes,
    ) -> Result<Forwarded, RouteError> {
        if candidates.is_empty() {
            return Err(RouteError::NoInstance);
        }
        let deadline = Instant::now() + RETRY_BUDGET;
        let mut last: Option<Forwarded> = None;
        let mut last_err = None;
        let n = candidates.len();
        let endpoint = path.trim_start_matches('/');
        for (i, inst) in candidates.into_iter().enumerate().take(MAX_ATTEMPTS) {
            if i > 0 && Instant::now() > deadline {
                break;
            }
            let res = self.forward(&inst, method.clone(), path, query, body.clone()).await;
            self.stats.record_attempt(&inst.id, endpoint, matches!(&res, Ok(f) if f.code == CODE_OK));
            match res {
                Ok(f) if f.code == CODE_OK || i + 1 == n || !should_retry(path, f.code) => return Ok(f),
                Ok(f) => {
                    tracing::info!(source = %inst.id, "{path} failed with code {}; trying another account", f.code);
                    last = Some(f);
                }
                Err(e) => {
                    tracing::warn!(source = %inst.id, "{path} transport error: {e}");
                    last_err = Some(e);
                }
            }
        }
        match (last, last_err) {
            (Some(f), _) => Ok(f),
            (None, Some(e)) => Err(RouteError::Upstream(e)),
            (None, None) => Err(RouteError::NoInstance),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_policy() {
        // wrapper-manager's retry_policy_test.go: caller errors and "no lyrics
        // in that language" are the same on every account, the rest is not.
        for (path, code, want) in [
            ("/lyrics", 404, false),
            ("/lyrics", 400, false),
            ("/m3u8", 400, false),
            ("/key", 400, false),
            ("/lyrics", 500, true),
            ("/key", 500, true),
            ("/m3u8", 404, true),
            ("/m3u8", 500, true),
            ("/webplayback", 404, true),
            ("/webplayback", 500, true),
        ] {
            assert_eq!(should_retry(path, code), want, "{path} {code}");
        }
    }
}
