//! `wrapper`: one binary for every role.
//!
//! * `wrapper agent` — multi-account supervisor + aggregated API (Linux: in
//!   the QEMU guest, or natively with a user-namespace sandbox).
//! * `wrapper gui` (default) — web panel that runs the agent in QEMU or natively.
//! * `wrapper qemu` — boot the agent in a QEMU guest (any host OS).
//! * `wrapper login` / `wrapper accounts` — manage accounts of a running agent.
//! * `wrapper backup` / `wrapper restore` — move accounts between machines.
//! * `wrapper update` — upgrade this install from GitHub releases.
//! * `wrapper sandbox-exec` — internal helper used by the agent to start lite
//!   inside the rootfs chroot.

mod client;
mod update;

use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context};
use clap::{Args, Parser, Subcommand, ValueEnum};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};
use wrapper_agent::{Agent, AgentConfig, BusLayer, EventBus, Layout, LiteOptions, LiteRuntime};

#[derive(Parser)]
#[command(name = "wrapper", version, about = "wrapper-lite multi-account manager")]
struct Cli {
    /// Defaults to `gui`.
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Web panel: manage the engine and accounts in the browser (default).
    Gui(GuiArgs),
    /// Run the multi-account agent (Linux only).
    Agent(Box<AgentArgs>),
    /// Boot the agent inside a QEMU guest and forward its port.
    Qemu(Box<QemuArgs>),
    /// Log an account in on a running agent (interactive 2FA).
    Login(client::LoginArgs),
    /// List the accounts of a running agent.
    Accounts(client::ApiArgs),
    /// Download a backup of every account (tokens, settings, statistics).
    Backup(client::BackupArgs),
    /// Restore a backup file, or migrate straight from another wrapper.
    Restore(client::RestoreArgs),
    /// Upgrade this install (binary, QEMU guest, rootfs) from GitHub releases.
    Update(update::UpdateArgs),
    /// Internal: run a program chrooted into the wrapper rootfs.
    #[command(hide = true)]
    SandboxExec(SandboxArgs),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RuntimeKind {
    /// `direct` when /system/bin/lite exists (QEMU guest), else `sandbox`.
    Auto,
    /// Start lite directly; the current root is the wrapper rootfs.
    Direct,
    /// chroot into the rootfs inside a user namespace (no root needed).
    Sandbox,
    /// chroot into the rootfs using real root privileges.
    SandboxPrivileged,
}

#[derive(Args)]
struct AgentArgs {
    /// Address the aggregated API listens on.
    #[arg(long, default_value = "127.0.0.1", env = "WRAPPER_HOST")]
    host: String,
    #[arg(long, default_value_t = 12340, env = "WRAPPER_PORT")]
    port: u16,
    #[arg(long, value_enum, default_value_t = RuntimeKind::Auto)]
    runtime: RuntimeKind,
    /// Wrapper rootfs for the sandbox runtimes [default: <exe dir>/rootfs].
    #[arg(long)]
    rootfs: Option<PathBuf>,
    /// lite binary for the direct runtime.
    #[arg(long, default_value = "/system/bin/lite")]
    lite_bin: PathBuf,
    /// State directory for the direct runtime (sandbox: always <rootfs>/data).
    #[arg(long, default_value = "/data")]
    state_dir: PathBuf,
    /// Proxy for lite and for Apple catalog probes.
    #[arg(long, env = "WRAPPER_PROXY")]
    proxy: Option<String>,
    /// Default lite --device-info for accounts without their own.
    #[arg(long)]
    device_info: Option<String>,
    #[arg(long, default_value = "info")]
    lite_log_level: String,
    #[arg(long, default_value_t = 1800)]
    token_refresh_interval: u32,
    /// Seconds lite waits for a 2FA code (0: lite's default of 60).
    #[arg(long, default_value_t = 300)]
    code_timeout: u32,
    /// Require this bearer token on management endpoints.
    #[arg(long, env = "WRAPPER_API_TOKEN")]
    api_token: Option<String>,
    /// Do not serve wrapper-manager style POST /login and /logout.
    #[arg(long)]
    no_legacy_management: bool,
    /// Concurrent requests forwarded to a single lite instance.
    #[arg(long, default_value_t = 2)]
    instance_concurrency: usize,
    /// Seconds a lite instance may take to become ready.
    #[arg(long, default_value_t = 240)]
    ready_timeout: u64,
    /// lite instances allowed to start at the same time.
    #[arg(long, default_value_t = 2)]
    startup_parallelism: usize,
    /// Seconds between active `/key` health probes of an account whose real
    /// traffic has shown no working key (0: probe only after FairPlay errors
    /// and on `/health?probe=1`).
    #[arg(long, default_value_t = 600)]
    health_probe_interval: u64,
    /// Agent log level (trace, debug, info, warn, error).
    #[arg(long, default_value = "info", env = "WRAPPER_LOG")]
    log_level: String,
    /// Turn public mode on (anyone may add their own account; needs
    /// --api-token). Otherwise the setting saved in agent.json applies.
    #[arg(long, env = "WRAPPER_PUBLIC")]
    public: bool,
    /// Most active accounts public mode accepts (0: no limit); saved.
    #[arg(long, env = "WRAPPER_MAX_ACCOUNTS")]
    max_accounts: Option<u32>,
    /// Do not serve the web panel on the API port.
    #[arg(long)]
    no_ui: bool,
}

#[derive(Args, Default)]
struct GuiArgs {
    /// Address the panel and API listen on [default: the saved setting,
    /// 127.0.0.1]; saved as the new setting.
    #[arg(long)]
    host: Option<String>,
    /// Port of the panel and API [default: the saved setting, 12340]; saved.
    #[arg(long)]
    port: Option<u16>,
    /// Where settings are stored [default: <exe dir>].
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// QEMU guest files [default: <exe dir>/qemu].
    #[arg(long)]
    assets_dir: Option<PathBuf>,
    /// Wrapper rootfs for the native engine [default: <exe dir>/rootfs].
    #[arg(long)]
    rootfs: Option<PathBuf>,
    /// Do not open a browser.
    #[arg(long)]
    no_browser: bool,
    /// Extra admin token, required even on this machine (other machines
    /// always need the management token or this one).
    #[arg(long, env = "WRAPPER_GUI_TOKEN")]
    access_token: Option<String>,
}

#[derive(Args)]
struct QemuArgs {
    /// Directory with the kernel, initramfs and bin/ [default: <exe dir>/qemu].
    #[arg(long)]
    assets_dir: Option<PathBuf>,
    /// Persistent data disk [default: <assets dir>/data.img].
    #[arg(long)]
    data_img: Option<PathBuf>,
    /// Size in MB of a newly created data disk.
    #[arg(long, default_value_t = 512)]
    disk_size: u64,
    #[arg(long, env = "QEMU_BIN")]
    qemu_bin: Option<PathBuf>,
    /// kvm, whpx, hvf, tcg or auto.
    #[arg(long, default_value = "auto", env = "LITE_QEMU_ACCEL")]
    accel: String,
    /// Guest memory in MB (lite needs roughly 150 MB per account).
    #[arg(long, default_value_t = 1024, env = "MEMORY")]
    memory: u32,
    #[arg(long, default_value_t = 2, env = "SMP")]
    smp: u32,
    /// Host address the forwarded port binds to.
    #[arg(long, default_value = "127.0.0.1", env = "LITE_QEMU_HOST")]
    host: String,
    #[arg(long, default_value_t = 12340, env = "HOST_PORT")]
    host_port: u16,
    /// Management API token for the agent.
    #[arg(long, env = "WRAPPER_API_TOKEN")]
    api_token: Option<String>,
    /// Proxy used inside the guest.
    #[arg(long, env = "WRAPPER_PROXY")]
    proxy: Option<String>,
    /// Extra `wrapper agent` arguments for the guest.
    #[arg(last = true)]
    agent_args: Vec<String>,
}

#[derive(Args)]
struct SandboxArgs {
    #[arg(long)]
    rootfs: PathBuf,
    #[arg(long)]
    privileged: bool,
    /// Program (path inside the rootfs) and its arguments.
    #[arg(last = true, required = true)]
    command: Vec<OsString>,
}

/// Drops `WRAPPER_*` variables set to an empty string, so `VAR=` (as Docker
/// Compose passes `${VAR:-}`) means "not set" instead of an invalid value.
fn unset_empty_env() {
    let empty: Vec<OsString> = std::env::vars_os()
        .filter(|(k, v)| v.is_empty() && k.to_str().is_some_and(|k| k.starts_with("WRAPPER_")))
        .map(|(k, _)| k)
        .collect();
    for k in empty {
        // Single-threaded: runs first thing in main, before any thread starts.
        std::env::remove_var(k);
    }
}

fn main() {
    unset_empty_env();
    let cli = Cli::parse();
    let code = match cli.cmd.unwrap_or_else(|| Cmd::Gui(GuiArgs::default())) {
        Cmd::Gui(args) => run_async(run_gui(args)),
        Cmd::SandboxExec(args) => sandbox_exec(args),
        Cmd::Agent(args) => run_async(run_agent(*args)),
        Cmd::Qemu(args) => run_async(run_qemu(*args)),
        Cmd::Login(args) => run_async(client::login(args)),
        Cmd::Accounts(args) => run_async(client::accounts(args)),
        Cmd::Backup(args) => run_async(client::backup(args)),
        Cmd::Restore(args) => run_async(client::restore(args)),
        Cmd::Update(args) => run_async(update::run(args)),
    };
    std::process::exit(code);
}

fn run_async(fut: impl std::future::Future<Output = anyhow::Result<()>>) -> i32 {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    match rt.block_on(fut) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e:#}");
            1
        }
    }
}

#[cfg(target_os = "linux")]
fn sandbox_exec(args: SandboxArgs) -> i32 {
    let mut command = args.command.into_iter();
    let spec = wrapper_agent::sandbox::SandboxSpec {
        rootfs: std::fs::canonicalize(&args.rootfs).unwrap_or(args.rootfs),
        privileged: args.privileged,
        program: command.next().expect("clap requires a command"),
        args: command.collect(),
    };
    match wrapper_agent::sandbox::run(&spec) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("sandbox-exec: {e}");
            126
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn sandbox_exec(_args: SandboxArgs) -> i32 {
    eprintln!("sandbox-exec is only available on Linux");
    126
}

fn init_logging(level: &str) -> std::sync::Arc<EventBus> {
    let bus = EventBus::new(3000);
    let filter = || EnvFilter::try_new(format!("{level},hyper=warn,reqwest=warn")).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()) && std::env::var_os("NO_COLOR").is_none())
                .with_writer(std::io::stderr)
                .with_filter(filter()),
        )
        .with(BusLayer::new(bus.clone()).with_filter(filter()))
        .init();
    bus
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn resolve_runtime(args: &AgentArgs) -> anyhow::Result<(LiteRuntime, Layout)> {
    if !cfg!(target_os = "linux") {
        bail!("the agent runs lite and therefore needs Linux; use the QEMU launcher on this OS");
    }
    let kind = match args.runtime {
        RuntimeKind::Auto if args.lite_bin.exists() => RuntimeKind::Direct,
        RuntimeKind::Auto => RuntimeKind::Sandbox,
        k => k,
    };
    Ok(match kind {
        RuntimeKind::Direct => {
            if !args.lite_bin.exists() {
                bail!("lite binary {} not found", args.lite_bin.display());
            }
            let state = args.state_dir.clone();
            let lite_root = state.to_string_lossy().into_owned();
            (LiteRuntime::Direct { lite_bin: args.lite_bin.clone() }, Layout { state_dir: state, lite_root })
        }
        _ => {
            let rootfs = match &args.rootfs {
                Some(r) => r.clone(),
                None => [exe_dir().join("rootfs"), PathBuf::from("rootfs")]
                    .into_iter()
                    .find(|p| p.join("system/bin/lite").exists())
                    .context("rootfs not found next to the binary; pass --rootfs")?,
            };
            let rootfs = std::fs::canonicalize(&rootfs).with_context(|| format!("rootfs {}", rootfs.display()))?;
            let helper = std::env::current_exe().context("cannot locate own executable")?;
            let layout = Layout { state_dir: rootfs.join("data"), lite_root: "/data".into() };
            let privileged = kind == RuntimeKind::SandboxPrivileged || rootless_needs_privileged(&rootfs)?;
            (LiteRuntime::Sandbox { helper, rootfs, privileged }, layout)
        }
    })
}

/// Whether the rootless sandbox cannot write to `rootfs` but root can: fall
/// back to the privileged one instead of failing every lite start with EACCES.
#[cfg(target_os = "linux")]
fn rootless_needs_privileged(rootfs: &Path) -> anyhow::Result<bool> {
    let Some(problem) = wrapper_agent::sandbox::rootless_problem(rootfs) else {
        return Ok(false);
    };
    if !wrapper_agent::sandbox::can_run_privileged() {
        bail!("{problem}");
    }
    tracing::warn!("{problem}; using the privileged sandbox instead");
    Ok(true)
}

#[cfg(not(target_os = "linux"))]
fn rootless_needs_privileged(_rootfs: &Path) -> anyhow::Result<bool> {
    Ok(false)
}

async fn run_agent(args: AgentArgs) -> anyhow::Result<()> {
    let bus = init_logging(&args.log_level);
    let (runtime, layout) = resolve_runtime(&args)?;
    let listen: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .with_context(|| format!("invalid listen address {}:{}", args.host, args.port))?;

    let mut cfg = AgentConfig::new(listen, layout, runtime);
    cfg.lite = LiteOptions {
        proxy: args.proxy.clone().filter(|p| !p.is_empty()),
        device_info: args.device_info.clone().filter(|d| !d.is_empty()),
        log_level: args.lite_log_level.clone(),
        token_refresh_interval: args.token_refresh_interval,
        code_timeout: args.code_timeout,
    };
    cfg.api_token = args.api_token.clone().filter(|t| !t.is_empty());
    cfg.legacy_management = !args.no_legacy_management;
    cfg.instance_concurrency = args.instance_concurrency;
    cfg.ready_timeout = Duration::from_secs(args.ready_timeout);
    cfg.startup_parallelism = args.startup_parallelism;
    cfg.probe_interval = Duration::from_secs(args.health_probe_interval);

    let listener = tokio::net::TcpListener::bind(listen).await.with_context(|| format!("cannot listen on {listen}"))?;
    let agent = Agent::new(cfg, bus)?;
    if args.public || args.max_accounts.is_some() {
        let patch = wrapper_agent::settings::SettingsPatch {
            public_mode: args.public.then_some(true),
            max_accounts: args.max_accounts,
            ..Default::default()
        };
        agent.update_settings(patch).map_err(anyhow::Error::msg)?;
    }
    tracing::info!("wrapper agent {} listening on http://{listen}", env!("CARGO_PKG_VERSION"));
    if agent.settings().public_mode {
        let max = agent.settings().max_accounts;
        tracing::info!("public mode is on (max accounts: {})", if max == 0 { "no limit".to_string() } else { max.to_string() });
    }
    agent.boot().await;

    let ui = (!args.no_ui).then(wrapper_host::gui::ui_router);
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(wrapper_agent::server::serve(agent.clone(), listener, ui, async {
        let _ = stop_rx.await;
    }));
    tokio::select! {
        _ = shutdown_signal() => {}
        _ = agent.shutdown_requested() => {}
    }
    tracing::info!("shutting down");
    agent.shutdown().await;
    let _ = stop_tx.send(());
    // Open SSE streams never finish on their own; do not wait for them forever.
    let _ = tokio::time::timeout(Duration::from_secs(3), server).await;
    Ok(())
}

async fn run_gui(args: GuiArgs) -> anyhow::Result<()> {
    use wrapper_host::gui::{self, engine::Paths, GuiOptions};
    let _bus = init_logging("info");
    wrapper_host::platform::kill_children_on_exit();
    let exe = wrapper_host::exe_dir();
    let data_dir = args.data_dir.unwrap_or_else(gui::default_data_dir);
    let listen = match (args.host, args.port) {
        (None, None) => None,
        (host, port) => {
            let saved = gui::config::GuiConfig::load(&gui::config::config_path(&data_dir));
            let host = host.unwrap_or(saved.api_host);
            let port = port.unwrap_or(saved.api_port);
            let addr: SocketAddr = format!("{host}:{port}")
                .parse()
                .or_else(|_| format!("[{host}]:{port}").parse())
                .with_context(|| format!("invalid listen address {host}:{port}"))?;
            Some(addr)
        }
    };
    let opts = GuiOptions {
        listen,
        paths: Paths {
            data_dir,
            assets_dir: args.assets_dir.unwrap_or_else(|| exe.join("qemu")),
            rootfs: args.rootfs.unwrap_or_else(|| exe.join("rootfs")),
        },
        open_browser: !args.no_browser,
        access_token: args.access_token.filter(|t| !t.is_empty()),
    };
    gui::run(opts, shutdown_signal()).await
}

async fn run_qemu(args: QemuArgs) -> anyhow::Result<()> {
    use wrapper_host::qemu::{run_vm, QemuConfig};
    wrapper_host::platform::kill_children_on_exit();
    let assets = args.assets_dir.unwrap_or_else(|| wrapper_host::exe_dir().join("qemu"));
    let mut cfg = QemuConfig::new(assets);
    if let Some(d) = args.data_img {
        cfg.data_img = d;
    }
    cfg.disk_size_mb = args.disk_size;
    cfg.qemu_bin = args.qemu_bin;
    cfg.accel = Some(args.accel).filter(|a| a != "auto" && !a.is_empty());
    cfg.memory_mb = args.memory;
    cfg.smp = args.smp;
    cfg.host = args.host;
    cfg.host_port = args.host_port;
    cfg.api_token = args.api_token.filter(|t| !t.is_empty());
    if let Some(p) = args.proxy.filter(|p| !p.is_empty()) {
        cfg.agent_args.extend(["--proxy".into(), p]);
    }
    cfg.agent_args.extend(args.agent_args);

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = stop_tx.send(true);
    });
    let sink: wrapper_host::qemu::LineSink = std::sync::Arc::new(|line: &str| println!("{line}"));
    let status = run_vm(&cfg, stop_rx, sink).await?;
    if !status.success() {
        bail!("QEMU exited with {status}");
    }
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(windows)]
    {
        // Closing the console window gives us a few seconds to shut down.
        let mut close = tokio::signal::windows::ctrl_close().expect("console close handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = close.recv() => {}
        }
    }
}
