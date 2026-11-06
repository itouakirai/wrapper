//! HTTP-level tests against fake lite upstreams (no real lite needed).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::accounts::{account_id, AccountRecord, Layout};
use crate::agent::{Agent, AgentConfig};
use crate::api::InstanceState;
use crate::events::EventBus;
use crate::health::Verdict;
use crate::lite::LiteRuntime;

/// Starts a fake lite that answers `/m3u8` with `code` and counts hits.
async fn fake_lite(code: i64, hits: Arc<AtomicUsize>) -> u16 {
    let app = Router::new()
        .route(
            "/m3u8",
            get(move |q: axum::extract::RawQuery| {
                let hits = hits.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    axum::Json(json!({"code": code, "msg": "x", "data": {"query": q.0}}))
                }
            }),
        )
        .route("/status", get(|| async { axum::Json(json!({"code":0,"msg":"SUCCESS","data":{"regions":["jp"]}})) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

fn test_agent(name: &str, token: Option<&str>) -> Arc<Agent> {
    test_agent_with(name, token, |_| {})
}

fn test_agent_with(name: &str, token: Option<&str>, tweak: impl FnOnce(&mut AgentConfig)) -> Arc<Agent> {
    let dir = std::env::temp_dir().join(format!("wrapper-agent-http-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let layout = Layout { state_dir: dir, lite_root: "/data".into() };
    let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let mut cfg = AgentConfig::new(listen, layout, LiteRuntime::Direct { lite_bin: "/nonexistent".into() });
    cfg.api_token = token.map(str::to_string);
    tweak(&mut cfg);
    Agent::new(cfg, EventBus::new(100)).unwrap()
}

/// A fake lite whose `/key` either delivers a key or fails like lite does
/// when the FairPlay context is broken.
async fn fake_key_lite(works: bool, hits: Arc<AtomicUsize>) -> u16 {
    let app = Router::new().route(
        "/key",
        get(move || {
            let hits = hits.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                if works {
                    axum::Json(json!({"code":0,"msg":"SUCCESS","data":{"contentKey":"k"}}))
                } else {
                    axum::Json(json!({"code":500,"msg":"key retrieval failed","data":null}))
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

/// Points the agent's "can we reach Apple" check at a local server (up) or a
/// closed port (down).
async fn set_apple_reachable(agent: &Agent, up: bool) {
    let port = if up {
        fake_lite(0, Arc::new(AtomicUsize::new(0))).await
    } else {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    agent.apple.set_reach_url(&format!("http://127.0.0.1:{port}/"));
}

fn add_running(agent: &Agent, user: &str, region: &str, port: u16) -> String {
    let rec = AccountRecord {
        id: account_id(user),
        username: user.into(),
        region: region.into(),
        enabled: true,
        ..Default::default()
    };
    let (inst, _) = agent.insert_instance(rec);
    inst.set_running_for_test(port);
    inst.id.clone()
}

async fn call(agent: &Arc<Agent>, req: Request<Body>) -> (StatusCode, Value) {
    let res = crate::server::router(agent.clone()).oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn get_req(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn forwards_and_passes_envelope_through() {
    let agent = test_agent("forward", None);
    let hits = Arc::new(AtomicUsize::new(0));
    let port = fake_lite(0, hits.clone()).await;
    add_running(&agent, "a@x.y", "jp", port);

    let (status, body) = call(&agent, get_req("/m3u8?adamId=123&extra=%20a")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["code"], 0);
    // The raw query string is forwarded untouched.
    assert_eq!(body["data"]["query"], "adamId=123&extra=%20a");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retries_on_instance_failure_but_not_on_bad_request() {
    let agent = test_agent("retry", None);
    let bad_hits = Arc::new(AtomicUsize::new(0));
    let good_hits = Arc::new(AtomicUsize::new(0));
    let bad = fake_lite(500, bad_hits.clone()).await;
    let good = fake_lite(0, good_hits.clone()).await;
    let bad_id = add_running(&agent, "bad@x.y", "jp", bad);
    let good_id = add_running(&agent, "good@x.y", "jp", good);

    for _ in 0..6 {
        let (_, body) = call(&agent, get_req("/m3u8?adamId=1")).await;
        assert_eq!(body["code"], 0, "a 500 from one instance must fall through to the other");
    }
    assert_eq!(good_hits.load(Ordering::SeqCst), 6);
    // The request counts once, as served; each attempt counts on its account.
    let s = agent.stats.summary();
    assert_eq!(s.total["m3u8"].ok, 6);
    assert_eq!(s.total["m3u8"].fail, 0);
    assert_eq!(agent.stats.account(&good_id).total["m3u8"].ok, 6);
    let bad = agent.stats.account(&bad_id).total.get("m3u8").copied().unwrap_or_default();
    assert_eq!(bad.fail as usize, bad_hits.load(Ordering::SeqCst));

    let agent = test_agent("noretry", None);
    let h1 = Arc::new(AtomicUsize::new(0));
    let h2 = Arc::new(AtomicUsize::new(0));
    add_running(&agent, "a@x.y", "jp", fake_lite(400, h1.clone()).await);
    add_running(&agent, "b@x.y", "jp", fake_lite(400, h2.clone()).await);
    let (_, body) = call(&agent, get_req("/m3u8?adamId=1")).await;
    assert_eq!(body["code"], 400);
    assert_eq!(h1.load(Ordering::SeqCst) + h2.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn validates_requests_without_touching_instances() {
    let agent = test_agent("validate", None);
    let (_, body) = call(&agent, get_req("/m3u8")).await;
    assert_eq!(body["msg"], "missing adamId");
    let (_, body) = call(&agent, get_req("/key?adamId=1")).await;
    assert_eq!(body["msg"], "missing uri");
    let (_, body) = call(&agent, get_req("/key?adamId=1&uri=skd://itunes.apple.com/P000000000/s1/e1")).await;
    assert_eq!(body["msg"], "invalid uri for adamId");
    let (_, body) = call(&agent, get_req("/m3u8?adamId=1")).await;
    assert_eq!(body["msg"], "no available instance");
    // wrapper-manager's key_request_test.go: the rest of the /key contract.
    let (_, body) = call(&agent, get_req("/key")).await;
    assert_eq!(body["msg"], "missing adamId");
    let (_, body) = call(&agent, get_req("/key?uri=skd%3A%2F%2Fx")).await;
    assert_eq!(body["msg"], "missing adamId");
    // The prefetch URI is fine with adamId=0 (validation passes, nobody serves), as is any other uri.
    let prefetch = "/key?adamId=0&uri=skd%3A%2F%2Fitunes.apple.com%2FP000000000%2Fs1%2Fe1";
    let (_, body) = call(&agent, get_req(prefetch)).await;
    assert_eq!(body["msg"], "no available instance");
    let (_, body) = call(&agent, get_req("/key?adamId=0&uri=skd%3A%2F%2Fother")).await;
    assert_eq!(body["msg"], "no available instance");
    let req = Request::post("/license").body(Body::from("{}")).unwrap();
    let (_, body) = call(&agent, req).await;
    assert_eq!(body["msg"], "missing adamId, challenge, or uri");
}

#[tokio::test]
async fn aggregates_status() {
    let agent = test_agent("status", None);
    let hits = Arc::new(AtomicUsize::new(0));
    add_running(&agent, "a@x.y", "jp", fake_lite(0, hits.clone()).await);
    add_running(&agent, "b@x.y", "us", fake_lite(0, hits.clone()).await);
    add_running(&agent, "c@x.y", "jp", fake_lite(0, hits).await);
    let (_, body) = call(&agent, get_req("/status")).await;
    assert_eq!(body["data"]["clientCount"], 3);
    assert_eq!(body["data"]["status"], true);
    let mut regions: Vec<String> = serde_json::from_value(body["data"]["regions"].clone()).unwrap();
    regions.sort();
    assert_eq!(regions, ["jp", "us"]);
}

#[tokio::test]
async fn management_requires_token_when_configured() {
    let agent = test_agent("auth", Some("s3cret"));
    let (status, _) = call(&agent, get_req("/api/accounts")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let req = Request::get("/api/accounts").header("Authorization", "Bearer s3cret").body(Body::empty()).unwrap();
    let (status, body) = call(&agent, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["code"], 0);
    let (status, _) = call(&agent, get_req("/api/logs?token=s3cret")).await;
    assert_eq!(status, StatusCode::OK);
    let req = Request::post("/logout").body(Body::from(r#"{"username":"x"}"#)).unwrap();
    let (status, _) = call(&agent, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // The resource API stays open.
    let (status, _) = call(&agent, get_req("/status")).await;
    assert_eq!(status, StatusCode::OK);
}

#[test]
fn logout_resolves_username_or_uuid() {
    use crate::api::LogoutRequest;
    let by_name = |u: &str| LogoutRequest { username: u.into(), ..Default::default() };
    let by_id = |i: &str| LogoutRequest { id: i.into(), ..Default::default() };

    assert_eq!(by_name("acct@example.com").resolve_id().unwrap(), account_id("acct@example.com"));
    // A raw UUID is used as-is (normalized to lower case).
    let raw = uuid::Uuid::new_v4().to_string();
    assert_eq!(by_id(&raw).resolve_id().unwrap(), raw);
    assert_eq!(by_id(&raw.to_uppercase()).resolve_id().unwrap(), raw);
    // id wins when both are given.
    let both = LogoutRequest { username: "acct@example.com".into(), id: raw.clone() };
    assert_eq!(both.resolve_id().unwrap(), raw);
    assert!(by_id("not-a-uuid").resolve_id().is_err());
    assert!(LogoutRequest::default().resolve_id().is_err());
}

#[tokio::test]
async fn logout_by_id_removes_the_account() {
    let agent = test_agent("logout-id", None);
    let id = add_running(&agent, "a@x.y", "jp", 1);
    let post = |body: String| {
        Request::post("/logout").header("Content-Type", "application/json").body(Body::from(body)).unwrap()
    };
    let (_, body) = call(&agent, post(r#"{"id":"not-a-uuid"}"#.into())).await;
    assert_eq!(body["msg"], "invalid id (expected a UUID)");
    let (_, body) = call(&agent, post("{}".into())).await;
    assert_eq!(body["msg"], "missing username or id");
    let (_, body) = call(&agent, post(format!(r#"{{"id":"{id}"}}"#))).await;
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["id"], id);
    assert!(agent.instance(&id).is_none());
    let (_, body) = call(&agent, post(format!(r#"{{"id":"{id}"}}"#))).await;
    assert_eq!(body["msg"], "no such account");
}

#[tokio::test]
async fn legacy_endpoints_validate_input() {
    let agent = test_agent("legacy", None);
    let req = Request::post("/logout")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"username":"ghost@x.y"}"#))
        .unwrap();
    let (_, body) = call(&agent, req).await;
    assert_eq!(body["msg"], "no such account");

    let req = Request::post("/login")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"username":"a@x.y","password":"p","code":"123456"}"#))
        .unwrap();
    let (_, body) = call(&agent, req).await;
    assert_eq!(body["code"], -1);
    assert!(body["msg"].as_str().unwrap().contains("no pending login"));

    let port = fake_lite(0, Arc::new(AtomicUsize::new(0))).await;
    add_running(&agent, "a@x.y", "jp", port);
    let req = Request::post("/login")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"username":"A@x.y","password":"p"}"#))
        .unwrap();
    let (_, body) = call(&agent, req).await;
    assert_eq!(body["msg"], "already login");
}

#[tokio::test]
async fn degraded_instances_are_skipped() {
    let agent = test_agent("degraded", None);
    let h1 = Arc::new(AtomicUsize::new(0));
    let h2 = Arc::new(AtomicUsize::new(0));
    let id1 = add_running(&agent, "a@x.y", "jp", fake_lite(0, h1.clone()).await);
    add_running(&agent, "b@x.y", "jp", fake_lite(0, h2.clone()).await);
    agent.instance(&id1).unwrap().open_circuit_for_test();
    for _ in 0..4 {
        call(&agent, get_req("/m3u8?adamId=1")).await;
    }
    assert_eq!(h1.load(Ordering::SeqCst), 0);
    assert_eq!(h2.load(Ordering::SeqCst), 4);
    assert_eq!(agent.instance(&id1).unwrap().state(), InstanceState::Running);
}

// ------------------------------------------------------------------ health

#[tokio::test]
async fn probe_tells_broken_lite_from_network_trouble() {
    let agent = test_agent("probe", None);
    let hits = Arc::new(AtomicUsize::new(0));
    let good = add_running(&agent, "good@x.y", "jp", fake_key_lite(true, hits.clone()).await);
    let bad = add_running(&agent, "bad@x.y", "jp", fake_key_lite(false, hits.clone()).await);
    let good = agent.instance(&good).unwrap();
    let bad = agent.instance(&bad).unwrap();

    set_apple_reachable(&agent, true).await;
    assert_eq!(agent.probe_instance(&good).await.verdict, Verdict::Ok);
    let r = agent.probe_instance(&bad).await;
    assert_eq!(r.verdict, Verdict::Broken, "Apple reachable and no key: lite is the problem");
    assert!(r.detail.contains("key retrieval failed"), "{}", r.detail);
    assert_eq!(bad.health_view().consecutive_failures, 1);

    set_apple_reachable(&agent, false).await;
    let r = agent.probe_instance(&bad).await;
    assert_eq!(r.verdict, Verdict::NetworkDown, "Apple unreachable: restarting would not help");
    assert_eq!(bad.health_view().consecutive_failures, 1, "network trouble must not count against lite");

    // The probe asks for the prefetch key, never for a real song.
    assert_eq!(hits.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn health_endpoint_reports_and_gates_live_probes() {
    let agent = test_agent("health-ep", Some("s3cret"));
    // Nothing configured: nobody can serve.
    let (status, body) = call(&agent, get_req("/health")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["data"]["healthy"], false);

    let hits = Arc::new(AtomicUsize::new(0));
    let good = add_running(&agent, "good@x.y", "jp", fake_key_lite(true, hits.clone()).await);
    let bad = add_running(&agent, "bad@x.y", "us", fake_key_lite(false, hits.clone()).await);
    set_apple_reachable(&agent, true).await;

    // Cached view: open (like /status), and no Apple traffic.
    let (status, body) = call(&agent, get_req("/health")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["status"], "ok");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    // Without the token it does not name the accounts.
    assert!(body["data"].get("accounts").is_none(), "{body}");
    assert!(!body.to_string().contains("good@x.y"));

    // A live probe needs the token...
    let (status, _) = call(&agent, get_req("/health?probe=1")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    // ...and then finds the broken account.
    let req = Request::get("/health?probe=1").header("Authorization", "Bearer s3cret").body(Body::empty()).unwrap();
    let (status, body) = call(&agent, req).await;
    assert_eq!(status, StatusCode::OK, "one healthy account is enough to serve");
    assert_eq!(body["data"]["status"], "degraded");
    let accounts = body["data"]["accounts"].as_array().unwrap();
    let find = |id: &str| accounts.iter().find(|a| a["id"] == id).unwrap();
    assert_eq!(find(&good)["healthy"], true);
    assert_eq!(find(&good)["health"]["verdict"], "ok");
    assert_eq!(find(&bad)["healthy"], false);
    assert_eq!(find(&bad)["health"]["verdict"], "broken");

    // Account views carry the same data for the panel.
    let (_, body) = call(&agent, get_req("/api/accounts?token=s3cret")).await;
    let view = body["data"].as_array().unwrap().iter().find(|a| a["id"] == bad).unwrap().clone();
    assert_eq!(view["health"]["verdict"], "broken");

    // The panel's way in: a management endpoint behind the token.
    let (status, _) = call(&agent, Request::post("/api/health/probe").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let req = Request::post("/api/health/probe").header("Authorization", "Bearer s3cret").body(Body::empty()).unwrap();
    let (status, body) = call(&agent, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["status"], "degraded");
    assert_eq!(body["data"]["accounts"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn monitor_restarts_only_a_confirmed_break() {
    let fast = |c: &mut AgentConfig| {
        c.probe_interval = Duration::from_millis(60);
        c.probe_confirm_gap = Duration::from_millis(30);
        c.probe_recheck = Duration::from_millis(60);
    };

    // Broken with Apple reachable: confirmed twice, then a restart is asked for.
    let agent = test_agent_with("mon-broken", None, fast);
    let id = add_running(&agent, "a@x.y", "jp", fake_key_lite(false, Arc::new(AtomicUsize::new(0))).await);
    set_apple_reachable(&agent, true).await;
    let inst = agent.instance(&id).unwrap();
    let why = tokio::time::timeout(Duration::from_secs(5), agent.clone().health_monitor(inst.clone()))
        .await
        .expect("a broken instance must be reported");
    assert!(why.contains("key probe failed 2x"), "{why}");

    // Same failure but Apple unreachable: a restart cannot fix it, so none is requested.
    let agent = test_agent_with("mon-net", None, fast);
    let id = add_running(&agent, "a@x.y", "jp", fake_key_lite(false, Arc::new(AtomicUsize::new(0))).await);
    set_apple_reachable(&agent, false).await;
    let inst = agent.instance(&id).unwrap();
    let monitor = tokio::spawn(agent.clone().health_monitor(inst.clone()));
    // (Refused connections can take ~2s to fail on Windows, hence the polling.)
    wait_until(|| inst.health_view().verdict == Some(Verdict::NetworkDown)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!monitor.is_finished(), "no restart while the network is down");
    assert_eq!(inst.health_view().consecutive_failures, 0);
    monitor.abort();

    // A healthy probe closes a circuit that log lines opened.
    let agent = test_agent_with("mon-ok", None, fast);
    let id = add_running(&agent, "a@x.y", "jp", fake_key_lite(true, Arc::new(AtomicUsize::new(0))).await);
    let inst = agent.instance(&id).unwrap();
    inst.open_circuit_for_test();
    assert!(inst.circuit_open());
    let monitor = tokio::spawn(agent.clone().health_monitor(inst.clone()));
    wait_until(|| !inst.circuit_open()).await;
    assert_eq!(inst.health_view().verdict, Some(Verdict::Ok), "the probe proved the key path works");
    assert!(!monitor.is_finished());
    monitor.abort();
}

#[tokio::test]
async fn a_failure_found_by_health_probe_is_confirmed_after_the_gap() {
    let agent = test_agent_with("mon-probe-all", None, |c| {
        c.probe_interval = Duration::ZERO;
        c.probe_confirm_gap = Duration::from_millis(600);
    });
    let hits = Arc::new(AtomicUsize::new(0));
    let id = add_running(&agent, "a@x.y", "jp", fake_key_lite(false, hits.clone()).await);
    set_apple_reachable(&agent, true).await;
    let inst = agent.instance(&id).unwrap();
    let monitor = tokio::spawn(agent.clone().health_monitor(inst.clone()));

    // `/health?probe=1` finds the break and wakes the monitor...
    agent.probe_all().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    // ...which must not count an immediate second probe as the confirmation.
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(!monitor.is_finished());
    let why = tokio::time::timeout(Duration::from_secs(5), monitor).await.expect("confirmed later").unwrap();
    assert!(why.contains("2x"), "{why}");
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_new_process_starts_with_a_clean_health_record() {
    let agent = test_agent("new-process", None);
    let id = add_running(&agent, "a@x.y", "jp", fake_key_lite(false, Arc::new(AtomicUsize::new(0))).await);
    set_apple_reachable(&agent, true).await;
    let inst = agent.instance(&id).unwrap();
    agent.probe_instance(&inst).await;
    agent.probe_instance(&inst).await;
    assert!(inst.take_restart_budget(3, Duration::from_secs(3600)));
    assert!(!inst.is_healthy());

    inst.health_new_process();
    let v = inst.health_view();
    assert_eq!((v.verdict, v.consecutive_failures), (None, 0), "the old process's results are gone");
    assert!(inst.is_healthy(), "a fresh process is not reported broken");
    assert_eq!(v.health_restarts, 1, "the restart budget survives restarts");
}

async fn wait_until(mut cond: impl FnMut() -> bool) {
    for _ in 0..200 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("condition not met within 10s");
}

#[tokio::test]
async fn suspicion_triggers_a_probe_even_when_periodic_probing_is_off() {
    let agent = test_agent_with("mon-suspect", None, |c| {
        c.probe_interval = Duration::ZERO;
        c.probe_confirm_gap = Duration::from_millis(10);
    });
    let hits = Arc::new(AtomicUsize::new(0));
    let id = add_running(&agent, "a@x.y", "jp", fake_key_lite(true, hits.clone()).await);
    let inst = agent.instance(&id).unwrap();
    let monitor = tokio::spawn(agent.clone().health_monitor(inst.clone()));

    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(hits.load(Ordering::SeqCst), 0, "interval 0 disables periodic probes");
    inst.suspect.notify_one();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    monitor.abort();
}

#[tokio::test]
async fn real_key_traffic_counts_as_health_evidence() {
    let agent = test_agent("key-evidence", None);
    let id = add_running(&agent, "a@x.y", "jp", fake_key_lite(true, Arc::new(AtomicUsize::new(0))).await);
    let inst = agent.instance(&id).unwrap();
    assert!(inst.health_view().last_key_ok_at.is_none());
    let (_, body) = call(&agent, get_req("/key?adamId=1&uri=skd://itunes.apple.com/P1/s1/e1")).await;
    assert_eq!(body["code"], 0);
    assert!(inst.health_view().last_key_ok_at.is_some());

    // ...and it clears a circuit that FairPlay log lines had opened.
    inst.open_circuit_for_test();
    assert!(inst.circuit_open());
    inst.note_key_ok();
    assert!(!inst.circuit_open());
}

#[tokio::test]
async fn restart_budget_is_rate_limited() {
    let agent = test_agent("budget", None);
    let id = add_running(&agent, "a@x.y", "jp", 1);
    let inst = agent.instance(&id).unwrap();
    let window = Duration::from_millis(200);
    assert!(inst.take_restart_budget(2, window));
    assert!(inst.take_restart_budget(2, window));
    assert!(!inst.take_restart_budget(2, window), "third restart inside the window is refused");
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(inst.take_restart_budget(2, window), "the window slides");
}

// ------------------------------------------------ stats, public mode, backup

fn json_req(method: &str, uri: &str, token: Option<&str>, body: Value) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(uri).header("Content-Type", "application/json");
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

fn authed_get(uri: &str, token: &str) -> Request<Body> {
    Request::get(uri).header("Authorization", format!("Bearer {token}")).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn served_requests_are_counted_per_account() {
    let agent = test_agent("stats", None);
    let ok = add_running(&agent, "a@x.y", "jp", fake_lite(0, Arc::new(AtomicUsize::new(0))).await);
    call(&agent, get_req("/m3u8?adamId=1")).await;
    call(&agent, get_req("/m3u8?adamId=2")).await;
    // Validation failures are not counted; requests nobody serves are.
    call(&agent, get_req("/key?adamId=1")).await;
    agent.remove_instance(&ok);
    call(&agent, get_req("/m3u8?adamId=3")).await;

    let (_, body) = call(&agent, get_req("/api/stats")).await;
    let data = &body["data"];
    assert_eq!(data["accounts"][&ok]["total"]["m3u8"]["ok"], 2);
    assert_eq!(data["accounts"][&ok]["today"]["m3u8"]["ok"], 2);
    assert_eq!(data["summary"]["total"]["m3u8"], json!({"ok": 2, "fail": 1}));
    assert!(data["summary"]["total"].get("key").is_none());
    assert_eq!(data["summary"]["unserved"]["m3u8"]["fail"], 1);
    assert_eq!(data["accounts"][&ok]["daily"].as_array().unwrap().len(), crate::stats::KEEP_DAYS as usize);
    assert!(data["accounts"][&ok]["lastOk"].is_i64());
    assert_eq!(data["summary"]["daily"].as_array().unwrap().len(), crate::stats::KEEP_DAYS as usize);
}

#[tokio::test]
async fn public_mode_needs_a_token_and_hides_private_data() {
    let open = test_agent("public-open", None);
    let (status, body) = call(&open, json_req("PUT", "/api/settings", None, json!({"publicMode": true}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let agent = test_agent("public", Some("t0k"));
    let (_, body) = call(&agent, get_req("/api/public/overview")).await;
    assert_eq!(body["data"]["publicMode"], false);
    let (status, _) = call(&agent, json_req("POST", "/api/public/login", None, json!({"username":"a@b.c","password":"p"}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&agent, json_req("PUT", "/api/settings", None, json!({"publicMode": true}))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let patch = json!({"publicMode": true, "maxAccounts": 5, "publicNotice": "hi"});
    let (status, body) = call(&agent, json_req("PUT", "/api/settings", Some("t0k"), patch)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["maxAccounts"], 5);
    let id = add_running(&agent, "someone.private@example.com", "jp", fake_lite(0, Arc::new(AtomicUsize::new(0))).await);
    call(&agent, get_req("/m3u8?adamId=1")).await;

    let (_, body) = call(&agent, get_req("/api/public/overview")).await;
    let text = body.to_string();
    assert!(!text.contains("someone.private"), "{text}");
    assert!(!text.contains(&id), "account ids are not public");
    let data = &body["data"];
    assert_eq!(data["notice"], "hi");
    assert_eq!(data["accounts"][0]["name"], "so***@e***.com");
    assert_eq!(data["accounts"][0]["usage"]["total"]["m3u8"]["ok"], 1);
    assert_eq!(data["regions"], json!(["jp"]));
    assert_eq!(data["activeAccounts"], 1);
    assert_eq!(data["accepting"], true);
    // Settings survive a restart of the agent.
    assert!(crate::settings::AgentSettings::load(&agent.layout().settings_path()).public_mode);
}

#[tokio::test]
async fn public_login_is_limited() {
    let agent = test_agent("public-login", Some("t"));
    agent
        .update_settings(crate::settings::SettingsPatch { public_mode: Some(true), max_accounts: Some(1), ..Default::default() })
        .unwrap();
    add_running(&agent, "taken@x.y", "jp", 1);
    let login = |user: &str| json_req("POST", "/api/public/login", None, json!({"username": user, "password": "pw"}));

    let (status, body) = call(&agent, login("Taken@x.y")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["msg"].as_str().unwrap().contains("already"), "{body}");
    let (status, body) = call(&agent, login("new@x.y")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["msg"].as_str().unwrap().contains("full"), "{body}");

    // With room, the login starts (and fails here: there is no lite), and
    // its state is only reachable through the ticket.
    agent.update_settings(crate::settings::SettingsPatch { max_accounts: Some(0), ..Default::default() }).unwrap();
    let (status, body) = call(&agent, login("new@x.y")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let data = &body["data"];
    assert_eq!(data["login"]["state"], "failed");
    assert_eq!(data["name"], "ne***@x***.y");
    assert_eq!(data["ownerKey"].as_str().unwrap().len(), 32);
    let ticket = data["ticket"].as_str().unwrap();
    let (_, body) = call(&agent, get_req(&format!("/api/public/login/{ticket}"))).await;
    assert_eq!(body["data"]["state"], "failed");
    let (status, _) = call(&agent, get_req("/api/public/login/nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(agent.instance(&account_id("new@x.y")).is_none(), "a failed login adds nothing");

    // Each address gets a limited number of attempts.
    let mut last = StatusCode::OK;
    for _ in 0..10 {
        last = call(&agent, login("other@x.y")).await.0;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn public_logout_needs_the_owner_key() {
    let agent = test_agent("public-logout", Some("t"));
    agent.update_settings(crate::settings::SettingsPatch { public_mode: Some(true), ..Default::default() }).unwrap();
    let id = add_running(&agent, "mine@x.y", "jp", 1);
    agent.instance(&id).unwrap().update_record(|r| r.owner_key_hash = Some(crate::settings::sha256_hex("k3y")));
    let admin_added = add_running(&agent, "admin@x.y", "jp", 1);
    let logout = |user: &str, key: &str| json_req("POST", "/api/public/logout", None, json!({"username": user, "key": key}));

    let (status, _) = call(&agent, logout("mine@x.y", "wrong")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&agent, logout("admin@x.y", "")).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "accounts without a key cannot be logged out publicly");
    let (status, body) = call(&agent, logout("MINE@x.y", "k3y")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(agent.instance(&id).is_none());
    assert!(agent.instance(&admin_added).is_some());
    let (_, body) = call(&agent, get_req("/api/accounts?token=t")).await;
    assert_eq!(body["data"][0]["public"], false);
}

#[tokio::test]
async fn session_cookie_signs_the_panel_in() {
    let agent = test_agent("session", Some("s3cret"));
    let (_, body) = call(&agent, get_req("/api/whoami")).await;
    assert_eq!(body["data"], json!({"mode": "agent", "admin": false, "tokenRequired": true, "version": env!("CARGO_PKG_VERSION")}));
    let (status, _) = call(&agent, json_req("POST", "/api/session", None, json!({"token": "nope"}))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let res = crate::server::router(agent.clone())
        .oneshot(json_req("POST", "/api/session", None, json!({"token": "s3cret"})))
        .await
        .unwrap();
    let cookie = res.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(cookie.starts_with("wrapper_admin=s3cret;") && cookie.contains("HttpOnly"), "{cookie}");
    let req = Request::get("/api/accounts").header("Cookie", "a=b; wrapper_admin=s3cret").body(Body::empty()).unwrap();
    let (status, _) = call(&agent, req).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn backup_restores_accounts_on_another_agent() {
    let src = test_agent("backup-src", Some("a"));
    src.update_settings(crate::settings::SettingsPatch { public_mode: Some(true), max_accounts: Some(7), ..Default::default() })
        .unwrap();
    let id = add_running(&src, "a@x.y", "jp", fake_lite(0, Arc::new(AtomicUsize::new(0))).await);
    let dir = src.layout().ensure_account_dir(&id).unwrap();
    std::fs::write(dir.join("MUSIC_TOKEN"), "music").unwrap();
    std::fs::write(dir.join("mpl_db/db"), "db").unwrap();
    call(&src, get_req("/m3u8?adamId=1")).await;
    let src_salt = src.settings().public_salt;

    let (status, _) = call(&src, get_req("/api/backup")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let res = crate::server::router(src.clone()).oneshot(authed_get("/api/backup", "a")).await.unwrap();
    assert_eq!(res.headers()["content-type"], "application/gzip");
    assert!(res.headers()["content-disposition"].to_str().unwrap().contains("wrapper-backup-"));
    let archive = res.into_body().collect().await.unwrap().to_bytes();

    let dst = test_agent("backup-dst", Some("b"));
    let other = add_running(&dst, "other@x.y", "us", 1);
    let restore = |q: &str| {
        Request::post(format!("/api/restore?{q}"))
            .header("Authorization", "Bearer b")
            .body(Body::from(archive.clone()))
            .unwrap()
    };
    let (status, body) = call(&dst, restore("mode=merge")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["added"], 1);
    assert_eq!(body["data"]["settings"], false);
    let restored = dst.layout().account_dir(&id);
    assert_eq!(std::fs::read_to_string(restored.join("MUSIC_TOKEN")).unwrap(), "music");
    assert!(restored.join("mpl_db/db").exists());
    assert!(dst.instance(&other).is_some(), "merge keeps the other accounts");
    assert_eq!(dst.stats.account(&id).total["m3u8"].ok, 1);
    assert!(!dst.settings().public_mode);
    assert!(Layout::load(dst.layout()).iter().any(|r| r.id == id), "the registry is saved");

    // Replace: an exact copy, settings included.
    let (_, body) = call(&dst, restore("mode=replace&settings=1")).await;
    assert_eq!(body["data"]["replaced"], 1);
    assert_eq!(body["data"]["removed"], 1);
    assert!(dst.instance(&other).is_none());
    let s = dst.settings();
    assert!(s.public_mode);
    assert_eq!(s.max_accounts, 7);
    assert_eq!(s.public_salt, src_salt, "public handles stay the same after a move");
    assert!(!dst.layout().state_dir.read_dir().unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".restore")));

    let bad = Request::post("/api/restore").header("Authorization", "Bearer b").body(Body::from("junk")).unwrap();
    let (status, _) = call(&dst, bad).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    src.shutdown().await;
    dst.shutdown().await;
}

#[tokio::test]
async fn pull_migrates_from_a_running_agent() {
    let src = test_agent("pull-src", Some("old"));
    let id = add_running(&src, "a@x.y", "jp", 1);
    std::fs::write(src.layout().ensure_account_dir(&id).unwrap().join("MUSIC_TOKEN"), "m").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = src.clone();
    tokio::spawn(async move { crate::server::serve(server, listener, None, std::future::pending()).await });

    let dst = test_agent("pull-dst", Some("new"));
    let pull = |token: &str| {
        json_req("POST", "/api/restore/pull", Some("new"), json!({"url": format!("http://127.0.0.1:{port}/"), "token": token}))
    };
    let (status, body) = call(&dst, pull("wrong")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["msg"].as_str().unwrap().contains("unauthorized"), "{body}");
    let (status, body) = call(&dst, pull("old")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["accounts"], json!(["a@x.y"]));
    assert!(dst.layout().account_dir(&id).join("MUSIC_TOKEN").exists());
    dst.shutdown().await;
}
