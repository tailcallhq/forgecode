//! CLI entrypoint: dispatches between `serve` (webhook server) and `run`
//! (one-shot CLI mode).

use anyhow::Result;
use std::path::PathBuf;

pub enum Command {
    /// Run the webhook server.
    Serve {
        #[allow(dead_code)]
        bind: String,
        /// Path to the GitHub App private key (PEM).
        #[allow(dead_code)]
        private_key: PathBuf,
        #[allow(dead_code)]
        app_id: u64,
        #[allow(dead_code)]
        installation_id: u64,
        /// Webhook signing secret.
        #[allow(dead_code)]
        webhook_secret: String,
    },
    /// Run the agent on a single request, one-shot.
    Run {
        #[allow(dead_code)]
        repo: String,
        #[allow(dead_code)]
        request: String,
        /// Optional checkout directory (defaults to /tmp/<repo>).
        #[allow(dead_code)]
        checkout_dir: Option<PathBuf>,
    },
}

pub fn parse_args() -> Result<Command> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    parse_command(&args)
}

fn parse_command(args: &[String]) -> Result<Command> {
    let (subcommand, rest) = args
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("usage: helios-bot <serve|run> ..."))?;

    match subcommand.as_str() {
        "serve" => {
            let s = parse_serve(rest)?;
            Ok(Command::Serve {
                bind: s.bind,
                private_key: s.private_key,
                app_id: s.app_id,
                installation_id: s.installation_id,
                webhook_secret: s.webhook_secret,
            })
        }
        "run" => {
            let r = parse_run(rest)?;
            Ok(Command::Run {
                repo: r.repo,
                request: r.request,
                checkout_dir: r.checkout_dir,
            })
        }
        other => anyhow::bail!("unknown subcommand: {other}"),
    }
}

struct ServeArgs {
    bind: String,
    private_key: PathBuf,
    app_id: u64,
    installation_id: u64,
    webhook_secret: String,
}

fn parse_serve(args: &[String]) -> Result<ServeArgs> {
    let mut bind = "0.0.0.0:8080".to_string();
    let mut private_key: Option<PathBuf> = None;
    let mut app_id: u64 = 0;
    let mut installation_id: u64 = 0;
    let mut webhook_secret = String::new();

    let mut i = 0;
    while let Some(flag) = args.get(i) {
        match flag.as_str() {
            "--bind" => {
                bind = args
                    .get(i + 1)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("--bind requires a value"))?;
                i += 2;
            }
            "--private-key" => {
                let v = args
                    .get(i + 1)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("--private-key requires a value"))?;
                private_key = Some(PathBuf::from(v));
                i += 2;
            }
            "--app-id" => {
                app_id = args
                    .get(i + 1)
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| anyhow::anyhow!("--app-id requires a u64"))?;
                i += 2;
            }
            "--installation-id" => {
                installation_id = args
                    .get(i + 1)
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| anyhow::anyhow!("--installation-id requires a u64"))?;
                i += 2;
            }
            "--webhook-secret" => {
                webhook_secret = args
                    .get(i + 1)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("--webhook-secret requires a value"))?;
                i += 2;
            }
            _ => anyhow::bail!("unknown flag: {flag}"),
        }
    }

    Ok(ServeArgs {
        bind,
        private_key: private_key.ok_or_else(|| anyhow::anyhow!("--private-key is required"))?,
        app_id,
        installation_id,
        webhook_secret,
    })
}

struct RunArgs {
    repo: String,
    request: String,
    checkout_dir: Option<PathBuf>,
}

fn parse_run(args: &[String]) -> Result<RunArgs> {
    let mut repo: Option<String> = None;
    let mut request: Option<String> = None;
    let mut checkout_dir: Option<PathBuf> = None;

    let mut i = 0;
    while let Some(flag) = args.get(i) {
        match flag.as_str() {
            "--repo" => {
                repo = Some(
                    args.get(i + 1)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("--repo requires a value"))?,
                );
                i += 2;
            }
            "--request" => {
                request = Some(
                    args.get(i + 1)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("--request requires a value"))?,
                );
                i += 2;
            }
            "--checkout-dir" => {
                let v = args
                    .get(i + 1)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("--checkout-dir requires a value"))?;
                checkout_dir = Some(PathBuf::from(v));
                i += 2;
            }
            _ => anyhow::bail!("unknown flag: {flag}"),
        }
    }

    Ok(RunArgs {
        repo: repo.ok_or_else(|| anyhow::anyhow!("--repo is required"))?,
        request: request.ok_or_else(|| anyhow::anyhow!("--request is required"))?,
        checkout_dir,
    })
}

pub async fn run() -> Result<()> {
    let cmd = parse_args()?;
    match cmd {
        Command::Serve { .. } => {
            // Webhook server is implemented in a separate binary (Cloudflare
            // Worker deployment is recommended). The Rust binary here is a
            // thin stub that prints a helpful message.
            eprintln!("helios-bot serve: webhook server is implemented as a Cloudflare Worker.");
            eprintln!("See .github/apps/helios-bot/README.md for deployment instructions.");
            eprintln!("For local dev, run `wrangler dev` in .github/apps/helios-bot/worker-src/");
            std::process::exit(0);
        }
        Command::Run { repo, request, checkout_dir } => {
            let checkout =
                checkout_dir.unwrap_or_else(|| std::env::temp_dir().join(repo.replace('/', "_")));
            eprintln!(
                "helios-bot run: repo={repo} request={request} checkout={}",
                checkout.display()
            );
            eprintln!(
                "(stub: in production this would clone {repo}, run forge, and post the result back)"
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_run_minimal() {
        let cmd = parse_run(&[
            "--repo".to_string(),
            "KooshaPari/forgecode".to_string(),
            "--request".to_string(),
            "fix typo".to_string(),
        ])
        .unwrap();
        assert_eq!(cmd.repo, "KooshaPari/forgecode");
        assert_eq!(cmd.request, "fix typo");
    }

    #[test]
    fn parse_run_missing_required_fails() {
        let result = parse_run(&["--repo".to_string(), "foo/bar".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_command_rejects_unknown_subcommand_without_indexing() {
        let fixture = ["unknown".to_string()];

        let actual = match parse_command(&fixture) {
            Ok(_) => panic!("unknown subcommand should fail"),
            Err(err) => err.to_string(),
        };
        let expected = "unknown subcommand: unknown";

        assert_eq!(actual, expected);
    }

    #[test]
    fn parse_serve_minimal() {
        let cmd = parse_serve(&[
            "--private-key".to_string(),
            "/tmp/key.pem".to_string(),
            "--app-id".to_string(),
            "12345".to_string(),
            "--installation-id".to_string(),
            "67890".to_string(),
            "--webhook-secret".to_string(),
            "secret".to_string(),
        ])
        .unwrap();
        assert_eq!(cmd.bind, "0.0.0.0:8080");
        assert_eq!(cmd.app_id, 12345);
    }
}
