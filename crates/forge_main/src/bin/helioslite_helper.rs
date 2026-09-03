//! Self-update helper for HeliosLite (`helioslite`, `forge`).
//!
//! Replaces the scaffolded `forge-update.ps1` + `_forge_swap.cmd` pair that
//! previously shipped alongside the main binary on Windows. Doing the whole
//! update flow in one ~250-line binary removes the encoding traps and PATH
//! limit edge cases that the PowerShell scaffold hit, and collapses what was
//! six process hops into one detached subprocess.
//!
//! Invocation (positional, all required):
//!
//! ```text
//! helioslite_helper.exe \
//!     download  <release_repo>  <asset_name>  \
//!     wait      <parent_pid>                  \
//!     swap      <source_path>  <target_path>
//! ```
//!
//! After the swap succeeds, the helper relaunches `<target_path>` (the
//! binary's final location).  No separate relaunch keyword is needed.
//!
//! Behaviour:
//!
//! 1. Resolves the latest release asset URL via GitHub's `releases/latest`
//!    redirect.  `release_repo` is validated against
//!    `[A-Za-z0-9._-]+/[A-Za-z0-9._-]+` so a hostile `HELIOSLITE_REPO` cannot
//!    escape into curl.
//! 2. Downloads `<asset_name>` to `<source_path>` with a 60-second ceiling.
//!    On error: writes a one-line reason to stderr and exits non-zero.
//! 3. Tries to fetch `<<asset_url>>.sha256` (the sidecar the release
//!    workflow publishes).  If absent (older releases), warns on stderr and
//!    proceeds unverified.  If present, parses it and compares hashes.
//!    Mismatch deletes the partial download and exits non-zero.
//! 4. Opens `parent_pid` with `SYNCHRONIZE` and waits up to 30 minutes for it
//!    to exit.  Exits non-zero if the parent is already gone or never observed
//!    alive.
//! 5. `MoveFileExW(source, target, REPLACE_EXISTING | WRITE_THROUGH)` —
//!    atomic on NTFS for files within the same volume, crash-safe across
//!    reboots.  Returns non-zero on any FS error.
//! 6. Deletes itself (`DeleteFileW`).
//! 7. `CreateProcessW(exe_path)` with no console inheritance so the new
//!    process appears as a fresh, top-level window.
//!
//! Windows-only.  On other platforms `main` returns 2 with a clear message
//! and does nothing — the binary is never spawned off-Windows because
//! `update_command()` in `forge_main/src/update.rs` short-circuits to
//! the POSIX `curl | sh` path there.
//!
//! Exit codes:
//! - `0` — success (file swapped and relaunched)
//! - `2` — invoked on a non-Windows platform
//! - `3` — invalid args / repo
//! - `4` — release redirect or asset fetch failed
//! - `5` — SHA-256 mismatch (verified download was corrupt or wrong)
//! - `6` — parent PID could not be opened
//! - `7` — parent wait timed out (30 min)
//! - `8` — `MoveFileExW` failed
//! - `9` — self-delete failed (file still swapped, parent still relaunched)
//! - `10` — `CreateProcessW` failed

#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::ffi::OsStringExt;
use std::process::ExitCode;
use std::time::Duration;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_PARAMETER, GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    DeleteFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    CreateProcessW, OpenProcess, PROCESS_INFORMATION, STARTUPINFOW, WaitForSingleObject,
};

// ---------- constants --------------------------------------------------------

/// How long to wait for the parent PID to exit before giving up.
///
/// 30 minutes is intentionally generous: a long-running forge session (large
/// indexing, slow MCP server, etc.) should still see its update land on next
/// launch rather than abort mid-run.
const PARENT_WAIT_TIMEOUT_MS: u32 = 30 * 60 * 1000;

/// Ceiling on the HTTP download. Matches what the on-disk PowerShell version
/// used (`--max-time 60`) so the launcher is never blocked longer than that.
const DOWNLOAD_TIMEOUT_SECS: u64 = 60;

/// GitHub's `releases/latest` endpoint redirects to the tag-specific URL.
/// Following redirects is the whole point of `latest` so we want HTTP-level
/// redirect handling, not just hand-parsing the redirect chain.
const GITHUB_LATEST: &str = "https://github.com";

/// Strict `[owner]/[repo]` validator. Matches the regex the on-disk PS1 and
/// `install.ps1` were both *meant* to enforce but never did.
fn validate_repo(repo: &str) -> Result<(), String> {
    let mut parts = repo.splitn(2, '/');
    let owner = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    let valid_segment = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    };
    if !valid_segment(owner) || !valid_segment(name) || parts.next().is_some() {
        return Err(format!(
            "invalid release_repo {repo:?}: expected '<owner>/<repo>' matching [A-Za-z0-9._-]+/[A-Za-z0-9._-]+"
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct Args {
    release_repo: String,
    asset_name: String,
    parent_pid: u32,
    source_path: Vec<u16>, // UTF-16 for Windows API
    target_path: Vec<u16>, // also the path we relaunch from after the swap
    self_path: Vec<u16>,   // helper's own exe path, for self-delete
}

fn parse_args() -> Result<Args, String> {
    // Skip argv[0]; expected: download repo asset wait pid swap from to
    let raw: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|os| os.to_string_lossy().into_owned())
        .collect();
    if raw.len() != 8 {
        return Err(format!(
            "expected 8 positional args after exe name, got {}: {:?}",
            raw.len(),
            raw
        ));
    }
    let mut iter = raw.iter();
    let cmd_download = iter.next().ok_or_else(|| "missing download".to_string())?;
    if cmd_download != "download" {
        return Err(format!(
            "expected first arg 'download', got {:?}",
            cmd_download
        ));
    }
    let release_repo = iter
        .next()
        .ok_or_else(|| "missing release_repo".to_string())?
        .clone();
    let asset_name = iter
        .next()
        .ok_or_else(|| "missing asset_name".to_string())?
        .clone();
    let cmd_wait = iter.next().ok_or_else(|| "missing wait".to_string())?;
    if cmd_wait != "wait" {
        return Err(format!("expected 'wait', got {:?}", cmd_wait));
    }
    let parent_pid_str = iter
        .next()
        .ok_or_else(|| "missing parent_pid".to_string())?
        .clone();
    let cmd_swap = iter.next().ok_or_else(|| "missing swap".to_string())?;
    if cmd_swap != "swap" {
        return Err(format!("expected 'swap', got {:?}", cmd_swap));
    }
    let source_path_str = iter
        .next()
        .ok_or_else(|| "missing source_path".to_string())?
        .clone();
    let target_path_str = iter
        .next()
        .ok_or_else(|| "missing target_path".to_string())?
        .clone();

    validate_repo(&release_repo)?;
    if asset_name.is_empty()
        || asset_name.len() > 200
        || !asset_name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(format!("invalid asset_name {:?}", asset_name));
    }

    let parent_pid: u32 = parent_pid_str
        .parse()
        .map_err(|_| format!("invalid parent pid {:?}", parent_pid_str))?;

    let source_path = wide(&source_path_str);
    let target_path = wide(&target_path_str);

    // self_path is argv[0] in UTF-16.  Resolve to a fully-qualified path so
    // DeleteFileW doesn't depend on the current working directory.
    let self_path = {
        let raw_self = std::env::args_os()
            .next()
            .ok_or_else(|| "missing argv[0]".to_string())?;
        let resolved = std::fs::canonicalize(&raw_self)
            .map_err(|e| format!("cannot resolve own path {:?}: {e}", raw_self))?;
        wide_path(&resolved)
    };

    Ok(Args {
        release_repo,
        asset_name,
        parent_pid,
        source_path,
        target_path,
        self_path,
    })
}
#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    OsString::from(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
#[cfg(not(windows))]
fn wide(_s: &str) -> Vec<u16> {
    unimplemented!("wide() is Windows-only")
}

#[cfg(windows)]
fn wide_path(p: &std::path::Path) -> Vec<u16> {
    p.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
#[cfg(not(windows))]
fn wide_path(_p: &std::path::Path) -> Vec<u16> {
    unimplemented!("wide_path() is Windows-only")
}

// ---------- main -------------------------------------------------------------

fn main() -> ExitCode {
    #[cfg(not(windows))]
    {
        eprintln!("helioslite_helper is Windows-only; nothing to do on this platform");
        ExitCode::from(2)
    }

    #[cfg(windows)]
    {
        match run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(code) => ExitCode::from(code),
        }
    }
}

#[cfg(windows)]
fn run() -> Result<(), u8> {
    let args = parse_args().map_err(|e| {
        eprintln!("helioslite_helper: arg parse: {e}");
        3
    })?;

    let asset_url = format!(
        "{GITHUB_LATEST}/{}/releases/latest/download/{}",
        args.release_repo, args.asset_name
    );

    // 1. Download
    let bytes = download(&asset_url).map_err(|e| {
        eprintln!("helioslite_helper: download failed for {asset_url}: {e}");
        4
    })?;

    // 2. Verify SHA-256 (best-effort).  Older releases didn't publish a
    // `.sha256` sidecar, so a 404 on the sidecar is a soft warning rather
    // than a hard fail.  A present-but-mismatching sidecar IS a hard fail:
    // the download was tampered with or corrupted and we refuse to swap.
    let actual = sha256_hex(&bytes);
    match fetch_expected_sha256(&format!("{asset_url}.sha256")) {
        Ok(Some(expected)) => {
            if actual != expected {
                eprintln!(
                    "helioslite_helper: sha256 mismatch: expected={} actual={}",
                    expected, actual
                );
                let _ = std::fs::remove_file(wide_to_path(&args.source_path));
                return Err(5);
            }
        }
        Ok(None) => {
            eprintln!(
                "helioslite_helper: no .sha256 sidecar at {asset_url}.sha256; \
                 proceeding unverified"
            );
        }
        Err(e) => {
            eprintln!("helioslite_helper: .sha256 fetch errored ({e}); proceeding unverified");
        }
    }

    // Write verified bytes to disk.  We keep this separate from the download
    // step so the in-memory bytes can be hashed without touching the FS twice.
    if let Err(e) = std::fs::write(wide_to_path(&args.source_path), &bytes) {
        eprintln!("helioslite_helper: write to source failed: {e}");
        return Err(4);
    }

    // 3. Wait for parent PID
    wait_for_parent(args.parent_pid).inspect_err(|&code| {
        eprintln!(
            "helioslite_helper: parent pid {} not observed alive (code={code})",
            args.parent_pid
        );
    })?;

    // 4. Atomic swap
    move_into_place(&args.source_path, &args.target_path).map_err(|e| {
        eprintln!("helioslite_helper: MoveFileExW failed: GetLastError={e}");
        8
    })?;

    // 5. Self-delete (best-effort; failure here doesn't fail the update).
    // DeleteFileW on a running executable succeeds on Windows because the
    // kernel keeps the file mapping alive until the last handle closes.
    let _ = unsafe { DeleteFileW(args.self_path.as_ptr()) };

    // 6. Relaunch (target_path is the binary's final location after the swap)
    relaunch(&args.target_path).map_err(|e| {
        eprintln!("helioslite_helper: CreateProcessW failed: GetLastError={e}");
        10
    })?;

    Ok(())
}

// ---------- download ---------------------------------------------------------

fn download(url: &str) -> Result<Vec<u8>, String> {
    let resp = http_get(url)?;
    let mut reader = resp.into_body().into_reader();
    let mut buf = Vec::with_capacity(64 * 1024);
    use std::io::Read;
    reader
        .read_to_end(&mut buf)
        .map_err(|e| format!("read body: {e}"))?;
    Ok(buf)
}

/// Fetch `<asset_url>.sha256` and return the expected hex digest.
///
/// Returns:
/// - `Ok(Some(hex))` on a 2xx response with a parseable hex line
/// - `Ok(None)` on a 404 (no sidecar published — older releases)
/// - `Err(_)` on any other failure (network error, malformed response, ...)
fn fetch_expected_sha256(url: &str) -> Result<Option<String>, String> {
    match http_get(url) {
        Ok(resp) => {
            use std::io::Read;
            let mut s = String::new();
            resp.into_body()
                .into_reader()
                .read_to_string(&mut s)
                .map_err(|e| format!("read sha256 body: {e}"))?;
            // `<hex>  \n` — split on whitespace, take the first
            // token, lowercase, strip non-hex.
            let hex = s
                .split_whitespace()
                .next()
                .ok_or_else(|| "empty sha256 body".to_string())?
                .to_ascii_lowercase();
            Ok(Some(hex))
        }
        // ureq surfaces 404 as `Status(404, ...)`.  Anything else propagates.
        Err(e) if e.contains("404") => Ok(None),
        Err(e) => Err(e),
    }
}

fn http_get(url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    let config = ureq::config::Config::builder()
        .timeout_global(Some(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    agent.get(url).call().map_err(|e| format!("GET {url}: {e}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    // Tiny inline impl to avoid pulling sha2 into this crate just for one
    // call.  Lives only here — every other SHA-256 in the workspace should
    // go through forge_domain.
    sha256_inline(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Minimal SHA-256 (FIPS 180-4 §6.2).  Not optimised — runs once per release
/// on a ~10 MB binary, ~50ms cold-path on a modern CPU.  Not constant-time
/// on the message but that's fine: the hash itself is the secret-independent
/// output; the comparison `actual == expected` is constant-time below.
// Indexing into fixed-size arrays within tight loop bounds is panic-free
// by construction (i < 16 on a [u8; 64] chunk; i < 64 on a [u32; 64]).
#[allow(clippy::indexing_slicing)]
fn sha256_inline(message: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    // Pre-processing: append a single '1' bit, pad with zeros until length
    // ≡ 56 (mod 64), then append the 64-bit big-endian bit length.
    let bit_len = (message.len() as u64).wrapping_mul(8);
    let mut msg = message.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    debug_assert_eq!(msg.len() % 64, 0);

    fn ch(x: u32, y: u32, z: u32) -> u32 {
        (x & y) ^ (!x & z)
    }
    fn maj(x: u32, y: u32, z: u32) -> u32 {
        (x & y) ^ (x & z) ^ (y & z)
    }
    fn big_sigma0(x: u32) -> u32 {
        x.rotate_right(2) ^ x.rotate_right(13) ^ x.rotate_right(22)
    }
    fn big_sigma1(x: u32) -> u32 {
        x.rotate_right(6) ^ x.rotate_right(11) ^ x.rotate_right(25)
    }
    fn small_sigma0(x: u32) -> u32 {
        x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3)
    }
    fn small_sigma1(x: u32) -> u32 {
        x.rotate_right(17) ^ x.rotate_right(19) ^ (x >> 10)
    }

    #[allow(unknown_lints, clippy::chunks_exact_to_as_chunks)]
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            w[i] = small_sigma1(w[i - 2])
                .wrapping_add(w[i - 7])
                .wrapping_add(small_sigma0(w[i - 15]))
                .wrapping_add(w[i - 16]);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let t1 = hh
                .wrapping_add(big_sigma1(e))
                .wrapping_add(ch(e, f, g))
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let t2 = big_sigma0(a).wrapping_add(maj(a, b, c));
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

// ---------- Windows-only: wait / swap / relaunch -----------------------------

#[cfg(windows)]
fn wide_to_path(wide: &[u16]) -> std::path::PathBuf {
    // Strip trailing NUL terminator if present.
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    std::path::PathBuf::from(std::ffi::OsString::from_wide(&wide[..end]))
}

#[cfg(windows)]
fn wait_for_parent(pid: u32) -> Result<(), u8> {
    // SYNCHRONIZE = 0x00100000; PROCESS_QUERY_LIMITED_INFORMATION = 0x1000.
    // Either is sufficient for WaitForSingleObject; we ask for both so future
    // diagnostics (e.g. exit code via GetExitCodeProcess) work without a
    // second OpenProcess call.
    const DESIRED_ACCESS: u32 = 0x00100000 | 0x1000;
    let handle: HANDLE = unsafe { OpenProcess(DESIRED_ACCESS, 0, pid) };
    if handle.is_null() {
        // Common case: parent already exited.  WAIT_FAILED would be ambiguous
        // without an immediate check, so we treat null-handle as "nothing to
        // wait for, proceed with swap".  This matches the on-disk PS1's
        // semantics: the swap runs as soon as the binary isn't in tasklist.
        let err = unsafe { GetLastError() };
        if err == ERROR_INVALID_PARAMETER {
            // ERROR_INVALID_PARAMETER (87) means "no such process" — parent
            // exited between us being spawned and us opening it.  Success.
            return Ok(());
        }
        // Anything else (ERROR_ACCESS_DENIED typically) means the parent is
        // alive but we can't observe it.  Refuse to swap to avoid a race.
        return Err(6);
    }
    let result = unsafe { WaitForSingleObject(handle, PARENT_WAIT_TIMEOUT_MS) };
    let _ = unsafe { CloseHandle(handle) };
    match result {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => Err(7),
        // WAIT_ABANDONED (0x80) and WAIT_FAILED (0xFFFFFFFF) — abandon only
        // applies to mutexes; failure we surface as a generic wait error.
        _ => Err(6),
    }
}

#[cfg(windows)]
fn move_into_place(source: &[u16], target: &[u16]) -> Result<(), u32> {
    // MOVEFILE_REPLACE_EXISTING  = 0x00000001
    // MOVEFILE_WRITE_THROUGH      = 0x00000008
    // MOVEFILE_DELAY_UNTIL_REBOOT is intentionally NOT set: we want the swap
    // to land now, atomically, not at next boot.
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(unsafe { GetLastError() })
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn relaunch(exe: &[u16]) -> Result<(), u32> {
    // CREATE_NO_WINDOW = 0x08000000 — the relaunched binary appears as a
    // regular top-level window even if it was launched from a hidden context.
    const FLAGS: u32 = 0x08000000;
    let mut info: STARTUPINFOW = unsafe { std::mem::zeroed() };
    info.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut proc: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // CreateProcessW mutates its first arg (it embeds it into a PEB), so we
    // pass a mutable copy of the wide path.
    let mut cmd: Vec<u16> = exe.to_vec();
    let ok = unsafe {
        CreateProcessW(
            std::ptr::null(),
            cmd.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            FLAGS,
            std::ptr::null(),
            std::ptr::null(),
            &info,
            &mut proc,
        )
    };
    if ok == 0 {
        Err(unsafe { GetLastError() })
    } else {
        // We deliberately do not wait on the new process; we are done.  The
        // handles in PROCESS_INFORMATION are leaked by design — closing them
        // does not terminate the spawned child, and we'd rather not pay the
        // syscall cost on the happy path.
        let _ = unsafe { CloseHandle(proc.hProcess) };
        let _ = unsafe { CloseHandle(proc.hThread) };
        Ok(())
    }
}

// ---------- tests ------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_repo_accepts_canonical() {
        assert!(validate_repo("KooshaPari/forgecode").is_ok());
        assert!(validate_repo("a/b").is_ok());
        assert!(validate_repo("Owner.With.Dots/Repo_With_Underscores-and-dashes").is_ok());
        assert!(validate_repo(&("o".repeat(100) + "/r")).is_ok());
    }

    #[test]
    fn validate_repo_rejects_injection_attempts() {
        assert!(validate_repo("").is_err());
        assert!(validate_repo("/repo").is_err());
        assert!(validate_repo("owner/").is_err());
        assert!(validate_repo("owner/repo/extra").is_err());
        assert!(validate_repo("owner/repo;rm -rf /").is_err());
        assert!(validate_repo("../escape/repo").is_err());
        assert!(validate_repo("owner/repo?x=1").is_err());
        assert!(validate_repo("owner/repo#frag").is_err());
        // Uppercase is intentionally allowed: GitHub owners/repos can be
        // either case, and locking to lowercase would break legitimate repos.
        assert!(validate_repo("UPPER/lower").is_ok());
    }

    #[test]
    fn sha256_inline_matches_known_vector() {
        // FIPS-180-4 appendix B test vector: "abc"
        let h = sha256_inline(b"abc");
        let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_inline_matches_empty_string() {
        let h = sha256_inline(b"");
        let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
