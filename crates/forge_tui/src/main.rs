#[cfg(unix)]
use std::{
    io::{BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::{Duration, Instant},
};

#[cfg(unix)]
use serde_json::Value;

#[cfg(unix)]
fn socket_path() -> PathBuf {
    if let Ok(s) = std::env::var("FORGE3D_SOCKET") {
        return PathBuf::from(s);
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(runtime).join("forge3/daemon.sock")
}

#[cfg(unix)]
fn send_request(stream: &mut UnixStream, req: &[u8]) -> Result<String, String> {
    let len = req.len() as u32;
    let header = len.to_be_bytes();
    stream
        .write_all(&header)
        .map_err(|e| format!("write: {e}"))?;
    stream.write_all(req).map_err(|e| format!("write: {e}"))?;

    let mut reader = BufReader::new(stream.try_clone().map_err(|e| format!("clone: {e}"))?);
    let mut resp_header = [0u8; 4];
    reader
        .read_exact(&mut resp_header)
        .map_err(|e| format!("read header: {e}"))?;
    let resp_len = u32::from_be_bytes(resp_header) as usize;
    let mut buf = vec![0u8; resp_len];
    reader
        .read_exact(&mut buf)
        .map_err(|e| format!("read body: {e}"))?;
    String::from_utf8(buf).map_err(|e| format!("utf8: {e}"))
}

#[cfg(unix)]
fn rpc_call(method: &str, params: Value) -> Result<String, String> {
    let mut stream = UnixStream::connect(socket_path()).map_err(|e| format!("connect: {e}"))?;
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    });
    let body = serde_json::to_vec(&req).map_err(|e| format!("serialize: {e}"))?;
    send_request(&mut stream, &body)
}

#[cfg(unix)]
fn print_dashboard() {
    // Fetch agent list
    let resp = match rpc_call("agent.list_active", serde_json::json!({})) {
        Ok(r) => r,
        Err(e) => {
            println!("⚠ forge3d unreachable: {e}");
            return;
        }
    };

    let v: Value = match serde_json::from_str(&resp) {
        Ok(v) => v,
        Err(_) => {
            println!("⚠ bad response: {resp:.100}");
            return;
        }
    };

    // Clear screen (ANSI: home + erase below)
    print!("\x1B[H\x1B[J");

    // Header
    println!("\x1B[1mforge3d \u{2014} Agent Dashboard\x1B[0m");
    println!("\u{250C}\u{2500}{}\u{2500}\u{2510}", "\u{2500}".repeat(58),);

    // agents line
    if let Some(agents) = v
        .get("result")
        .and_then(|r| r.get("agents"))
        .and_then(|a| a.as_array())
    {
        if agents.is_empty() {
            println!("\u{2502}  \u{2139} No registered agents                         \u{2502}");
        } else {
            println!(
                "\u{2502}  \u{2022} {} agent(s) registered{:>33}  \u{2502}",
                agents.len(),
                ""
            );
            println!(
                "\u{2502}  \u{2500}{}\u{2500}  \u{2502}",
                "\u{2500}".repeat(44)
            );
            for agent in agents {
                let id = agent
                    .get("agent_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let pid = agent.get("pid").and_then(|v| v.as_u64()).unwrap_or(0);
                let lane = agent.get("lane").and_then(|v| v.as_str()).unwrap_or("?");
                println!(
                    "\u{2502}   {:<18} pid={:<8} lane={:<12}  \u{2502}",
                    id, pid, lane
                );
            }
        }
    } else {
        // Error case — show the raw error
        if let Some(err) = v.get("error") {
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            println!("\u{2502}  \u{26A0} RPC error: {:<38} \u{2502}", msg);
        }
    }

    println!("\u{2514}{}\u{2518}", "\u{2500}".repeat(58));

    // Status line
    println!("  |  socket: {}", socket_path().display());
}

#[cfg(unix)]
fn main() {
    let start = Instant::now();
    println!("forge_tui — polling every 2s. Ctrl+C to exit.");
    loop {
        print_dashboard();
        print!("\rup: {:.0}s started", start.elapsed().as_secs_f64());
        std::io::stdout().flush().ok();
        std::thread::sleep(Duration::from_secs(2));
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!(
        "forge_tui requires Unix domain socket support and is not available on this platform"
    );
    std::process::exit(1);
}
