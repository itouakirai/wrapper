//! `wrapper gui`: one port (Settings → Network, default 12340) for everything:
//!
//! * the wrapper-lite API (`/m3u8`, `/key`, ... `/login`, `/logout`), passed
//!   through to the agent unchanged;
//! * `/api/public/*` — public mode, open to everyone (the agent enforces it);
//! * `/api/host/*` — GUI backend (config, engine, console, package download),
//!   administrators only;
//! * `/api/*` — the agent's management API, administrators only (the GUI adds
//!   the agent's token);
//! * everything else — the embedded single-page app.
//!
//! The agent itself listens on a private loopback port (the QEMU port forward
//! or the native agent), so only this front door is exposed.
//!
//! Administrators are requests from this machine (unless `--access-token` is
//! given, as on Android where every app shares 127.0.0.1), or requests with
//! the management token (cookie from `POST /api/session` or `?token=`,
//! `Authorization: Bearer`).

pub mod config;
pub mod engine;
pub mod package;

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use config::{config_path, native_supported, GuiConfig};
use engine::{Engine, EngineState, HostEvent, Paths};

mod assets {
    include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));
}

/// Same cookie as the headless agent's `POST /api/session`.
const COOKIE: &str = "wrapper_admin";

/// The wrapper-lite / wrapper-manager API, passed through to the agent.
const RESOURCE_PATHS: &[&str] =
    &["/status", "/health", "/m3u8", "/key", "/lyrics", "/webplayback", "/license", "/login", "/logout"];

#[derive(Clone, Debug)]
pub struct GuiOptions {
    /// Listen address from the command line; replaces (and updates) the one
    /// in the settings.
    pub listen: Option<SocketAddr>,
    pub paths: Paths,
    pub open_browser: bool,
    /// Extra admin token. When set, requests from this machine need a token
    /// too (Android, where every app can reach 127.0.0.1).
    pub access_token: Option<String>,
}

struct Server {
    addr: SocketAddr,
    stop: oneshot::Sender<()>,
}

pub struct Gui {
    opts: GuiOptions,
    config: Mutex<GuiConfig>,
    pub engine: Arc<Engine>,
    proxy_http: reqwest::Client,
    downloading: AtomicBool,
    /// Newer release found by the background check.
    update: Mutex<Option<Value>>,
    server: Mutex<Option<Server>>,
}

type AppState = Arc<Gui>;

fn reply(status: StatusCode, code: i64, msg: &str, data: Value) -> Response {
    (status, Json(json!({ "code": code, "msg": msg, "data": data }))).into_response()
}

fn ok(data: impl serde::Serialize) -> Response {
    reply(StatusCode::OK, 0, "SUCCESS", serde_json::to_value(data).unwrap_or(Value::Null))
}

fn fail(status: StatusCode, msg: &str) -> Response {
    reply(status, -1, msg, Value::Null)
}

fn listen_addr(cfg: &GuiConfig) -> Option<SocketAddr> {
    Some(SocketAddr::new(cfg.api_host.parse().ok()?, cfg.api_port))
}

/// `http://host:port` for an address this machine can open.
fn local_url(addr: SocketAddr) -> String {
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => format!("http://127.0.0.1:{}", addr.port()),
        IpAddr::V6(ip) if ip.is_unspecified() => format!("http://[::1]:{}", addr.port()),
        IpAddr::V6(ip) => format!("http://[{ip}]:{}", addr.port()),
        IpAddr::V4(ip) => format!("http://{ip}:{}", addr.port()),
    }
}

/// Everything but the listen address and GUI-only options: what the
/// running engine was started with.
fn engine_settings(cfg: &GuiConfig) -> GuiConfig {
    GuiConfig {
        api_host: String::new(),
        api_port: 0,
        autostart: false,
        mirror: false,
        package_base_url: String::new(),
        ..cfg.clone()
    }
}

impl Gui {
    pub fn new(opts: GuiOptions) -> Arc<Self> {
        let path = config_path(&opts.paths.data_dir);
        let mut config = GuiConfig::load(&path);
        if let Some(l) = opts.listen {
            config.api_host = l.ip().to_string();
            config.api_port = l.port();
            if let Err(e) = config.save(&path) {
                tracing::warn!("cannot save settings: {e}");
            }
        }
        Arc::new(Self {
            opts,
            config: Mutex::new(config),
            engine: Engine::new(),
            proxy_http: reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(5))
                .build()
                .expect("http client"),
            downloading: AtomicBool::new(false),
            update: Mutex::new(None),
            server: Mutex::new(None),
        })
    }

    pub fn config(&self) -> GuiConfig {
        self.config.lock().unwrap().clone()
    }

    /// Where the panel and API are being served.
    pub fn bound_addr(&self) -> Option<SocketAddr> {
        self.server.lock().unwrap().as_ref().map(|s| s.addr)
    }

    /// The API endpoint as clients on this machine reach it.
    pub fn api_url(&self) -> String {
        match self.bound_addr() {
            Some(a) => local_url(a),
            None => self.config().api_url(),
        }
    }

    /// Panel URL to print / open; carries the token when one is needed.
    pub fn panel_url(&self) -> String {
        let mut url = format!("{}/", self.api_url());
        let remote = self.bound_addr().is_some_and(|a| !a.ip().is_loopback());
        if let Some(t) = &self.opts.access_token {
            url.push_str(&format!("?token={t}"));
        } else if remote {
            url.push_str(&format!("?token={}", self.config().api_token));
        }
        url
    }

    pub async fn start_engine(&self) -> Result<(), String> {
        self.engine.start(self.config(), self.opts.paths.clone(), self.api_url()).await
    }

    fn admin_tokens(&self) -> Vec<String> {
        let mut v = vec![self.config().api_token];
        v.extend(self.opts.access_token.clone());
        v.retain(|t| !t.is_empty());
        v
    }

    fn token_ok(&self, presented: &str) -> bool {
        self.admin_tokens().iter().any(|t| token_eq(presented, t))
    }

    /// Whether the request carries a valid admin token.
    fn signed_in(&self, headers: &HeaderMap, query: Option<&str>) -> bool {
        presented_tokens(headers, query).iter().any(|t| self.token_ok(t))
    }

    fn is_admin(&self, headers: &HeaderMap, query: Option<&str>, peer: Option<SocketAddr>) -> bool {
        if self.signed_in(headers, query) {
            return true;
        }
        self.opts.access_token.is_none() && peer.is_some_and(|p| trusted_local(headers, p))
    }

    pub fn router(self: &Arc<Self>) -> Router {
        let admin = Router::new()
            .route("/api/host/info", get(info))
            .route("/api/host/config", get(get_config).put(put_config))
            .route("/api/host/status", get(status))
            .route("/api/host/start", post(start))
            .route("/api/host/stop", post(stop))
            .route("/api/host/restart", post(restart))
            .route("/api/host/console", get(console))
            .route("/api/host/events", get(events))
            .route("/api/host/package/download", post(download))
            .route("/api/{*path}", any(agent_proxy))
            .route_layer(middleware::from_fn_with_state(self.clone(), require_admin));
        let mut open = Router::new()
            .route("/api/whoami", get(whoami))
            .route("/api/session", post(session_login).delete(session_logout))
            .route("/api/public/{*path}", any(public_proxy));
        for path in RESOURCE_PATHS {
            open = open.route(path, any(passthrough));
        }
        open.merge(admin)
            .fallback(get(static_file))
            .layer(middleware::from_fn_with_state(self.clone(), token_link))
            .with_state(self.clone())
    }

    /// Serves the router on `listener`, replacing the current server.
    fn spawn_server(self: &Arc<Self>, listener: tokio::net::TcpListener) -> std::io::Result<SocketAddr> {
        let addr = listener.local_addr()?;
        let (stop, stopped) = oneshot::channel::<()>();
        let app = self.router().into_make_service_with_connect_info::<SocketAddr>();
        tokio::spawn(async move {
            let serve = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = stopped.await;
            });
            if let Err(e) = serve.await {
                tracing::warn!("panel server on {addr} failed: {e}");
            }
        });
        if let Some(old) = self.server.lock().unwrap().replace(Server { addr, stop }) {
            // Stops accepting at once; requests in flight (this one) finish.
            let _ = old.stop.send(());
        }
        Ok(addr)
    }

    /// Moves the panel and API to `addr` (Settings → Network).
    async fn rebind(self: &Arc<Self>, addr: SocketAddr) -> std::io::Result<()> {
        let current = self.bound_addr();
        if current == Some(addr) {
            return Ok(());
        }
        match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => return self.spawn_server(l).map(|_| ()),
            // 127.0.0.1:p -> 0.0.0.0:p: the old socket has to go first.
            Err(e) if current.is_some_and(|c| c.port() == addr.port()) && e.kind() == std::io::ErrorKind::AddrInUse => {}
            Err(e) => return Err(e),
        }
        let old = self.server.lock().unwrap().take();
        if let Some(old) = old {
            let _ = old.stop.send(());
        }
        let mut last = None;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            match tokio::net::TcpListener::bind(addr).await {
                Ok(l) => return self.spawn_server(l).map(|_| ()),
                Err(e) => last = Some(e),
            }
        }
        // Go back to where we were.
        if let Some(prev) = current {
            if let Ok(l) = tokio::net::TcpListener::bind(prev).await {
                let _ = self.spawn_server(l);
            }
        }
        Err(last.unwrap_or_else(|| std::io::Error::other("cannot bind")))
    }

    fn stop_server(&self) {
        if let Some(s) = self.server.lock().unwrap().take() {
            let _ = s.stop.send(());
        }
    }
}

// --------------------------------------------------------------- access

fn token_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn query_token(query: Option<&str>) -> Option<&str> {
    query.and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")))
}

fn presented_tokens(headers: &HeaderMap, query: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().strip_prefix(COOKIE)?.strip_prefix('=').map(str::to_string))
        .collect();
    if let Some(b) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) {
        out.push(b.to_string());
    }
    out.extend(query_token(query).map(str::to_string));
    out
}

/// `localhost`, `127.x.x.x`, `[::1]`, with or without a port.
fn is_loopback_host(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host.rsplit_once(':').map_or(host, |(h, _)| h)
    };
    name.eq_ignore_ascii_case("localhost") || name.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// A request a person on this machine made through the panel: from a
/// loopback peer, not relayed by a local reverse proxy, addressed to a
/// loopback name (a DNS-rebinding page would send its own host) and not
/// sent by another site open in the browser.
fn trusted_local(headers: &HeaderMap, peer: SocketAddr) -> bool {
    if !peer.ip().is_loopback() {
        return false;
    }
    if ["x-forwarded-for", "forwarded", "x-real-ip"].iter().any(|h| headers.contains_key(*h)) {
        return false;
    }
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    if !is_loopback_host(host) {
        return false;
    }
    if headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) == Some("cross-site") {
        return false;
    }
    match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        None => true,
        Some(origin) => origin.strip_prefix("http://").is_some_and(|o| o.eq_ignore_ascii_case(host)),
    }
}

fn peer_of(req: &Request) -> Option<SocketAddr> {
    req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0)
}

async fn require_admin(State(g): State<AppState>, req: Request, next: Next) -> Response {
    if g.is_admin(req.headers(), req.uri().query(), peer_of(&req)) {
        return next.run(req).await;
    }
    fail(StatusCode::UNAUTHORIZED, "unauthorized")
}

fn cookie_header(value: &str, max_age: u64) -> Option<HeaderValue> {
    HeaderValue::from_str(&format!("{COOKIE}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}")).ok()
}

/// `/?token=...` (the URL printed at start-up) signs the browser in.
async fn token_link(State(g): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if req.method() == Method::GET && !path.starts_with("/api/") && !RESOURCE_PATHS.contains(&path) {
        if let Some(t) = query_token(req.uri().query()).filter(|t| g.token_ok(t)) {
            let mut res = Redirect::to(path).into_response();
            if let Some(v) = cookie_header(t, 365 * 86_400) {
                res.headers_mut().insert(header::SET_COOKIE, v);
            }
            return res;
        }
    }
    next.run(req).await
}

/// `GET /api/whoami`: what the panel should show this visitor.
async fn whoami(State(g): State<AppState>, req: Request) -> Response {
    let (headers, query, peer) = (req.headers(), req.uri().query(), peer_of(&req));
    ok(json!({
        "mode": "gui",
        "admin": g.is_admin(headers, query, peer),
        "signedIn": g.signed_in(headers, query),
        "tokenRequired": true,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(Deserialize)]
struct SessionRequest {
    token: String,
}

async fn session_login(State(g): State<AppState>, body: Result<Json<SessionRequest>, axum::extract::rejection::JsonRejection>) -> Response {
    let Ok(Json(req)) = body else {
        return fail(StatusCode::BAD_REQUEST, "expected {\"token\"}");
    };
    let token = req.token.trim();
    if !g.token_ok(token) {
        tokio::time::sleep(Duration::from_secs(1)).await;
        return fail(StatusCode::UNAUTHORIZED, "wrong token");
    }
    let mut res = ok(Value::Null);
    match cookie_header(token, 30 * 86_400) {
        Some(v) => {
            res.headers_mut().insert(header::SET_COOKIE, v);
            res
        }
        None => fail(StatusCode::BAD_REQUEST, "this token cannot be stored in a cookie"),
    }
}

async fn session_logout() -> Response {
    let mut res = ok(Value::Null);
    if let Some(v) = cookie_header("", 0) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    res
}

// ----------------------------------------------------------- host API

/// Primary LAN address (the one used for the default route).
fn lan_address() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

async fn info(State(g): State<AppState>) -> Response {
    let cfg = g.config();
    let qcfg = engine::qemu_config(&cfg, &g.opts.paths);
    ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "nativeSupported": native_supported(),
        "rootfsReady": g.opts.paths.rootfs.join("system/bin/lite").is_file(),
        "qemu": crate::qemu::check_assets(&qcfg),
        "packageName": package::package_name(),
        "packageUrl": package::package_url(&cfg.package_base_url, cfg.mirror),
        "downloading": g.downloading.load(Ordering::Relaxed),
        "dataDir": g.opts.paths.data_dir,
        "assetsDir": g.opts.paths.assets_dir,
        "lanAddress": lan_address(),
        "listen": g.bound_addr().map(|a| a.to_string()),
        "apiUrl": g.api_url(),
        "accessProtected": g.opts.access_token.is_some(),
        "update": g.update.lock().unwrap().clone(),
    }))
}

async fn get_config(State(g): State<AppState>) -> Response {
    ok(g.config())
}

/// Merges the given fields into the current settings (partial updates keep
/// everything else, including the token; an empty token regenerates it). A
/// new listen address takes effect at once.
async fn put_config(State(g): State<AppState>, body: Result<Json<Value>, axum::extract::rejection::JsonRejection>) -> Response {
    let Json(patch) = match body {
        Ok(b) => b,
        Err(e) => return fail(StatusCode::BAD_REQUEST, &e.body_text()),
    };
    let Value::Object(patch) = patch else {
        return fail(StatusCode::BAD_REQUEST, "expected a JSON object");
    };
    let old = g.config();
    let mut merged = serde_json::to_value(&old).unwrap_or_default();
    if let Value::Object(m) = &mut merged {
        m.extend(patch);
    }
    let mut new: GuiConfig = match serde_json::from_value(merged) {
        Ok(c) => c,
        Err(e) => return fail(StatusCode::BAD_REQUEST, &format!("invalid settings: {e}")),
    };
    if new.api_token.trim().is_empty() {
        new.api_token = config::random_token();
    }
    if let Err(e) = new.validate() {
        return fail(StatusCode::BAD_REQUEST, &e);
    }
    if let Some(addr) = listen_addr(&new).filter(|a| listen_addr(&old) != Some(*a)) {
        if let Err(e) = g.rebind(addr).await {
            return fail(StatusCode::BAD_REQUEST, &format!("cannot listen on {addr}: {e}"));
        }
        tracing::info!("panel and API moved to {addr}");
        g.engine.set_public_url(g.api_url());
    }
    if let Err(e) = new.save(&config_path(&g.opts.paths.data_dir)) {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, &format!("cannot save settings: {e}"));
    }
    *g.config.lock().unwrap() = new.clone();
    let running = matches!(g.engine.status().state, EngineState::Running | EngineState::Starting);
    let restart = running && engine_settings(&old) != engine_settings(&new);
    ok(json!({ "config": new, "restartRequired": restart, "apiUrl": g.api_url() }))
}

async fn status(State(g): State<AppState>) -> Response {
    ok(g.engine.status())
}

async fn start(State(g): State<AppState>) -> Response {
    match g.start_engine().await {
        Ok(()) => ok(g.engine.status()),
        Err(e) => fail(StatusCode::BAD_REQUEST, &e),
    }
}

async fn stop(State(g): State<AppState>) -> Response {
    g.engine.stop().await;
    ok(g.engine.status())
}

async fn restart(State(g): State<AppState>) -> Response {
    g.engine.stop().await;
    match g.start_engine().await {
        Ok(()) => ok(g.engine.status()),
        Err(e) => fail(StatusCode::BAD_REQUEST, &e),
    }
}

#[derive(Deserialize)]
struct ConsoleQuery {
    after: Option<u64>,
    limit: Option<usize>,
}

async fn console(State(g): State<AppState>, Query(q): Query<ConsoleQuery>) -> Response {
    ok(g.engine.console(q.after, q.limit.unwrap_or(1000).min(4000)))
}

async fn events(State(g): State<AppState>) -> impl IntoResponse {
    let stream = BroadcastStream::new(g.engine.subscribe()).filter_map(|ev| {
        let ev = ev.ok()?;
        Some(Ok::<_, Infallible>(SseEvent::default().json_data(ev).ok()?))
    });
    // Reverse proxies (nginx) buffer responses by default, which holds the
    // stream back; this tells them not to.
    ([("x-accel-buffering", "no")], Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn download(State(g): State<AppState>) -> Response {
    let cfg = g.config();
    let Some(url) = package::package_url(&cfg.package_base_url, cfg.mirror) else {
        return fail(StatusCode::BAD_REQUEST, "no prebuilt package for this platform");
    };
    if matches!(g.engine.status().state, EngineState::Running | EngineState::Starting) {
        return fail(StatusCode::CONFLICT, "stop the engine before updating the package");
    }
    if g.downloading.swap(true, Ordering::SeqCst) {
        return fail(StatusCode::CONFLICT, "a download is already running");
    }
    // Packages contain `qemu/...`: install next to the assets dir (the exe
    // dir on desktops; the app's files dir on Android, where the exe dir is
    // read-only).
    let dest = g.opts.paths.assets_dir.parent().map(PathBuf::from).unwrap_or_else(crate::exe_dir);
    let gui = g.clone();
    let task_url = url.clone();
    tokio::spawn(async move {
        let url = task_url;
        let engine = gui.engine.clone();
        engine.console_line(&format!("[gui] downloading {url}"));
        let ev = engine.clone();
        let proxy = Some(cfg.proxy.clone()).filter(|p| !p.is_empty());
        let result = package::download_and_install(&url, &dest, proxy.as_deref(), move |p| {
            ev.emit(HostEvent::Download { progress: p });
        })
        .await;
        if let Err(e) = result {
            let msg = format!("{e:#}");
            engine.console_line(&format!("[gui] download failed: {msg}"));
            engine.emit(HostEvent::Download {
                progress: package::DownloadProgress {
                    status: "error",
                    downloaded: 0,
                    total: None,
                    speed: 0,
                    message: Some(msg),
                },
            });
        } else {
            engine.console_line("[gui] package installed");
        }
        gui.downloading.store(false, Ordering::SeqCst);
    });
    ok(json!({ "url": url }))
}

// ----------------------------------------------------------- agent proxy

#[derive(Clone, Copy, PartialEq, Eq)]
enum Auth {
    /// Management API: the GUI adds the agent's token.
    Admin,
    /// Public mode: no credentials at all.
    Public,
    /// The resource API: the client's own `Authorization` (for `/login`,
    /// `/logout`, `/health?probe=1`).
    Client,
}

/// `/api/*` -> the agent's management API, with its token.
async fn agent_proxy(State(g): State<AppState>, Path(path): Path<String>, req: Request) -> Response {
    proxy(&g, &format!("api/{path}"), Auth::Admin, req).await
}

/// `/api/public/*` -> the agent's public-mode API.
async fn public_proxy(State(g): State<AppState>, Path(path): Path<String>, req: Request) -> Response {
    proxy(&g, &format!("api/public/{path}"), Auth::Public, req).await
}

/// The wrapper-lite API, unchanged.
async fn passthrough(State(g): State<AppState>, req: Request) -> Response {
    let path = req.uri().path().trim_start_matches('/').to_string();
    proxy(&g, &path, Auth::Client, req).await
}

const HOP_BY_HOP: &[&str] =
    &["connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "te", "trailer", "transfer-encoding", "upgrade"];

async fn proxy(g: &Gui, path: &str, auth: Auth, req: Request) -> Response {
    let Some((base, token)) = g.engine.current_api() else {
        return fail(StatusCode::SERVICE_UNAVAILABLE, "the engine is not running");
    };
    let peer = peer_of(&req);
    let (parts, body) = req.into_parts();
    let mut url = format!("{base}/{path}");
    if let Some(q) = parts.uri.query().filter(|q| !q.is_empty()) {
        url.push('?');
        url.push_str(q);
    }
    let Ok(method) = reqwest::Method::from_bytes(parts.method.as_str().as_bytes()) else {
        return fail(StatusCode::METHOD_NOT_ALLOWED, "bad method");
    };
    let mut rb = g.proxy_http.request(method, url);
    for name in [header::CONTENT_TYPE, header::ACCEPT, header::ORIGIN, header::ACCESS_CONTROL_REQUEST_METHOD, header::ACCESS_CONTROL_REQUEST_HEADERS] {
        if let Some(v) = parts.headers.get(&name) {
            rb = rb.header(name.as_str(), v.as_bytes());
        }
    }
    match auth {
        Auth::Admin => rb = rb.bearer_auth(token),
        Auth::Client => {
            if let Some(v) = parts.headers.get(header::AUTHORIZATION) {
                rb = rb.header("Authorization", v.as_bytes());
            }
        }
        Auth::Public => {}
    }
    // The agent rate-limits public logins per client address.
    let forwarded = parts.headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).map(str::to_string);
    if let Some(peer) = peer {
        let chain = match forwarded {
            Some(f) => format!("{f}, {}", peer.ip()),
            None => peer.ip().to_string(),
        };
        rb = rb.header("X-Forwarded-For", chain);
    }
    let has_body = parts.headers.contains_key(header::TRANSFER_ENCODING)
        || parts
            .headers
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v != "0");
    if has_body {
        // Streamed: restoring a backup uploads more than fits in memory.
        rb = rb.body(reqwest::Body::wrap_stream(body.into_data_stream()));
    }
    let resp = match rb.send().await {
        Ok(r) => r,
        Err(e) => {
            let msg = if e.is_connect() { "the agent is not reachable yet".to_string() } else { e.to_string() };
            return fail(StatusCode::BAD_GATEWAY, &msg);
        }
    };
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut headers = HeaderMap::new();
    for (name, value) in resp.headers() {
        let n = name.as_str();
        // Only the open API may be read by other sites' scripts.
        if HOP_BY_HOP.contains(&n) || (auth == Auth::Admin && n.starts_with("access-control-")) {
            continue;
        }
        if let (Ok(n), Ok(v)) = (header::HeaderName::from_bytes(n.as_bytes()), HeaderValue::from_bytes(value.as_bytes())) {
            headers.append(n, v);
        }
    }
    let mut out = Response::new(Body::from_stream(resp.bytes_stream()));
    *out.status_mut() = status;
    *out.headers_mut() = headers;
    if auth != Auth::Client {
        out.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    out
}

// ----------------------------------------------------------- static files

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "json" | "webmanifest" => "application/json",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Serves the embedded panel (also used by the headless agent).
pub async fn static_file(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let (name, bytes) = match assets::ASSETS.iter().find(|(p, _)| *p == path) {
        Some((p, b)) => (*p, *b),
        // Single-page app: unknown paths render the shell.
        None if !path.starts_with("api/") && !path.starts_with("assets/") => match assets::ASSETS.iter().find(|(p, _)| *p == "index.html") {
            Some((p, b)) => (*p, *b),
            None => return fail(StatusCode::NOT_FOUND, "not found"),
        },
        None => return fail(StatusCode::NOT_FOUND, "not found"),
    };
    // Hashed bundle files can be cached forever; the shell must revalidate.
    let cache = if name.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    ([(header::CONTENT_TYPE, mime(name)), (header::CACHE_CONTROL, cache)], bytes).into_response()
}

/// The panel as a router, for the headless agent to answer unknown paths with.
pub fn ui_router() -> Router {
    Router::new().fallback(get(static_file))
}

// ----------------------------------------------------------------- run

fn open_browser(url: &str) {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/c", "start", "", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    let _ = cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

/// Runs the GUI until `shutdown` resolves, then stops the engine.
pub async fn run(opts: GuiOptions, shutdown: impl std::future::Future<Output = ()> + Send + 'static) -> anyhow::Result<()> {
    let gui = Gui::new(opts.clone());
    let cfg = gui.config();
    let want = listen_addr(&cfg).ok_or_else(|| anyhow::anyhow!("invalid listen address {:?}", cfg.api_host))?;
    let listener = match tokio::net::TcpListener::bind(want).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("cannot listen on {want} ({e}); using a free port");
            tokio::net::TcpListener::bind(SocketAddr::new(want.ip(), 0)).await?
        }
    };
    gui.spawn_server(listener)?;
    let url = gui.panel_url();
    println!("wrapper {} panel: {url}", env!("CARGO_PKG_VERSION"));
    println!("wrapper-lite API: {}", gui.api_url());
    if opts.open_browser {
        open_browser(&url);
    }
    if std::env::var_os("WRAPPER_NO_UPDATE_CHECK").is_none() {
        tokio::spawn(check_updates(gui.clone()));
    }
    if cfg.autostart {
        if let Err(e) = gui.start_engine().await {
            tracing::warn!("autostart failed: {e}");
        }
    }
    shutdown.await;
    println!("stopping...");
    gui.engine.stop().await;
    gui.stop_server();
    tokio::time::sleep(Duration::from_millis(200)).await;
    Ok(())
}

/// Looks for a newer release at start-up and then daily; only reports it
/// (installing needs the engine stopped: `wrapper update`).
async fn check_updates(gui: AppState) {
    use crate::update;
    let repo = std::env::var("WRAPPER_UPDATE_REPO").unwrap_or_else(|_| update::DEFAULT_REPO.into());
    loop {
        let cfg = gui.config();
        let src = update::Source {
            mirror: cfg.mirror,
            proxy: Some(cfg.proxy.clone()).filter(|p| !p.is_empty()),
            ..update::Source::new(&repo)
        };
        match src.manifest(None).await {
            Ok((m, _)) if update::is_newer(&m.version, env!("CARGO_PKG_VERSION")) => {
                let known = gui.update.lock().unwrap().as_ref().and_then(|u| u["version"].as_str().map(str::to_string));
                if known.as_deref() != Some(m.version.as_str()) {
                    gui.engine.console_line(&format!(
                        "[gui] wrapper {} is available; stop the engine and run `wrapper update`",
                        m.version
                    ));
                    *gui.update.lock().unwrap() = Some(json!({ "version": m.version, "tag": m.tag, "notes": m.notes }));
                    gui.engine.emit(HostEvent::Update { version: m.version });
                }
            }
            Ok(_) => {}
            Err(e) => tracing::debug!("update check failed: {e:#}"),
        }
        tokio::time::sleep(Duration::from_secs(24 * 3600)).await;
    }
}

/// Default data directory: next to the executable when writable.
pub fn default_data_dir() -> PathBuf {
    crate::exe_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(header::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap());
        }
        h
    }

    #[test]
    fn only_same_machine_panel_requests_are_trusted() {
        let lo: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let lan: SocketAddr = "192.168.1.9:5000".parse().unwrap();
        let host = ("host", "127.0.0.1:12340");
        assert!(trusted_local(&headers(&[host]), lo));
        assert!(trusted_local(&headers(&[("host", "localhost:12340"), ("origin", "http://localhost:12340")]), lo));
        assert!(trusted_local(&headers(&[("host", "[::1]:12340")]), "[::1]:1".parse().unwrap()));
        assert!(!trusted_local(&headers(&[host]), lan), "other machines need the token");
        assert!(!trusted_local(&headers(&[host, ("x-forwarded-for", "1.2.3.4")]), lo), "local reverse proxy");
        assert!(!trusted_local(&headers(&[("host", "evil.example:12340")]), lo), "DNS rebinding");
        assert!(!trusted_local(&headers(&[host, ("origin", "http://127.0.0.1:8080")]), lo), "another local site");
        assert!(!trusted_local(&headers(&[host, ("sec-fetch-site", "cross-site")]), lo));
        assert!(!trusted_local(&headers(&[]), lo));
    }

    #[test]
    fn tokens_come_from_cookie_bearer_or_query() {
        let h = headers(&[("cookie", "x=1; wrapper_admin=abc"), ("authorization", "Bearer def")]);
        assert_eq!(presented_tokens(&h, Some("a=1&token=ghi")), ["abc", "def", "ghi"]);
    }

    #[test]
    fn engine_settings_ignore_the_listen_address() {
        let a = GuiConfig::default();
        let b = GuiConfig { api_host: "0.0.0.0".into(), api_port: 9, autostart: true, ..a.clone() };
        assert_eq!(engine_settings(&a), engine_settings(&b));
        let c = GuiConfig { memory_mb: 4096, ..a.clone() };
        assert_ne!(engine_settings(&a), engine_settings(&c));
    }

    #[test]
    fn local_urls() {
        assert_eq!(local_url("0.0.0.0:12340".parse().unwrap()), "http://127.0.0.1:12340");
        assert_eq!(local_url("[::]:1".parse().unwrap()), "http://[::1]:1");
        assert_eq!(local_url("10.0.0.2:5".parse().unwrap()), "http://10.0.0.2:5");
    }

    fn test_gui(name: &str, access: Option<&str>) -> Arc<Gui> {
        let dir = std::env::temp_dir().join(format!("wrapper-gui-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Gui::new(GuiOptions {
            listen: None,
            paths: Paths { data_dir: dir.clone(), assets_dir: dir.join("qemu"), rootfs: dir.join("rootfs") },
            open_browser: false,
            access_token: access.map(str::to_string),
        })
    }

    async fn get(gui: &Arc<Gui>, uri: &str, peer: &str, extra: &[(&str, &str)]) -> (StatusCode, HeaderMap, Value) {
        use tower::ServiceExt;
        let mut req = Request::get(uri).header("host", "127.0.0.1:12340");
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let mut req = req.body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo::<SocketAddr>(peer.parse().unwrap()));
        let res = gui.router().oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let bytes = http_body_util::BodyExt::collect(res.into_body()).await.unwrap().to_bytes();
        (status, headers, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn event_stream_is_not_buffered_by_proxies() {
        use tower::ServiceExt;
        let g = test_gui("events", None);
        let mut req = Request::get("/api/host/events").header("host", "127.0.0.1:12340").body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo::<SocketAddr>("127.0.0.1:5000".parse().unwrap()));
        // Headers only: the stream never ends.
        let res = g.router().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "text/event-stream");
        assert_eq!(res.headers()["x-accel-buffering"], "no");
    }

    #[tokio::test]
    async fn front_door_routes_and_guards() {
        let g = test_gui("routes", None);
        let token = g.config().api_token;
        let lo = "127.0.0.1:5000";
        let lan = "192.168.1.9:5000";

        let (s, _, _) = get(&g, "/api/host/status", lo, &[]).await;
        assert_eq!(s, StatusCode::OK, "this machine is trusted");
        let (s, _, _) = get(&g, "/api/host/status", lan, &[]).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        let (s, _, _) = get(&g, "/api/accounts", lan, &[]).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "the agent's management API is guarded too");
        let auth = format!("Bearer {token}");
        let (s, _, _) = get(&g, "/api/host/status", lan, &[("authorization", &auth)]).await;
        assert_eq!(s, StatusCode::OK);

        let (_, _, body) = get(&g, "/api/whoami", lan, &[]).await;
        assert_eq!(body["data"]["admin"], false);
        assert_eq!(body["data"]["mode"], "gui");
        // Public and resource paths reach the (stopped) engine without a token.
        let (s, _, _) = get(&g, "/api/public/overview", lan, &[]).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        let (s, _, _) = get(&g, "/m3u8?adamId=1", lan, &[]).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        let (s, _, _) = get(&g, "/api/accounts", lo, &[]).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);

        // The token link sets the cookie.
        let (s, h, _) = get(&g, &format!("/?token={token}"), lan, &[]).await;
        assert_eq!(s, StatusCode::SEE_OTHER);
        assert!(h["set-cookie"].to_str().unwrap().starts_with(&format!("wrapper_admin={token};")));
        let cookie = format!("wrapper_admin={token}");
        let (s, _, _) = get(&g, "/api/host/config", lan, &[("cookie", &cookie)]).await;
        assert_eq!(s, StatusCode::OK);
    }

    #[tokio::test]
    async fn an_access_token_disables_local_trust() {
        let g = test_gui("access", Some("android"));
        let (s, _, _) = get(&g, "/api/host/status", "127.0.0.1:5000", &[]).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        let (s, _, _) = get(&g, "/api/host/status", "127.0.0.1:5000", &[("cookie", "wrapper_admin=android")]).await;
        assert_eq!(s, StatusCode::OK);
    }
}
