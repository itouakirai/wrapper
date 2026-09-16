#!/usr/bin/env python3
"""
bundle_termux_deps.py

Downloads Termux headless QEMU (aarch64) and recursively resolves, downloads,
and bundles all its shared library (.so) dependencies and firmware into the
target directory.

All symlinks are dereferenced so that every library file is a self-contained
ELF binary, preventing linking issues in Android APK assets and non-root
environments. Non-x86 firmware is omitted to keep package size lean.
"""

import argparse
import concurrent.futures
import gzip
import io
import lzma
import os
import re
import shutil
import struct
import sys
import tarfile
import urllib.request

TERMUX_BASE = "https://packages.termux.dev/apt/termux-main"
PACKAGES_URL = f"{TERMUX_BASE}/dists/stable/main/binary-aarch64/Packages"
CONTENTS_URL = f"{TERMUX_BASE}/dists/stable/main/Contents-aarch64.gz"

# Android system Bionic libraries provided by the OS, not packaged by Termux
BIONIC_LIBS = {
    "libc.so",
    "libm.so",
    "libdl.so",
    "liblog.so",
    "libaaudio.so",
    "libandroid.so",
    "libEGL.so",
    "libGLESv1_CM.so",
    "libGLESv2.so",
    "libGLESv3.so",
    "libOpenMAXAL.so",
    "libOpenSLES.so",
}


def log(msg):
    print(f"[bundle-deps] {msg}", flush=True)


def download_url(url, dest_path=None, retries=3):
    for attempt in range(retries):
        try:
            req = urllib.request.Request(
                url,
                headers={"User-Agent": "wrapper-lite-bundler/1.0"}
            )
            with urllib.request.urlopen(req, timeout=30) as resp:
                data = resp.read()
            if dest_path:
                os.makedirs(os.path.dirname(os.path.abspath(dest_path)), exist_ok=True)
                with open(dest_path, "wb") as f:
                    f.write(data)
            return data
        except Exception as e:
            if attempt == retries - 1:
                raise RuntimeError(f"Failed to fetch {url}: {e}")
            log(f"Retry {attempt + 1} for {url} ({e})")


def get_dt_needed(elf_data):
    """Extract DT_NEEDED entries from an ELF 64-bit Little Endian binary."""
    if len(elf_data) < 64 or elf_data[:4] != b"\x7fELF":
        return []
    # e_ident[4] == 2 (64-bit), e_ident[5] == 1 (little endian)
    if elf_data[4] != 2 or elf_data[5] != 1:
        return []
    try:
        e_phoff = struct.unpack("<Q", elf_data[32:40])[0]
        e_phentsize = struct.unpack("<H", elf_data[54:56])[0]
        e_phnum = struct.unpack("<H", elf_data[56:58])[0]

        dynamic_offset = 0
        dynamic_size = 0
        for i in range(e_phnum):
            ph = elf_data[e_phoff + i * e_phentsize : e_phoff + (i + 1) * e_phentsize]
            p_type = struct.unpack("<I", ph[:4])[0]
            if p_type == 2:  # PT_DYNAMIC
                dynamic_offset = struct.unpack("<Q", ph[8:16])[0]
                dynamic_size = struct.unpack("<Q", ph[32:40])[0]
                break

        if dynamic_offset == 0:
            return []

        dt_needed_offsets = []
        strtab_vaddr = 0
        for i in range(0, dynamic_size, 16):
            tag, val = struct.unpack("<QQ", elf_data[dynamic_offset + i : dynamic_offset + i + 16])
            if tag == 0:  # DT_NULL
                break
            elif tag == 1:  # DT_NEEDED
                dt_needed_offsets.append(val)
            elif tag == 5:  # DT_STRTAB
                strtab_vaddr = val

        strtab_offset = 0
        for i in range(e_phnum):
            ph = elf_data[e_phoff + i * e_phentsize : e_phoff + (i + 1) * e_phentsize]
            p_type = struct.unpack("<I", ph[:4])[0]
            if p_type == 1:  # PT_LOAD
                p_offset = struct.unpack("<Q", ph[8:16])[0]
                p_vaddr = struct.unpack("<Q", ph[16:24])[0]
                p_memsz = struct.unpack("<Q", ph[40:48])[0]
                if p_vaddr <= strtab_vaddr < p_vaddr + p_memsz:
                    strtab_offset = p_offset + (strtab_vaddr - p_vaddr)
                    break

        needed = []
        for off in dt_needed_offsets:
            end = elf_data.find(b"\x00", strtab_offset + off)
            needed.append(elf_data[strtab_offset + off : end].decode("ascii", errors="replace"))
        return needed
    except Exception:
        return []


def parse_ar_deb(deb_data):
    """Parse Debian ar package and extract data.tar.* content."""
    offset = 8
    while offset < len(deb_data):
        header = deb_data[offset : offset + 60]
        if len(header) < 60:
            break
        name = header[:16].decode("ascii", errors="replace").strip()
        size = int(header[48:58].decode("ascii", errors="replace").strip())
        offset += 60
        data = deb_data[offset : offset + size]
        offset += size + (size % 2)
        if "data.tar" in name:
            if name.endswith(".xz") or name.endswith(".xz/"):
                return lzma.decompress(data)
            elif name.endswith(".gz") or name.endswith(".gz/"):
                return gzip.decompress(data)
            else:
                return data
    raise RuntimeError("No data.tar member found in .deb archive")


def is_x86_firmware(filename):
    """Filter firmware files to only those needed by x86/x86_64 QEMU machines."""
    basename = os.path.basename(filename)
    if "keymaps/" in filename:
        return True
    if basename.startswith(("bios", "vgabios", "kvmvapic", "linuxboot", "multiboot", "pvh", "qboot", "efi-", "pxe-")):
        return True
    if basename.startswith(("edk2-i386", "edk2-x86_64")):
        return True
    return False


def is_so_name(name):
    """Check if filename is a shared library name (ends with .so or .so.X...)."""
    fname = name.split("/")[-1]
    if fname.endswith((".py", ".pyc", ".la", ".a", ".h", ".c", ".txt", ".cmake")):
        return False
    return bool(re.search(r"\.so(\.\d+)*$", fname))


def load_package_metadata(cache_dir):
    """Download or load cached Packages and Contents-aarch64.gz files."""
    os.makedirs(cache_dir, exist_ok=True)
    pkgs_file = os.path.join(cache_dir, "Packages")
    contents_file = os.path.join(cache_dir, "Contents-aarch64.gz")

    # Check local workspace root as well
    ws_root = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
    ws_pkgs = os.path.join(ws_root, "packages_aarch64")
    ws_contents = os.path.join(ws_root, "Contents-aarch64.gz")

    if not os.path.exists(pkgs_file):
        if os.path.exists(ws_pkgs):
            log("Using repository cached packages_aarch64")
            with open(ws_pkgs, "rb") as sf, open(pkgs_file, "wb") as df:
                df.write(sf.read())
        else:
            log("Downloading Termux Packages index...")
            download_url(PACKAGES_URL, pkgs_file)

    if not os.path.exists(contents_file):
        if os.path.exists(ws_contents):
            log("Using repository cached Contents-aarch64.gz")
            with open(ws_contents, "rb") as sf, open(contents_file, "wb") as df:
                df.write(sf.read())
        else:
            log("Downloading Termux Contents-aarch64.gz index...")
            download_url(CONTENTS_URL, contents_file)

    pkgs = {}
    with open(pkgs_file, "r", encoding="utf-8", errors="ignore") as f:
        for block in f.read().split("\n\n"):
            pkg_name = None
            filename = ""
            deps = ""
            for line in block.strip().split("\n"):
                if line.startswith("Package: "):
                    pkg_name = line.split(": ", 1)[1].strip()
                elif line.startswith("Filename: "):
                    filename = line.split(": ", 1)[1].strip()
                elif line.startswith("Depends: "):
                    deps = line.split(": ", 1)[1].strip()
            if pkg_name and filename:
                pkgs[pkg_name] = {"filename": filename, "deps": deps}

    so_to_pkg = {}
    with gzip.open(contents_file, "rt", encoding="utf-8", errors="ignore") as f:
        for line in f:
            parts = line.strip().split()
            if len(parts) >= 2:
                path, pkg_str = parts[0], parts[1]
                if path.startswith("data/data/com.termux/files/usr/lib/"):
                    rel = path[len("data/data/com.termux/files/usr/lib/") :]
                    if not rel.startswith(("ao/", "aws-cli/")):
                        fname = rel.split("/")[-1]
                        if is_so_name(fname):
                            chosen_pkg = None
                            for p in pkg_str.split(","):
                                p = p.strip()
                                if p in pkgs:
                                    chosen_pkg = p
                                    break
                            if not chosen_pkg:
                                chosen_pkg = pkg_str.split(",")[0].strip()
                            if "/" not in rel or fname not in so_to_pkg:
                                so_to_pkg[fname] = chosen_pkg

    return pkgs, so_to_pkg


def bundle_qemu_and_deps(target_dir, cache_dir=None):
    if not cache_dir:
        # Avoid placing cache inside target_dir so it doesn't get bundled into APK/release
        parent = os.path.dirname(os.path.abspath(target_dir))
        cache_dir = os.path.join(parent, ".termux_cache")
    os.makedirs(target_dir, exist_ok=True)
    os.makedirs(cache_dir, exist_ok=True)

    pkgs, so_to_pkg = load_package_metadata(cache_dir)

    qemu_bin = os.path.join(target_dir, "qemu-system-x86_64")
    have_qemu = os.path.exists(qemu_bin) and os.path.getsize(qemu_bin) > 1000000

    deb_cache = os.path.join(cache_dir, "debs")
    os.makedirs(deb_cache, exist_ok=True)

    def fetch_deb(pkg_name):
        meta = pkgs.get(pkg_name)
        if not meta:
            return None, None
        filename = meta["filename"]
        # Replace colon in filename for Windows compatibility (NTFS stream avoidance)
        safe_name = os.path.basename(filename).replace(":", "_")
        local_deb = os.path.join(deb_cache, safe_name)
        if not os.path.exists(local_deb) or os.path.getsize(local_deb) == 0:
            url = f"{TERMUX_BASE}/{filename.replace(':', '%3a')}"
            download_url(url, local_deb)
        with open(local_deb, "rb") as f:
            deb_bytes = f.read()
        tar_bytes = parse_ar_deb(deb_bytes)
        return pkg_name, tar_bytes

    if not have_qemu:
        log("Downloading and unpacking QEMU headless binary...")
        _, headless_tar = fetch_deb("qemu-system-x86-64-headless")
        with tarfile.open(fileobj=io.BytesIO(headless_tar)) as tar:
            for m in tar.getmembers():
                if m.name.endswith("/qemu-system-x86_64") and m.isfile():
                    data = tar.extractfile(m).read()
                    with open(qemu_bin, "wb") as f:
                        f.write(data)
                    os.chmod(qemu_bin, 0o755)
                    break

    # Always ensure x86 firmware from qemu-common is present
    bios_bin = os.path.join(target_dir, "bios-256k.bin")
    if not os.path.exists(bios_bin):
        log("Downloading and unpacking QEMU common x86 firmware...")
        _, common_tar = fetch_deb("qemu-common")
        with tarfile.open(fileobj=io.BytesIO(common_tar)) as tar:
            for m in tar.getmembers():
                if "/share/qemu/" in m.name and m.isfile() and is_x86_firmware(m.name):
                    rel = m.name.split("/share/qemu/", 1)[1]
                    dst = os.path.join(target_dir, rel)
                    os.makedirs(os.path.dirname(dst), exist_ok=True)
                    with open(dst, "wb") as f:
                        f.write(tar.extractfile(m).read())
                    os.chmod(dst, 0o644)

    # Read initial DT_NEEDED from qemu-system-x86_64
    with open(qemu_bin, "rb") as f:
        initial_needed = set(get_dt_needed(f.read()))
    log(f"Initial DT_NEEDED count: {len(initial_needed)}: {sorted(initial_needed)}")

    available_sos = {}  # soname -> bytes
    all_symlinks = {}   # soname -> target_soname

    # Scan target_dir for any existing valid ELF .so files
    for f in os.listdir(target_dir):
        fp = os.path.join(target_dir, f)
        if os.path.isfile(fp) and ".so" in f:
            try:
                with open(fp, "rb") as ef:
                    content = ef.read()
                if content[:4] == b"\x7fELF":
                    available_sos[f] = content
            except Exception:
                pass

    all_dt_needed = set(initial_needed)
    downloaded_pkgs = set()
    queue = set(initial_needed) - BIONIC_LIBS - set(available_sos.keys())

    # Add direct dependencies of qemu-system-x86-64-headless to kickstart
    qemu_deps = pkgs.get("qemu-system-x86-64-headless", {}).get("deps", "")
    for part in qemu_deps.split(","):
        p = part.strip().split()[0]
        if p and p in pkgs and p not in {"qemu-common", "python", "ca-certificates"}:
            for soname, provider in so_to_pkg.items():
                if provider == p:
                    queue.add(soname)

    while queue:
        # Determine packages to fetch for current queue
        needed_pkgs = set()
        for so in list(queue):
            if so in available_sos or so in BIONIC_LIBS:
                queue.discard(so)
                continue
            pkg = so_to_pkg.get(so)
            if pkg and pkg not in downloaded_pkgs:
                needed_pkgs.add(pkg)

        if not needed_pkgs:
            unresolved = [s for s in queue if s not in available_sos and s not in BIONIC_LIBS]
            if unresolved:
                log(f"Notice: remaining unresolved libraries in queue: {unresolved}")
            break

        log(f"Fetching {len(needed_pkgs)} packages: {sorted(needed_pkgs)[:5]}...")
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as executor:
            results = list(executor.map(fetch_deb, needed_pkgs))

        for pkg_name, tar_bytes in results:
            if not tar_bytes:
                continue
            downloaded_pkgs.add(pkg_name)

            tar_files = {}
            tar_symlinks = {}
            with tarfile.open(fileobj=io.BytesIO(tar_bytes)) as tar:
                for m in tar.getmembers():
                    if "/usr/lib/" in m.name and is_so_name(m.name):
                        fname = m.name.split("/")[-1]
                        if m.isfile():
                            data = tar.extractfile(m).read()
                            if len(data) >= 4 and data[:4] == b"\x7fELF":
                                tar_files[fname] = data
                        elif m.issym():
                            tar_symlinks[fname] = m.linkname.split("/")[-1]

            all_symlinks.update(tar_symlinks)

            # Save regular ELF files
            for fname, content in tar_files.items():
                available_sos[fname] = content
                dst = os.path.join(target_dir, fname)
                with open(dst, "wb") as f:
                    f.write(content)
                os.chmod(dst, 0o755)

                sub_needed = get_dt_needed(content)
                for sn in sub_needed:
                    all_dt_needed.add(sn)
                    if sn not in BIONIC_LIBS and sn not in available_sos:
                        queue.add(sn)

    # Global multi-pass chained symlink dereferencing
    changed = True
    while changed:
        changed = False
        for fname, target in list(all_symlinks.items()):
            if fname not in available_sos and is_so_name(fname):
                curr = target
                seen = {fname}
                while curr in all_symlinks and curr not in seen:
                    seen.add(curr)
                    curr = all_symlinks[curr]
                if curr in available_sos:
                    content = available_sos[curr]
                    if len(content) >= 4 and content[:4] == b"\x7fELF":
                        available_sos[fname] = content
                        dst = os.path.join(target_dir, fname)
                        with open(dst, "wb") as f:
                            f.write(content)
                        os.chmod(dst, 0o755)
                        changed = True

    # Final pass: check for any pseudo-symlinks or text link files in target_dir
    for fname in os.listdir(target_dir):
        fp = os.path.join(target_dir, fname)
        if os.path.isfile(fp) and ".so" in fname and os.path.getsize(fp) in range(1, 512):
            try:
                with open(fp, "rb") as f:
                    data = f.read()
                if data[:4] != b"\x7fELF":
                    tgt = data.decode("utf-8", errors="ignore").strip().split("/")[-1]
                    if tgt in available_sos:
                        with open(fp, "wb") as f:
                            f.write(available_sos[tgt])
                        os.chmod(fp, 0o755)
            except Exception:
                pass

    # Ensure all permissions
    for fname in os.listdir(target_dir):
        fp = os.path.join(target_dir, fname)
        if os.path.isfile(fp):
            if ".so" in fname or fname.startswith("qemu-system-"):
                os.chmod(fp, 0o755)

    # Clean up any misplaced cache inside target_dir
    internal_cache = os.path.join(target_dir, ".termux_cache")
    if os.path.exists(internal_cache):
        shutil.rmtree(internal_cache, ignore_errors=True)

    missing = [s for s in all_dt_needed if s not in BIONIC_LIBS and s not in available_sos]
    log(f"Bundling complete!")
    log(f"Packages downloaded: {len(downloaded_pkgs)}")
    log(f"Total .so libraries bundled: {len([f for f in os.listdir(target_dir) if '.so' in f])}")
    if missing:
        log(f"Warning: missing non-bionic libraries: {missing}")
    else:
        log("Verification: ALL dynamic shared library dependencies are 100% satisfied!")


def main():
    parser = argparse.ArgumentParser(description="Bundle Termux QEMU and shared libraries for Android")
    parser.add_argument(
        "--target-dir",
        default=os.path.abspath(os.path.join(os.path.dirname(__file__), "app/src/main/assets/qemu/bin")),
        help="Target directory for QEMU binary, firmware, and .so libraries",
    )
    parser.add_argument("--cache-dir", default=None, help="Cache directory for downloaded .deb files")
    args = parser.parse_args()

    target_dir = os.path.abspath(args.target_dir)
    log(f"Target directory: {target_dir}")
    bundle_qemu_and_deps(target_dir, args.cache_dir)


if __name__ == "__main__":
    main()
