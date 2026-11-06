//! Downloads the prebuilt package for this platform (QEMU, guest files,
//! rootfs) from nightly.link and installs it next to the executable.

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    /// `downloading`, `extracting`, `done` or `error`.
    pub status: &'static str,
    pub downloaded: u64,
    pub total: Option<u64>,
    /// Bytes per second.
    pub speed: u64,
    pub message: Option<String>,
}

impl DownloadProgress {
    fn new(status: &'static str) -> Self {
        Self { status, downloaded: 0, total: None, speed: 0, message: None }
    }
}

/// Artifact name of the package for the running platform.
pub fn package_name() -> Option<&'static str> {
    let name = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "wrapper-windows-x86_64.zip",
        ("linux", "x86_64") => "wrapper-linux-x86_64.zip",
        ("linux", "aarch64") => "wrapper-linux-aarch64.zip",
        ("android", "aarch64") => "wrapper-android-aarch64.zip",
        _ => return None,
    };
    Some(name)
}

pub fn package_url(base: &str, mirror: bool) -> Option<String> {
    let url = format!("{}/{}", base.trim_end_matches('/'), package_name()?);
    Some(if mirror { format!("https://gh-proxy.com/{url}") } else { url })
}

/// Entries never overwritten when they already exist (user state).
fn is_user_state(rel: &Path) -> bool {
    let s = rel.to_string_lossy().replace('\\', "/");
    s == "qemu/data.img" || s.starts_with("rootfs/data/") || s.ends_with("wrapper-gui.json")
}

pub async fn download_and_install(
    url: &str,
    dest: &Path,
    proxy: Option<&str>,
    progress: impl Fn(DownloadProgress),
) -> anyhow::Result<()> {
    let mut b = reqwest::Client::builder().connect_timeout(Duration::from_secs(20));
    if let Some(p) = proxy.filter(|p| !p.is_empty()) {
        let p = if p.contains("://") { p.to_string() } else { format!("http://{p}") };
        b = b.proxy(reqwest::Proxy::all(p)?);
    }
    let resp = b.build()?.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("download failed: HTTP {}", resp.status());
    }
    let total = resp.content_length();
    std::fs::create_dir_all(dest)?;
    let tmp = dest.join(".wrapper-download.zip");
    let mut file = std::fs::File::create(&tmp)?;
    let mut stream = resp.bytes_stream();
    let mut p = DownloadProgress { total, ..DownloadProgress::new("downloading") };
    progress(p.clone());
    let (mut last, mut last_bytes) = (Instant::now(), 0u64);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)?;
        p.downloaded += chunk.len() as u64;
        let dt = last.elapsed();
        if dt >= Duration::from_millis(500) {
            p.speed = ((p.downloaded - last_bytes) as f64 / dt.as_secs_f64()) as u64;
            progress(p.clone());
            last = Instant::now();
            last_bytes = p.downloaded;
        }
    }
    drop(file);
    progress(DownloadProgress { downloaded: p.downloaded, total, ..DownloadProgress::new("extracting") });
    let dest_owned = dest.to_path_buf();
    let tmp_owned = tmp.clone();
    let result = tokio::task::spawn_blocking(move || extract(&tmp_owned, &dest_owned)).await?;
    let _ = std::fs::remove_file(&tmp);
    let n = result?;
    progress(DownloadProgress { message: Some(format!("{n} files installed")), ..DownloadProgress::new("done") });
    Ok(())
}

/// Extracts `zip` into `dest`; returns the number of files written.
pub fn extract(zip: &Path, dest: &Path) -> anyhow::Result<usize> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip)?)?;
    let current_exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    let mut written = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else {
            continue; // path traversal attempt
        };
        if rel.components().any(|c| matches!(c, Component::ParentDir)) {
            continue;
        }
        let out: PathBuf = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if out.exists() && is_user_state(&rel) {
            continue;
        }
        // The running executable cannot be replaced (Windows) and should not
        // change underneath us anyway.
        if let (Some(cur), Ok(o)) = (&current_exe, out.canonicalize()) {
            if *cur == o {
                continue;
            }
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut f)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let name = rel.to_string_lossy();
            let mode = entry.unix_mode().unwrap_or(0o644);
            let exec = mode & 0o111 != 0 || name.contains("bin/") || name.ends_with(".so") || name.starts_with("wrapper");
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(if exec { 0o755 } else { 0o644 }));
        }
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protects_user_state() {
        assert!(is_user_state(Path::new("qemu/data.img")));
        assert!(is_user_state(Path::new("rootfs/data/accounts.json")));
        assert!(!is_user_state(Path::new("qemu/lite-initramfs.cpio.gz")));
    }

    #[test]
    fn extracts_and_skips_traversal() {
        let dir = std::env::temp_dir().join(format!("wrapper-pkg-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zpath = dir.join("t.zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zpath).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("qemu/a.txt", o).unwrap();
            w.write_all(b"new").unwrap();
            w.start_file("qemu/data.img", o).unwrap();
            w.write_all(b"blank").unwrap();
            w.start_file("../evil.txt", o).unwrap();
            w.write_all(b"x").unwrap();
            w.finish().unwrap();
        }
        let out = dir.join("out");
        std::fs::create_dir_all(out.join("qemu")).unwrap();
        std::fs::write(out.join("qemu/data.img"), "mine").unwrap();
        let n = extract(&zpath, &out).unwrap();
        assert_eq!(n, 1);
        assert_eq!(std::fs::read_to_string(out.join("qemu/a.txt")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(out.join("qemu/data.img")).unwrap(), "mine");
        assert!(!dir.join("evil.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
