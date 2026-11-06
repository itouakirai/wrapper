//! Self-update from GitHub releases (`wrapper update`).
//!
//! Every release publishes `manifest.json` (and `manifest.json.sig`, an
//! Ed25519 signature checked against the key compiled into the binary, see
//! `update-key.pub`). Per platform it lists the components an install is made
//! of:
//!
//! | component | installed at                     | asset                      |
//! |-----------|----------------------------------|----------------------------|
//! | `guest`   | `qemu/` kernel + initramfs       | `guest.tar.gz`             |
//! | `qemu`    | `qemu/bin/`                      | `qemu-<platform>.tar.gz`   |
//! | `rootfs`  | `rootfs/` except `data/`         | `rootfs-<platform>.tar.gz` |
//! | `bin`     | the `wrapper` executable         | `wrapper-<platform>[.exe]` |
//!
//! Each entry carries the content [`digest`] of the installed form, computed
//! in CI by this very code (`wrapper update --digest`), so an update fetches
//! only what changed and never adds a component the user removed.
//!
//! Nothing is replaced before everything is downloaded, verified, unpacked
//! and (for the binary) test-run. The swap itself is a sequence of renames
//! inside the install directory, undone in reverse on the first failure.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const DEFAULT_REPO: &str = "itouakirai/wrapper";
const MIRROR: &str = "https://gh-proxy.com/";
const MANIFEST: &str = "manifest.json";
const SCHEMA: u32 = 1;
/// Hex Ed25519 public key; empty in builds without `update-key.pub`.
const PUBKEY: &str = env!("WRAPPER_UPDATE_PUBKEY");
/// Kernel and initramfs: the `guest` component inside `qemu/`.
const GUEST_FILES: [&str; 2] = ["vmlinuz-lite-qemu", "lite-initramfs.cpio.gz"];
/// User state inside `rootfs/`, carried over into the new tree.
const ROOTFS_KEEP: [&str; 1] = ["data"];
/// Scratch directory (in the install dir, so renames stay on one filesystem).
const STAGE: &str = ".wrapper-update";
const LOCK: &str = ".wrapper-update.lock";

// ------------------------------------------------------------- manifest

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    /// Version without the `v`, as in `Cargo.toml`.
    pub version: String,
    pub tag: String,
    #[serde(default)]
    pub notes: Option<String>,
    /// platform -> component name -> asset.
    pub platforms: BTreeMap<String, BTreeMap<String, Asset>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub asset: String,
    pub size: u64,
    /// Of the downloaded file.
    pub sha256: String,
    /// Of the installed form, see [`digest`].
    pub digest: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    Guest,
    Qemu,
    Rootfs,
    Bin,
}

impl Component {
    /// Install order: the binary goes last, after everything it depends on.
    pub const ALL: [Component; 4] = [Component::Guest, Component::Qemu, Component::Rootfs, Component::Bin];

    pub fn name(self) -> &'static str {
        match self {
            Component::Guest => "guest",
            Component::Qemu => "qemu",
            Component::Rootfs => "rootfs",
            Component::Bin => "bin",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == s)
    }
}

/// Release platform name of this build (`linux-x86_64`, ...). macOS has no
/// release packages since v3.0.4 (it runs the Docker image instead).
pub fn platform() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("windows", "x86_64") => "windows-x86_64",
        ("android", "aarch64") => "android-aarch64",
        _ => return None,
    })
}

pub fn signing_enabled() -> bool {
    !PUBKEY.is_empty()
}

/// `true` when `candidate` is a later version than `current` (`v` prefix and
/// pre-release suffixes allowed; a release beats its pre-releases).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parse(v: &str) -> ([u64; 3], Option<&str>) {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (v, None),
        };
        let mut n = [0u64; 3];
        for (slot, part) in n.iter_mut().zip(core.split('.')) {
            *slot = part.parse().unwrap_or(0);
        }
        (n, pre)
    }
    let (a, ap) = parse(candidate);
    let (b, bp) = parse(current);
    match a.cmp(&b) {
        std::cmp::Ordering::Equal => match (ap, bp) {
            (None, Some(_)) => true,
            (Some(x), Some(y)) => x > y,
            _ => false,
        },
        o => o.is_gt(),
    }
}

// ------------------------------------------------------------- digests

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(hex(&h.finalize()));
        }
        h.update(&buf[..n]);
    }
}

/// Content digest of a component installed at `path` (the executable for
/// `bin`, `qemu/` for `guest`, `qemu/bin/` for `qemu`, `rootfs/` for
/// `rootfs`). Trees hash sorted `/`-separated paths with file contents and
/// symlink targets; modes, timestamps and empty directories do not count, so
/// the same files give the same digest on every OS.
pub fn digest(component: Component, path: &Path) -> std::io::Result<String> {
    match component {
        Component::Bin => sha256_file(path),
        Component::Guest => tree_digest(path, &|rel, is_dir| !is_dir && GUEST_FILES.contains(&rel)),
        Component::Qemu => tree_digest(path, &|_, _| true),
        Component::Rootfs => tree_digest(path, &|rel, _| !ROOTFS_KEEP.contains(&rel)),
    }
}

fn tree_digest(root: &Path, include: &dyn Fn(&str, bool) -> bool) -> std::io::Result<String> {
    fn walk(
        dir: &Path,
        prefix: &str,
        include: &dyn Fn(&str, bool) -> bool,
        out: &mut Vec<(String, String)>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            let ty = entry.file_type()?;
            if !include(&rel, ty.is_dir()) {
                continue;
            }
            if ty.is_dir() {
                walk(&entry.path(), &rel, include, out)?;
            } else if ty.is_symlink() {
                let target = std::fs::read_link(entry.path())?;
                out.push((rel, format!("l {}", target.to_string_lossy().replace('\\', "/"))));
            } else if ty.is_file() {
                out.push((rel, format!("f {}", sha256_file(&entry.path())?)));
            }
        }
        Ok(())
    }
    let mut entries = Vec::new();
    walk(root, "", include, &mut entries)?;
    entries.sort();
    let mut h = Sha256::new();
    for (rel, what) in entries {
        h.update(rel.as_bytes());
        h.update(b"\0");
        h.update(what.as_bytes());
        h.update(b"\n");
    }
    Ok(hex(&h.finalize()))
}

// ------------------------------------------------------------- install

/// Where the components of an install live.
#[derive(Clone, Debug)]
pub struct Install {
    pub root: PathBuf,
    pub exe: PathBuf,
}

impl Install {
    /// The install the running executable belongs to.
    pub fn current() -> anyhow::Result<Self> {
        let exe = std::env::current_exe().context("cannot locate own executable")?;
        let exe = exe.canonicalize().unwrap_or(exe);
        let root = exe.parent().context("executable has no parent directory")?.to_path_buf();
        Ok(Self { root, exe })
    }

    /// An install in another directory (`wrapper[.exe]` inside `root`).
    pub fn at(root: &Path) -> anyhow::Result<Self> {
        let root = root.canonicalize().with_context(|| format!("install directory {}", root.display()))?;
        let exe = root.join(format!("wrapper{}", std::env::consts::EXE_SUFFIX));
        Ok(Self { root, exe })
    }

    pub fn path(&self, c: Component) -> PathBuf {
        match c {
            Component::Bin => self.exe.clone(),
            Component::Guest => self.root.join("qemu"),
            Component::Qemu => self.root.join("qemu/bin"),
            Component::Rootfs => self.root.join("rootfs"),
        }
    }

    /// Whether the install has the component at all (we never add one).
    pub fn has(&self, c: Component) -> bool {
        match c {
            Component::Bin => self.exe.is_file(),
            Component::Guest => self.root.join("qemu").join(GUEST_FILES[1]).is_file(),
            Component::Qemu => self.root.join("qemu/bin").is_dir(),
            Component::Rootfs => self.root.join("rootfs/system/bin/lite").is_file(),
        }
    }

    /// Removes what an earlier update could not (on Windows: the replaced
    /// executable, still running back then), unless an update is running.
    pub fn remove_leftovers(&self) {
        if !self.root.join(LOCK).exists() {
            let _ = std::fs::remove_dir_all(self.root.join(STAGE));
        }
    }

    /// Reason this install must not be updated in place, if any. `force`
    /// overrides the guesses but not the impossible.
    pub fn refuse_reason(&self, force: bool) -> Option<String> {
        let name = self.exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.ends_with(".so") {
            return Some("this is the Android app; install the new APK instead".into());
        }
        if force {
            return None;
        }
        if Path::new("/.dockerenv").exists() {
            return Some("running in a container; pull the new image instead (or pass --force)".into());
        }
        let in_target = self.root.components().any(|c| c.as_os_str() == "target")
            && self.root.ancestors().any(|a| a.join("Cargo.toml").is_file());
        if in_target {
            return Some("this is a cargo build; rebuild from source instead (or pass --force)".into());
        }
        None
    }
}

/// Other processes running from the install (wrapper, QEMU, chrooted lite),
/// which would keep stale files open or lose `rootfs/data` mid-write.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn processes_using(root: &Path) -> Vec<(u32, PathBuf)> {
    let me = std::process::id();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut found: Vec<_> = dir
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|&pid| pid != me)
        .filter_map(|pid| Some((pid, std::fs::read_link(format!("/proc/{pid}/exe")).ok()?)))
        .filter(|(_, exe)| exe.starts_with(root))
        .collect();
    found.sort();
    found
}

/// Not detectable here; Windows refuses to rename files in use instead.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub fn processes_using(_root: &Path) -> Vec<(u32, PathBuf)> {
    Vec::new()
}

// ------------------------------------------------------------- fetching

/// Where releases come from.
#[derive(Clone, Debug)]
pub struct Source {
    /// `owner/name` on GitHub, or the URL of anything laid out like a GitHub
    /// releases page (`<url>/latest/download/<file>`,
    /// `<url>/download/<tag>/<file>`), e.g. a self-hosted mirror.
    pub repo: String,
    /// Download through gh-proxy (mainland China). Safe with a signed
    /// manifest: every file is checked against it.
    pub mirror: bool,
    pub proxy: Option<String>,
}

impl Source {
    pub fn new(repo: &str) -> Self {
        Self { repo: repo.to_string(), mirror: false, proxy: None }
    }

    /// `tag: None` resolves to the latest release through GitHub's redirect,
    /// which needs no API token and has no API rate limit.
    fn url(&self, tag: Option<&str>, file: &str) -> String {
        let base = if self.repo.contains("://") {
            self.repo.trim_end_matches('/').to_string()
        } else {
            format!("https://github.com/{}/releases", self.repo)
        };
        let url = match tag {
            Some(t) => format!("{base}/download/{t}/{file}"),
            None => format!("{base}/latest/download/{file}"),
        };
        if self.mirror {
            format!("{MIRROR}{url}")
        } else {
            url
        }
    }

    fn client(&self) -> anyhow::Result<reqwest::Client> {
        let mut b = reqwest::Client::builder()
            .user_agent(concat!("wrapper/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(20))
            .read_timeout(Duration::from_secs(60));
        if let Some(p) = self.proxy.as_deref().filter(|p| !p.is_empty()) {
            let p = if p.contains("://") { p.to_string() } else { format!("http://{p}") };
            b = b.proxy(reqwest::Proxy::all(p)?);
        }
        Ok(b.build()?)
    }

    async fn get(&self, client: &reqwest::Client, url: &str) -> anyhow::Result<reqwest::Response> {
        let resp = client.get(url).send().await.with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            bail!("GET {url}: HTTP {}", resp.status());
        }
        Ok(resp)
    }

    /// Fetches and verifies the manifest of `tag` (default: latest release).
    /// Returns whether it carried a valid signature (always, when this build
    /// has a key; never otherwise).
    pub async fn manifest(&self, tag: Option<&str>) -> anyhow::Result<(Manifest, bool)> {
        let client = self.client()?;
        let body = self.get(&client, &self.url(tag, MANIFEST)).await?.bytes().await?;
        let signed = if signing_enabled() {
            let sig_url = self.url(tag, &format!("{MANIFEST}.sig"));
            let sig = self.get(&client, &sig_url).await?.bytes().await?;
            verify_signature(&body, &sig).context("manifest signature is invalid; refusing to update")?;
            true
        } else {
            false
        };
        let manifest: Manifest = serde_json::from_slice(&body).context("malformed release manifest")?;
        if manifest.schema > SCHEMA {
            bail!("release {} needs a newer updater; download it manually", manifest.tag);
        }
        if let Some(t) = tag {
            // A signed manifest of another release must not stand in for this one.
            if manifest.tag != t {
                bail!("asked for {t} but the manifest describes {}", manifest.tag);
            }
        }
        Ok((manifest, signed))
    }
}

fn verify_signature(msg: &[u8], sig: &[u8]) -> anyhow::Result<()> {
    use ed25519_dalek::{Signature, VerifyingKey};
    let key: [u8; 32] = (0..PUBKEY.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(PUBKEY.get(i..i + 2).unwrap_or("zz"), 16))
        .collect::<Result<Vec<u8>, _>>()
        .ok()
        .and_then(|v| v.try_into().ok())
        .context("update-key.pub is not a 32-byte hex key")?;
    let key = VerifyingKey::from_bytes(&key).context("update-key.pub is not a valid Ed25519 key")?;
    let sig = Signature::from_slice(sig).context("signature has the wrong length")?;
    key.verify_strict(msg, &sig)?;
    Ok(())
}

// ------------------------------------------------------------- planning

#[derive(Clone, Debug)]
pub struct Step {
    pub component: Component,
    pub asset: Asset,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub manifest: Manifest,
    pub steps: Vec<Step>,
    /// Components left alone, with the reason.
    pub skipped: Vec<(Component, &'static str)>,
}

impl Plan {
    pub fn download_size(&self) -> u64 {
        self.steps.iter().map(|s| s.asset.size).sum()
    }
}

/// Decides what to replace. `only` limits the components; `force` replaces
/// them even when their digest already matches. Hashes local trees, so run
/// it off the async runtime.
pub fn plan(install: &Install, manifest: Manifest, only: &[Component], force: bool) -> anyhow::Result<Plan> {
    let plat = platform().context(
        "no prebuilt releases for this platform (macOS: run the Docker image, see the README's Docker section)",
    )?;
    let assets = manifest
        .platforms
        .get(plat)
        .with_context(|| format!("release {} has no {plat} build", manifest.tag))?
        .clone();
    let (mut steps, mut skipped) = (Vec::new(), Vec::new());
    for c in Component::ALL {
        let reason = if !only.is_empty() && !only.contains(&c) {
            Some("not selected")
        } else if !install.has(c) {
            Some("not installed")
        } else {
            match assets.get(c.name()) {
                None => Some("not in this release"),
                Some(a) if !force && digest(c, &install.path(c)).ok().as_deref() == Some(&a.digest) => {
                    Some("unchanged")
                }
                Some(a) => {
                    steps.push(Step { component: c, asset: a.clone() });
                    None
                }
            }
        };
        if let Some(r) = reason {
            skipped.push((c, r));
        }
    }
    Ok(Plan { manifest, steps, skipped })
}

// ------------------------------------------------------------- applying

#[derive(Clone, Copy, Debug)]
pub enum Progress {
    Download { component: Component, done: u64, total: u64 },
    Verify(Component),
    Install(Component),
}

/// Downloads, verifies and installs `plan`. Either every step is installed
/// or (on error) the install is left as it was.
pub async fn apply(src: &Source, install: &Install, plan: &Plan, progress: &dyn Fn(Progress)) -> anyhow::Result<()> {
    let _lock = Lock::acquire(&install.root)?;
    let stage = install.root.join(STAGE);
    // Leftovers of an earlier run (on Windows: the replaced executable).
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).with_context(|| format!("cannot create {}", stage.display()))?;
    let client = src.client()?;

    let mut staged = Vec::new();
    for step in &plan.steps {
        let c = step.component;
        let file = stage.join(&step.asset.asset);
        let url = src.url(Some(&plan.manifest.tag), &step.asset.asset);
        download(&client, &url, &file, &step.asset, &|done| {
            progress(Progress::Download { component: c, done, total: step.asset.size })
        })
        .await?;
        progress(Progress::Verify(c));
        let (file, asset, version) = (file.clone(), step.asset.clone(), plan.manifest.version.clone());
        let dir = stage.join(c.name());
        let ready = tokio::task::spawn_blocking(move || prepare(c, &file, &dir, &asset, &version)).await??;
        staged.push((c, ready));
    }

    let mut journal = Journal::default();
    for (c, ready) in &staged {
        progress(Progress::Install(*c));
        if let Err(e) = commit(install, *c, ready, &stage.join("old"), &mut journal) {
            journal.rollback();
            return Err(e.context(format!("installing {} failed; nothing was changed", c.name())));
        }
    }
    // Fails for the old executable on Windows while it runs; the next update
    // removes it.
    let _ = std::fs::remove_dir_all(&stage);
    Ok(())
}

async fn download(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    asset: &Asset,
    progress: &dyn Fn(u64),
) -> anyhow::Result<()> {
    let resp = client.get(url).send().await.with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        bail!("GET {url}: HTTP {}", resp.status());
    }
    let mut file = std::fs::File::create(dest).with_context(|| format!("cannot write {}", dest.display()))?;
    let mut h = Sha256::new();
    let mut done = 0u64;
    let mut stream = resp.bytes_stream();
    let mut last = std::time::Instant::now();
    progress(0);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("downloading {}", asset.asset))?;
        done += chunk.len() as u64;
        if done > asset.size {
            bail!("{} is larger than the manifest says", asset.asset);
        }
        h.update(&chunk);
        file.write_all(&chunk)?;
        if last.elapsed() >= Duration::from_millis(200) {
            progress(done);
            last = std::time::Instant::now();
        }
    }
    progress(done);
    file.sync_all()?;
    let sum = hex(&h.finalize());
    if done != asset.size || !sum.eq_ignore_ascii_case(&asset.sha256) {
        bail!("{} does not match the release manifest (corrupt or tampered download)", asset.asset);
    }
    Ok(())
}

/// Turns a downloaded asset into the exact thing that will be renamed into
/// place, and checks it: the unpacked tree must have the manifest digest, the
/// binary must run and report the expected version.
fn prepare(c: Component, file: &Path, dir: &Path, asset: &Asset, version: &str) -> anyhow::Result<PathBuf> {
    if c == Component::Bin {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755))?;
        }
        let out = std::process::Command::new(file)
            .arg("--version")
            .output()
            .context("the new binary does not run on this system")?;
        let printed = String::from_utf8_lossy(&out.stdout);
        if !out.status.success() || !printed.split_whitespace().any(|w| w == version) {
            bail!("the new binary reports {:?} instead of version {version}", printed.trim());
        }
        return Ok(file.to_path_buf());
    }
    std::fs::create_dir_all(dir)?;
    let gz = flate2::read::GzDecoder::new(std::fs::File::open(file)?);
    let mut archive = tar::Archive::new(gz);
    archive.set_preserve_permissions(true);
    archive.set_unpack_xattrs(false);
    // `unpack` keeps every entry inside `dir` (no absolute paths, no `..`).
    archive.unpack(dir).with_context(|| format!("cannot unpack {}", asset.asset))?;
    if digest(c, dir)? != asset.digest {
        bail!("{} unpacked to different files than the release manifest describes", asset.asset);
    }
    let _ = std::fs::remove_file(file);
    Ok(dir.to_path_buf())
}

/// Renames performed so far, so they can be undone.
#[derive(Default)]
struct Journal(Vec<(PathBuf, PathBuf)>);

impl Journal {
    fn mv(&mut self, from: &Path, to: &Path) -> anyhow::Result<()> {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(from, to).with_context(|| {
            format!("cannot move {} to {} (is wrapper or QEMU still running?)", from.display(), to.display())
        })?;
        self.0.push((from.to_path_buf(), to.to_path_buf()));
        Ok(())
    }

    fn rollback(self) {
        for (from, to) in self.0.into_iter().rev() {
            if let Err(e) = std::fs::rename(&to, &from) {
                tracing::error!("rollback: cannot move {} back to {}: {e}", to.display(), from.display());
            }
        }
    }
}

fn commit(install: &Install, c: Component, ready: &Path, old: &Path, j: &mut Journal) -> anyhow::Result<()> {
    let target = install.path(c);
    let backup = old.join(c.name());
    match c {
        Component::Bin => {
            // Windows allows renaming (not overwriting) a running executable.
            j.mv(&target, &backup)?;
            j.mv(ready, &target)?;
        }
        Component::Guest => {
            for f in GUEST_FILES {
                if target.join(f).exists() {
                    j.mv(&target.join(f), &backup.join(f))?;
                }
                j.mv(&ready.join(f), &target.join(f))?;
            }
        }
        Component::Qemu | Component::Rootfs => {
            j.mv(&target, &backup)?;
            j.mv(ready, &target)?;
            if c == Component::Rootfs {
                for keep in ROOTFS_KEEP {
                    let (from, to) = (backup.join(keep), target.join(keep));
                    if from.exists() {
                        // The archive has no user state, but be sure.
                        let _ = std::fs::remove_dir_all(&to);
                        j.mv(&from, &to)?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// One update per install at a time; a lock older than an hour is stale.
struct Lock(PathBuf);

impl Lock {
    fn acquire(root: &Path) -> anyhow::Result<Self> {
        let path = root.join(LOCK);
        let stale = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .is_ok_and(|t| SystemTime::now().duration_since(t).unwrap_or_default() > Duration::from_secs(3600));
        if stale {
            let _ = std::fs::remove_file(&path);
        }
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                let _ = write!(f, "{}", std::process::id());
                Ok(Self(path))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                bail!("another update is running (remove {} if it is not)", path.display())
            }
            Err(e) => Err(e).with_context(|| format!("cannot write to {} (permissions?)", root.display())),
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wrapper-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn version_order() {
        assert!(is_newer("v3.1.0", "3.0.1"));
        assert!(is_newer("3.0.10", "3.0.9"));
        assert!(is_newer("3.1.0", "3.1.0-rc.1"));
        assert!(!is_newer("3.1.0-rc.1", "3.1.0"));
        assert!(!is_newer("v3.0.1", "3.0.1"));
        assert!(!is_newer("2.9.9", "3.0.0"));
    }

    #[test]
    fn digest_ignores_user_state_and_order() {
        let a = tmp("digest-a");
        std::fs::create_dir_all(a.join("system/bin")).unwrap();
        std::fs::create_dir_all(a.join("data")).unwrap();
        std::fs::write(a.join("system/bin/lite"), "lite").unwrap();
        std::fs::write(a.join("data/accounts.json"), "secret").unwrap();
        let before = digest(Component::Rootfs, &a).unwrap();
        std::fs::write(a.join("data/accounts.json"), "changed").unwrap();
        assert_eq!(before, digest(Component::Rootfs, &a).unwrap());
        std::fs::write(a.join("system/bin/lite"), "lite2").unwrap();
        assert_ne!(before, digest(Component::Rootfs, &a).unwrap());

        let q = tmp("digest-q");
        std::fs::create_dir_all(q.join("bin")).unwrap();
        std::fs::write(q.join("bin/qemu"), "q").unwrap();
        std::fs::write(q.join("data.img"), "disk").unwrap();
        std::fs::write(q.join(GUEST_FILES[0]), "k").unwrap();
        let g = digest(Component::Guest, &q).unwrap();
        std::fs::write(q.join("data.img"), "disk2").unwrap();
        std::fs::write(q.join("bin/qemu"), "q2").unwrap();
        assert_eq!(g, digest(Component::Guest, &q).unwrap());
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&q);
    }

    #[test]
    fn rootfs_swap_keeps_data_and_rolls_back() {
        let root = tmp("swap");
        let install = Install { root: root.clone(), exe: root.join("wrapper") };
        std::fs::create_dir_all(root.join("rootfs/system/bin")).unwrap();
        std::fs::create_dir_all(root.join("rootfs/data")).unwrap();
        std::fs::write(root.join("rootfs/system/bin/lite"), "old").unwrap();
        std::fs::write(root.join("rootfs/data/accounts.json"), "mine").unwrap();
        let ready = root.join(STAGE).join("rootfs");
        std::fs::create_dir_all(ready.join("system/bin")).unwrap();
        std::fs::write(ready.join("system/bin/lite"), "new").unwrap();

        let mut j = Journal::default();
        commit(&install, Component::Rootfs, &ready, &root.join(STAGE).join("old"), &mut j).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("rootfs/system/bin/lite")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(root.join("rootfs/data/accounts.json")).unwrap(), "mine");

        j.rollback();
        assert_eq!(std::fs::read_to_string(root.join("rootfs/system/bin/lite")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(root.join("rootfs/data/accounts.json")).unwrap(), "mine");
        assert!(ready.join("system/bin/lite").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bad_signature_is_rejected() {
        if signing_enabled() {
            assert!(verify_signature(b"{}", &[0u8; 64]).is_err());
        }
    }
}
