# Architecture

wrapper-lite was restructured so that a single Rust binary, `wrapper`,
replaces the old C launchers (`wrapper-lite.c`, `wrapper-lite-rootless.c`),
the C++ QEMU launcher (`wrapper-lite-qemu.cpp`) and the Go GUI, and so that one
installation can serve **multiple Apple Music accounts** (the feature set of
[wrapper-manager](https://github.com/WorldObservationLog/wrapper-manager)).

The Android-side `lite` binary (`lite/*.cpp`) is unchanged in role: it links
Apple's Android libraries and therefore stays C++. Its HTTP API (`/m3u8`,
`/key`, `/lyrics`, `/webplayback`, `/license`, `/status`) is the contract the
Rust side builds on.

```
host (Windows / macOS / Linux / Android)        QEMU guest, or Linux natively
┌──────────────────────────────┐               ┌───────────────────────────────────┐
│ wrapper gui        (phase 3) │   hostfwd     │ wrapper agent                     │
│  web panel, QEMU launcher,   │──────────────▶│  aggregated lite API + /api       │
│  package download            │  one port     │  per-account lite processes       │
└──────────────────────────────┘               │  ├─ lite --base-dir accounts/<id1>│
                                               │  └─ lite --base-dir accounts/<id2>│
                                               └───────────────────────────────────┘
```

## Delivery phases

1. **Agent core + native Linux** — `crates/agent`, `wrapper agent`,
   `wrapper sandbox-exec`.
2. **QEMU launcher in Rust** — `crates/host`, `wrapper qemu`; the guest init
   starts `wrapper agent`.
3. **Web GUI** — `wrapper gui` (default command) serves the Vite + Svelte
   panel from `web/`, runs the engine and proxies the agent API.
4. **CI, Android, cleanup** — new workflow (Rust checks, web, lite, guest
   with a native-sandbox smoke test, desktop/Android packages, APK), thin
   Android shell, Docker on the native agent, old launchers and GUI removed.

## Crates

| Crate | Purpose |
|---|---|
| `crates/agent` (`wrapper-agent`) | Library: account registry, lite supervision, login/2FA, routing, HTTP server, Linux sandbox. |
| `crates/host` (`wrapper-host`) | Library: QEMU launcher (asset lookup, acceleration fallback, graceful shutdown) and the GUI server. |
| `web/` | The panel (Svelte 5, Vite). `crates/host/build.rs` embeds `web/dist` into the binary. |
| `crates/wrapper` | The `wrapper` binary: `agent`, `qemu`, `login`, `accounts`, internal `sandbox-exec`. |

The guest contract (fw_cfg arguments, data disk, shutdown) is described in
[qemu/README.md](../qemu/README.md).

## The agent

`wrapper agent` runs where lite can run:

| Runtime | Where | How lite is started |
|---|---|---|
| `direct` | QEMU guest (root fs = wrapper rootfs) | exec `/system/bin/lite` |
| `sandbox` | Linux host, no root | `wrapper sandbox-exec`: user + mount + PID namespace, chroot into `rootfs/` |
| `sandbox-privileged` | Linux host as root | same, without a user namespace |

`--runtime auto` picks `direct` when `/system/bin/lite` exists, else `sandbox`.
When the rootless sandbox cannot write to `rootfs/` (it is owned by another
uid, which only the host uid is mapped to) and the agent runs as root, it
falls back to `sandbox-privileged` with a warning; as non-root it refuses to
start and prints the `chown` command to run.

### State layout

```
<state>/                    guest: /data   native: <rootfs>/data
├── accounts.json           [{id, username, region, enabled, deviceInfo?, createdAt, ownerKeyHash?}]
├── agent.json              runtime settings: publicMode, maxAccounts, publicNotice, publicSalt
├── stats.json              request counters (per account and endpoint; per day, 30 days)
├── data/                   Android app data shared by all instances (ANDROID_DATA)
└── accounts/<id>/          lite --base-dir of one account
    ├── mpl_db/
    ├── MUSIC_TOKEN, DEV_TOKEN, STOREFRONT_ID, token_cache.json, ANDROID_ID
    ├── .login-args         credentials for a running login (0600, removed after)
    └── 2fa.txt             only while a login waits for its code
```

`<id>` is the UUIDv5 (nil namespace) of the lower-cased, trimmed username.
A pre-multi-account install (tokens directly in `<state>/`) is moved into
`accounts/<id>/` as `legacy-account` on first start.

### Instance lifecycle

```
Stopped ──start──▶ Starting ──/status has a region──▶ Running
   ▲                  │  exit / timeout                 │ exit / 3 failed health checks
   │                  ▼                                 ▼
   └──stop──────── Backoff (2s … 120s, doubling) ◀──────┘

Running ──"No Active Subscription" / session ended──▶ NeedsLogin
```

* One lite process per account, on a free `127.0.0.1` port.
* At most `--startup-parallelism` (2) instances start at once; lite under TCG is
  CPU heavy while starting.
* Liveness: `/status` every 30 s; region changes are persisted. This only
  proves lite's HTTP server is up.
* FairPlay failures (log lines containing `Fairplay error` or
  `KDCanProcessCKC`, as in wrapper-manager) open a circuit breaker after 5
  hits: the account is skipped for 5 minutes but keeps running. Any
  successful `/key` (or probe), and every fresh lite process, closes it again
  and resets the count.

### Key-path health (`crates/agent/src/health.rs`)

Those errors are not recoverable inside lite; restarting is. Log lines are
only a *suspicion*, a `/key?adamId=215740514&uri=skd://itunes.apple.com/p1429258278/c6` request is the
*judge*, and a separate reachability check of Apple's key server (through
lite's proxy) separates "the network is down" from "lite is broken":

```
FairPlay log line ─┐
every 10 min,      ├─▶ probe /key (adamId=215740514)
unless a real      │      ok ──────────────▶ healthy, circuit closed
/key succeeded ────┘      failed
                           ├─ Apple key server unreachable ─▶ network problem, no restart
                           └─ reachable, failed twice (15 s apart) ─▶ restart lite
                                 └─ already restarted 3× in the last hour ─▶ circuit breaker instead
```

* A health restart goes through the normal backoff (`Backoff`, 2 s doubling).
* `--health-probe-interval <s>` (default 600, `0` = only on log suspicion and
  `/health?probe=1`).
* `GET /health` reports the result per account (`verdict`: `ok`, `busy`,
  `liteDown`, `networkDown`, `broken`); the per-account list needs the token
  when one is set (it names the Apple accounts). The same `health` object is
  part of every account in `/api/accounts`.
* A freshly started lite begins with a clean record (no verdict, no counted
  failures); only the restart budget carries over.
* Probes take a normal request slot, so they never exceed
  `--instance-concurrency`.

### Login

1. `POST /api/accounts/login {username, password, deviceInfo?}` starts
   `lite --login` with credentials passed through `LITE_ARGS_FILE` (not argv).
   The call returns when the login settles, or as soon as lite waits for a
   code (`state: "need2fa"`).
2. `POST /api/logins/<id>/code {code}` writes `2fa.txt` and waits again.
3. On success the account is registered and its service instance started.

Logging in to an existing account stops its instance first (they share the
base dir) and restarts it if the login fails. lite waits `--code-timeout`
(default 300 s) for the code.

### Routing

For `/m3u8`, `/key`, `/webplayback`, `/license`:

1. Candidates = `Running` accounts whose circuit is closed.
2. If they span several storefronts, keep those whose catalog has the item
   (`amp-api` probe per region, songs then music videos, cached 24 h). If every
   probe errors (no Apple access), keep all.
3. Least in-flight first; at most 2 concurrent requests per instance.
4. Retry on another candidate (max 3 attempts, 45 s budget) unless the lite
   code is 400, or 404 for `/lyrics`.

`/lyrics` uses the same candidates, but keeps only the storefronts whose
catalog entry says `hasLyrics` (`data[0].attributes.hasLyrics` of the same
probe, so no extra request). If none does, every storefront that has the song
stays. `language` and `script` are forwarded to lite untouched and play no part
in routing.

### Statistics

Every routed request is counted by endpoint (`m3u8`, `key`, `lyrics`,
`webplayback`, `license`) as ok (`code 0`) or failed, for the account that
answered (requests no account could serve count only in the totals), plus
per-day totals for 30 days. Validation errors are not counted, nor are health
probes. The counters live in memory and are written to `stats.json` every
minute and at shutdown.

### Backup and restore (`crates/agent/src/backup.rs`)

```
wrapper-backup/
├── manifest.json        {format: 1, version, createdAt, accounts: [{id, username, region}]}
├── accounts.json, agent.json, stats.json
└── accounts/<id>/       the lite base dir, without 2fa.txt / .login-args / *.tmp
```

* `GET /api/backup` streams the `.tar.gz` (written on a blocking thread into
  a channel; nothing is buffered whole). Files are read whole before they are
  added, so lite rewriting one meanwhile cannot corrupt the archive.
* `POST /api/restore?mode=merge|replace&settings=1` (body: the archive) or
  `POST /api/restore/pull {url, token, mode, settings}` (the agent downloads
  `<url>/api/backup` itself) unpack into `<state>/.restore-<random>/`. Only
  the files above are extracted; absolute paths, `..`, links and devices are
  refused, and a newer `format` is rejected. Then, per account: stop it, swap
  its dir for the restored one, update the registry, start it. `replace`
  first removes accounts that are not in the backup. A restore refuses to run
  while logins are in progress.
* The shared Android data dir (`<state>/data`) holds no account state and is
  not part of a backup. Paths are the same in the guest and the native
  sandbox (`/data/accounts/<id>`), so backups move freely between them.

## Public mode

`agent.json` → `publicMode` (needs a management token: without one every
visitor would be an administrator). The public API is open:

| Method | Path | Body | Result |
|---|---|---|---|
| GET | `/api/public/overview` | | `{publicMode, notice, maxAccounts, activeAccounts, accepting, regions, accounts: [{ref, name, region, state, verdict, usage}], stats}` (only `{publicMode: false}` when off) |
| POST | `/api/public/login` | `{username, password}` | `{ticket, ref, name, ownerKey, login: {state, message}}` |
| GET / DELETE | `/api/public/login/{ticket}` | | login state / cancel |
| POST | `/api/public/login/{ticket}/code` | `{code}` | login state (5 tries per ticket) |
| POST | `/api/public/logout` | `{username, key}` | removes the account if the key matches |

* `name` is masked (`jo***@g***.com`); `ref` is a salted hash of the account
  id (the id itself is derived from the Apple ID and could be checked against
  a guess).
* A login is refused when the account already exists and is not `needsLogin`,
  when `MAX_PENDING_LOGINS` (2) logins run, when `maxAccounts` active
  accounts (enabled and not `needsLogin`) plus pending logins of new accounts
  are reached, or after 6 logins per client address in 30 minutes. The client
  address is the TCP peer, or behind a proxy on the same machine (the GUI)
  the last non-loopback `X-Forwarded-For` entry.
* `ownerKey` is random; only its SHA-256 is stored (`ownerKeyHash`), on the
  account once the login succeeds. A public re-login issues a new key; an
  admin login keeps the owner's.
* The 2FA step is bound to the random `ticket`, not to the account id, so
  nobody can feed codes into someone else's login.

## HTTP API

Responses use the lite envelope `{"code", "msg", "data"}`. Agent-generated
failures use `code: -1`.

### Resource API (open)

Same parameters as lite; see the README. `GET /status` returns
`{status, regions, clientCount, ready, degraded, accounts}` (wrapper-manager
shape plus extras).

### Management API (`--api-token` protects these when set)

Send `Authorization: Bearer <token>`; `?token=` also works (for SSE), as
does the `wrapper_admin` cookie set by `POST /api/session {token}` (the
panel's sign-in; `DELETE` signs out). `GET /api/whoami` tells the panel who
it is talking to.

| Method | Path | Body / query | Result |
|---|---|---|---|
| GET | `/api/overview` | | version, ready, runtime, accounts, logins |
| GET | `/api/accounts` | | `AccountView[]` |
| POST | `/api/accounts/login` | `{username, password, deviceInfo?}` | `LoginView` |
| POST | `/api/logins/{id}/code` | `{code}` | `LoginView` |
| GET / DELETE | `/api/logins/{id}` | | view / cancel |
| GET | `/api/logins` | | `LoginView[]` |
| POST | `/api/accounts/{id}/start` · `/stop` · `/restart` | | `AccountView` |
| DELETE | `/api/accounts/{id}` | | remove account and its data |
| POST | `/api/health/probe` | | probe every running account now (like `/health?probe=1`), returns the `/health` report |
| GET | `/api/logs` | `source`, `after`, `limit` | log entries |
| POST | `/api/system/shutdown` | | stop all instances and exit (the guest then powers off) |
| GET / PUT | `/api/settings` | `{publicMode?, maxAccounts?, publicNotice?}` | public-mode settings (saved to `agent.json`) |
| GET | `/api/stats` | | `{summary: {since, total, today, daily}, accounts: {<id>: {total, today, lastUsed}}}` |
| GET | `/api/backup` | | `.tar.gz` download |
| POST | `/api/restore` | archive; `?mode=merge\|replace&settings=1` | restore report |
| POST | `/api/restore/pull` | `{url, token, mode, settings}` | restore report |
| GET | `/api/events` | | SSE: `log`, `account`, `accountRemoved`, `login` |

## The GUI (`wrapper gui`)

One port (Settings → Network, default `127.0.0.1:12340`) serves everything;
the agent listens on a private loopback port that only the GUI talks to (the
QEMU port forward, or the native agent's `--port`).

```
client / browser ──▶ wrapper gui (apiHost:apiPort)
                      ├─ /m3u8 /key /lyrics /webplayback /license /status /health /login /logout
                      │                    ─▶ agent, unchanged (the client's own Authorization)
                      ├─ /api/public/*     ─▶ agent public mode (open)
                      ├─ /api/whoami, /api/session   GUI sign-in
                      ├─ /api/host/*       settings, engine, console, package download   (admin)
                      ├─ /api/*            ─▶ agent management API, with its token          (admin)
                      └─ /*                embedded single-page app
```

* **Admin** = a request from this machine (loopback peer, a loopback `Host`,
  no `X-Forwarded-For` / `Forwarded` / `X-Real-IP` from a local reverse proxy,
  no cross-site `Origin` / `Sec-Fetch-Site`), or one carrying the management
  token (`wrapper_admin` cookie, Bearer, `?token=`). With `--access-token`
  (Android) every request needs a token.
* Responses of the admin API lose the agent's CORS headers, so other sites
  cannot read them; the resource and public API keep them.
* Request and response bodies are streamed (backups).
* Changing the listen address rebinds at once (no engine restart); moving
  `127.0.0.1:p` to `0.0.0.0:p` releases the old socket first.
* The panel decides what to show from `/api/whoami`: the admin panel, the
  public page (public mode on), or the sign-in form. A headless agent serves
  the same panel on its API port (`mode: "agent"`: no engine, settings or
  console).

* Engines: **QEMU** (any OS; `crates/host/src/qemu.rs`) or **native** (Linux
  x86_64; spawns `wrapper agent --runtime sandbox`).
* Settings live in `<data dir>/wrapper-gui.json` (default: next to the
  binary). A random management token is generated on first run and passed to
  the agent, so `/api` is never open on the public port.
* When listening on a non-loopback address, wrapper prints the panel URL with
  `?token=<management token>`; opening it sets the cookie.
* Children die with the GUI: `PR_SET_PDEATHSIG` on Linux, a kill-on-close job
  object on Windows; closing the console window triggers a graceful stop.

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/host/info` | platform, component status, package URL |
| GET / PUT | `/api/host/config` | settings (PUT merges fields; an empty `apiToken` regenerates it; a new `apiHost` / `apiPort` applies at once) |
| GET | `/api/host/status` | engine state |
| POST | `/api/host/start` · `/stop` · `/restart` | control the engine |
| GET | `/api/host/console` | QEMU / agent console lines |
| GET | `/api/host/events` | SSE: `console`, `status`, `download` |
| POST | `/api/host/package/download` | download and install the platform package |

## Android

The APK is a thin Kotlin shell around the same binary, built for
`aarch64-linux-android` and shipped as `jniLibs/arm64-v8a/libwrapper.so` (so
it is extracted to the executable native library dir):

* `WrapperService` (foreground service, wake + Wi-Fi locks) unpacks the
  bundled Termux QEMU and guest files to `filesDir/qemu` once per APK version,
  then runs `libwrapper.so gui --data-dir filesDir --assets-dir filesDir/qemu
  --access-token <random>` and restarts it if it dies. The panel and API use
  the port from the settings (default 12340).
* `MainActivity` is a WebView on the panel URL that wrapper prints.
* `targetSdk` stays 28 so QEMU can be executed from `filesDir`.

## Packages

| Artifact | Contents |
|---|---|
| `wrapper-<os>-<arch>` | `wrapper(.exe)`, `qemu/` (kernel, initramfs, bundled QEMU in `qemu/bin`); linux-x86_64 also `rootfs/` for the native engine |
| `wrapper-android-aarch64` | Android `wrapper` + Termux QEMU, for Termux |
| `wrapper-android-apk` | the APK |

The GUI's "download package" fetches `wrapper-<os>-<arch>.zip` and installs
it next to `qemu/`, skipping the running executable, `qemu/data.img` and
`rootfs/data/`.

### wrapper-manager compatibility

`POST /login {username, password[, code]}` → `code 0` (done), `code 2`
(`"2fa code require"`, `data.loginId`), `code -1` (failure, or
`"already login"`). `POST /logout {username}` or `{id}` (the account UUID;
`id` wins when both are given) removes the account.
Disable both with `--no-legacy-management`.
