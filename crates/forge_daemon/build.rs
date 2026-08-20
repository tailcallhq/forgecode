// build.rs — builds the Zig forge-daemon-core static library and links it.
//
// The Zig core lives in ../../forge-daemon/ relative to this crate.  We run
// `zig build` there, then tell Cargo where the .a lives and which linker
// flags to pass.  Only re-runs when Zig source files change.
use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(forge_daemon_zig_core)");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // forge-daemon/ is two levels up from crates/forge_daemon/
    let zig_dir = manifest_dir
        .parent() // crates/
        .and_then(|p| p.parent()) // workspace root
        .map(|p| p.join("forge-daemon"))
        .expect("could not resolve forge-daemon/ path");

    println!("cargo:rerun-if-changed={}", zig_dir.join("src").display());
    println!(
        "cargo:rerun-if-changed={}",
        zig_dir.join("build.zig").display()
    );

    let build_zig_core = env::var("FORGE_DAEMON_BUILD_ZIG_CORE").as_deref() == Ok("1");
    if !build_zig_core {
        println!(
            "cargo:warning=skipping forge-daemon Zig core build; set FORGE_DAEMON_BUILD_ZIG_CORE=1 to enable"
        );
        return;
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" {
        println!(
            "cargo:warning=skipping forge-daemon Zig core build for unsupported target_os={target_os}"
        );
        return;
    }

    println!("cargo:rustc-cfg=forge_daemon_zig_core");

    // Detect target; map Cargo triple → Zig target.
    let zig_target = zig_target_from_cargo();

    // Run: zig build -Dtarget=<zig-target> -Doptimize=ReleaseSafe
    let status = Command::new("zig")
        .current_dir(&zig_dir)
        .args(["build", "-Doptimize=ReleaseSafe"])
        .args(zig_target.iter().flat_map(|t| ["-Dtarget", t.as_str()]))
        .status()
        .expect("zig build failed — is zig installed and in PATH?");

    if !status.success() {
        panic!("zig build exited with non-zero status: {status}");
    }

    // libforge_daemon_core.a lives under zig-out/lib/
    let lib_dir = zig_dir.join("zig-out").join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=forge_daemon_core");

    // On macOS, link Foundation (for kqueue) and libc.
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=Foundation");
    }
}

/// Map CARGO_CFG_TARGET_ARCH + CARGO_CFG_TARGET_OS → Zig cross-target string.
/// Returns None to use Zig's native target detection (most common case).
fn zig_target_from_cargo() -> Option<String> {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let _env_abi = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    // Map only if cross-compiling (host != target).
    let host = env::var("HOST").unwrap_or_default();
    let target = env::var("TARGET").unwrap_or_default();
    if host == target {
        return None; // native build — let zig auto-detect
    }

    let zig_os = match os.as_str() {
        "macos" => "macos",
        "linux" => "linux",
        "windows" => "windows",
        _ => return None,
    };
    let zig_arch = match arch.as_str() {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        _ => return None,
    };
    Some(format!("{zig_arch}-{zig_os}"))
}
