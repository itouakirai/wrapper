//! Public mode (`/api/public/*`, no management token): anyone can add their
//! own Apple Music account and remove it again.
//!
//! * The page shows the service, masked account names and request counters,
//!   never usernames, ids, ports or logs.
//! * A login is accepted only while fewer than `maxAccounts` accounts are
//!   active (enabled and logged in: each one is a lite process), with at most
//!   [`MAX_PENDING_LOGINS`] logins running and a per-address rate limit.
//! * The login answer carries a random *owner key*; only its hash is stored.
//!   Logging out needs the Apple ID and that key, so visitors cannot remove
//!   each other's accounts. Logging in again (which proves the password)
//!   issues a new key.
//! * The 2FA step is bound to a random ticket handed to whoever started the
//!   login, not to the (guessable) account id.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::accounts::{account_id, normalize_username};
use crate::agent::{Agent, Instance};
use crate::api::{CodeRequest, Envelope, InstanceState, LoginRequest, LoginState, LoginView};
use crate::health::Verdict;
use crate::login::{wait_settled, LoginError, LoginSession};
use crate::settings::{mask_username, public_ref, random_hex, sha256_hex};
use crate::stats::{AccountSummary, StatsSummary};

/// Logins one address may start per [`WINDOW`].
const LOGIN_LIMIT: usize = 6;
/// Logout attempts one address may make per [`WINDOW`].
const LOGOUT_LIMIT: usize = 20;
const WINDOW: Duration = Duration::from_secs(30 * 60);
/// Public logins running at the same time (each is a lite process).
pub const MAX_PENDING_LOGINS: usize = 2;
const TICKET_TTL: Duration = Duration::from_secs(20 * 60);
const MAX_CODE_TRIES: u32 = 5;

struct Ticket {
    session: Arc<LoginSession>,
    created: Instant,
    code_tries: u32,
}

#[derive(Default)]
pub struct PublicState {
    tickets: Mutex<HashMap<String, Ticket>>,
    attempts: Mutex<HashMap<(IpAddr, &'static str), VecDeque<Instant>>>,
}

impl PublicState {
    /// Counts an attempt; false when `ip` already used up `limit` in [`WINDOW`].
    fn allow(&self, ip: IpAddr, kind: &'static str, limit: usize) -> bool {
        let mut map = self.attempts.lock().unwrap();
        map.retain(|_, q| q.back().is_some_and(|t| t.elapsed() < WINDOW));
        let q = map.entry((ip, kind)).or_default();
        while q.front().is_some_and(|t| t.elapsed() >= WINDOW) {
            q.pop_front();
        }
        if q.len() >= limit {
            return false;
        }
        q.push_back(Instant::now());
        true
    }

    fn add_ticket(&self, session: Arc<LoginSession>) -> String {
        let id = random_hex(16);
        let mut map = self.tickets.lock().unwrap();
        map.retain(|_, t| t.created.elapsed() < TICKET_TTL);
        map.insert(id.clone(), Ticket { session, created: Instant::now(), code_tries: 0 });
        id
    }

    fn ticket(&self, id: &str) -> Option<Arc<LoginSession>> {
        let map = self.tickets.lock().unwrap();
        map.get(id).filter(|t| t.created.elapsed() < TICKET_TTL).map(|t| t.session.clone())
    }

    /// Counts a code attempt; false once the ticket used up its tries.
    fn count_code_try(&self, id: &str) -> bool {
        let mut map = self.tickets.lock().unwrap();
        match map.get_mut(id) {
            Some(t) => {
                t.code_tries += 1;
                t.code_tries <= MAX_CODE_TRIES
            }
            None => false,
        }
    }
}

/// The caller's address: the TCP peer, or behind a proxy on this machine
/// (the GUI, nginx) the last non-loopback `X-Forwarded-For` entry.
pub struct ClientIp(pub IpAddr);

pub fn client_ip(peer: Option<IpAddr>, headers: &HeaderMap) -> IpAddr {
    match peer {
        Some(p) if !p.is_loopback() => p,
        _ => headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .filter_map(|s| s.trim().parse::<IpAddr>().ok())
            .rfind(|ip| !ip.is_loopback())
            .or(peer)
            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
        Ok(ClientIp(client_ip(peer, &parts.headers)))
    }
}

impl Agent {
    /// Enabled and logged in: runs (or is about to run) a lite process.
    fn is_active(inst: &Instance) -> bool {
        inst.record().enabled && inst.state() != InstanceState::NeedsLogin
    }

    pub fn active_accounts(&self) -> usize {
        self.instances().iter().filter(|i| Self::is_active(i)).count()
    }

    /// Whether a public login for a new (or logged-out) account fits under
    /// `maxAccounts`, counting logins of such accounts that are under way.
    pub fn has_capacity(&self) -> bool {
        let max = self.settings().max_accounts as usize;
        if max == 0 {
            return true;
        }
        let pending = self
            .active_sessions()
            .iter()
            .filter(|s| !self.instance(&s.id).is_some_and(|i| Self::is_active(&i)))
            .count();
        self.active_accounts() + pending < max
    }
}

type AppState = Arc<Agent>;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/public/overview", get(overview))
        .route("/api/public/login", post(login))
        .route("/api/public/login/{ticket}", get(login_status).delete(login_cancel))
        .route("/api/public/login/{ticket}/code", post(login_code))
        .route("/api/public/logout", post(logout))
}

fn fail(status: StatusCode, msg: impl Into<String>) -> Envelope {
    Envelope::fail(msg).status(status)
}

fn closed() -> Envelope {
    fail(StatusCode::FORBIDDEN, "public mode is off")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicAccount {
    #[serde(rename = "ref")]
    handle: String,
    name: String,
    region: String,
    state: InstanceState,
    verdict: Option<Verdict>,
    degraded: bool,
    since: i64,
    usage: AccountSummary,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicOverview {
    public_mode: bool,
    version: &'static str,
    ready: bool,
    notice: String,
    max_accounts: u32,
    active_accounts: usize,
    /// A new account would be accepted right now.
    accepting: bool,
    regions: Vec<String>,
    accounts: Vec<PublicAccount>,
    stats: StatsSummary,
}

async fn overview(State(a): State<AppState>) -> Envelope {
    let settings = a.settings();
    if !settings.public_mode {
        return Envelope::ok(json!({ "publicMode": false, "version": env!("CARGO_PKG_VERSION") }));
    }
    let usage = a.stats.accounts();
    let mut regions: Vec<String> = Vec::new();
    let accounts = a
        .instances()
        .iter()
        .map(|i| {
            let rec = i.record();
            let state = i.state();
            if state == InstanceState::Running && !rec.region.is_empty() && !regions.contains(&rec.region) {
                regions.push(rec.region.clone());
            }
            PublicAccount {
                handle: public_ref(&settings.public_salt, &rec.id),
                name: mask_username(&rec.username),
                region: rec.region,
                state,
                verdict: i.health_view().verdict.filter(|_| state == InstanceState::Running),
                degraded: i.circuit_open(),
                since: rec.created_at,
                usage: usage.get(&rec.id).cloned().unwrap_or_default(),
            }
        })
        .collect();
    regions.sort();
    Envelope::ok(PublicOverview {
        public_mode: true,
        version: env!("CARGO_PKG_VERSION"),
        ready: a.is_ready(),
        notice: settings.public_notice,
        max_accounts: settings.max_accounts,
        active_accounts: a.active_accounts(),
        accepting: a.has_capacity() && !a.restoring.load(Ordering::Relaxed),
        regions,
        accounts,
        stats: a.stats.summary(),
    })
}

#[derive(Deserialize)]
struct PublicLoginRequest {
    username: String,
    password: String,
}

fn login_json(v: &LoginView) -> serde_json::Value {
    json!({ "state": v.state, "message": v.message })
}

fn login_failure(e: LoginError) -> Envelope {
    match e {
        LoginError::Busy => fail(StatusCode::CONFLICT, e.to_string()),
        LoginError::NoSession => fail(StatusCode::NOT_FOUND, e.to_string()),
        LoginError::Io(_) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        LoginError::Invalid(_) => fail(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

async fn login(
    State(a): State<AppState>,
    ClientIp(ip): ClientIp,
    body: Result<Json<PublicLoginRequest>, JsonRejection>,
) -> Envelope {
    let settings = a.settings();
    if !settings.public_mode {
        return closed();
    }
    let Json(req) = match body {
        Ok(b) => b,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e.body_text()),
    };
    let username = normalize_username(&req.username);
    if username.is_empty() || req.password.is_empty() {
        return fail(StatusCode::BAD_REQUEST, "username and password are required");
    }
    if a.restoring.load(Ordering::Relaxed) {
        return fail(StatusCode::SERVICE_UNAVAILABLE, "a backup is being restored; try again in a minute");
    }
    let id = account_id(&username);
    if a.instance(&id).is_some_and(|i| i.state() != InstanceState::NeedsLogin) {
        return fail(
            StatusCode::CONFLICT,
            "this Apple ID is already on this server; log it out with its key first to log in again",
        );
    }
    if a.session(&id).is_some_and(|s| s.is_active()) {
        return login_failure(LoginError::Busy);
    }
    if a.active_sessions().len() >= MAX_PENDING_LOGINS {
        return fail(StatusCode::TOO_MANY_REQUESTS, "other logins are in progress; try again in a minute");
    }
    if !a.has_capacity() {
        return fail(
            StatusCode::CONFLICT,
            format!("this server is full ({} accounts); try again later", settings.max_accounts),
        );
    }
    if !a.public.allow(ip, "login", LOGIN_LIMIT) {
        return fail(StatusCode::TOO_MANY_REQUESTS, "too many login attempts; try again later");
    }
    let key = random_hex(16);
    let req = LoginRequest { username: username.clone(), password: req.password, device_info: None, code: None };
    tracing::info!("public login for {} from {ip}", mask_username(&username));
    let session = match a.start_login(req, Some(sha256_hex(&key)), false).await {
        Ok(s) => s,
        Err(e) => return login_failure(e),
    };
    let ticket = a.public.add_ticket(session.clone());
    let view = wait_settled(&session).await;
    Envelope::ok(json!({
        "ticket": ticket,
        "ref": public_ref(&settings.public_salt, &id),
        "name": mask_username(&username),
        "ownerKey": key,
        "login": login_json(&view),
    }))
}

async fn login_status(State(a): State<AppState>, Path(ticket): Path<String>) -> Envelope {
    match a.public.ticket(&ticket) {
        Some(s) => Envelope::ok(login_json(&s.view())),
        None => fail(StatusCode::NOT_FOUND, "this login has expired; start again"),
    }
}

async fn login_code(
    State(a): State<AppState>,
    Path(ticket): Path<String>,
    body: Result<Json<CodeRequest>, JsonRejection>,
) -> Envelope {
    let Some(session) = a.public.ticket(&ticket) else {
        return fail(StatusCode::NOT_FOUND, "this login has expired; start again");
    };
    let Json(req) = match body {
        Ok(b) => b,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e.body_text()),
    };
    if !a.public.count_code_try(&ticket) {
        cancel(&a, &session);
        return fail(StatusCode::TOO_MANY_REQUESTS, "too many wrong codes; start the login again");
    }
    match a.submit_code_to(&session, &req.code).await {
        Ok(v) => Envelope::ok(login_json(&v)),
        Err(e) => login_failure(e),
    }
}

fn cancel(a: &Agent, session: &Arc<LoginSession>) {
    if a.session(&session.id).is_some_and(|s| Arc::ptr_eq(&s, session)) && session.is_active() {
        a.cancel_login(&session.id);
    }
}

async fn login_cancel(State(a): State<AppState>, Path(ticket): Path<String>) -> Envelope {
    if let Some(s) = a.public.ticket(&ticket) {
        cancel(&a, &s);
    }
    Envelope::ok(serde_json::Value::Null)
}

#[derive(Deserialize)]
struct PublicLogoutRequest {
    username: String,
    key: String,
}

fn hash_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn logout(
    State(a): State<AppState>,
    ClientIp(ip): ClientIp,
    body: Result<Json<PublicLogoutRequest>, JsonRejection>,
) -> Envelope {
    if !a.settings().public_mode {
        return closed();
    }
    let Json(req) = match body {
        Ok(b) => b,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e.body_text()),
    };
    if !a.public.allow(ip, "logout", LOGOUT_LIMIT) {
        return fail(StatusCode::TOO_MANY_REQUESTS, "too many attempts; try again later");
    }
    let id = account_id(&req.username);
    let hash = sha256_hex(req.key.trim());
    let owned = a
        .instance(&id)
        .and_then(|i| i.record().owner_key_hash)
        .is_some_and(|stored| hash_eq(&stored, &hash));
    if !owned {
        return fail(StatusCode::FORBIDDEN, "the Apple ID and key do not match an account on this server");
    }
    if a.session(&id).is_some_and(|s| s.state() == LoginState::Running) {
        return login_failure(LoginError::Busy);
    }
    a.remove_account(&id).await;
    tracing::info!("public logout of {} from {ip}", mask_username(&req.username));
    Envelope::ok(json!({ "removed": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_ip_prefers_peer_then_forwarded() {
        let mut h = HeaderMap::new();
        let lan: IpAddr = "192.168.1.5".parse().unwrap();
        let lo: IpAddr = "127.0.0.1".parse().unwrap();
        assert_eq!(client_ip(Some(lan), &h), lan);
        assert_eq!(client_ip(Some(lo), &h), lo);
        h.insert("x-forwarded-for", "1.1.1.1, 2.2.2.2, 127.0.0.1".parse().unwrap());
        // A forged first entry does not win over what the proxy appended.
        assert_eq!(client_ip(Some(lo), &h), "2.2.2.2".parse::<IpAddr>().unwrap());
        // Only a local proxy is trusted to say who the client is.
        assert_eq!(client_ip(Some(lan), &h), lan);
    }

    #[test]
    fn rate_limit_is_per_address_and_kind() {
        let p = PublicState::default();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        for _ in 0..3 {
            assert!(p.allow(ip, "login", 3));
        }
        assert!(!p.allow(ip, "login", 3));
        assert!(p.allow(ip, "logout", 3));
        assert!(p.allow("10.0.0.2".parse().unwrap(), "login", 3));
    }
}
