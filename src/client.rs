use capnweb_client::{Client as CapnClient, ClientConfig};
use capnweb_core::CapId;
use serde_json::json;
use std::env;

const DEFAULT_CAPN_BACKEND: &str = "http://localhost:8080";
const DEFAULT_WORKER_BACKEND: &str = "https://capinrs-server.veronika-m-winters.workers.dev";
const RPC_PATH: &str = "/rpc/batch";

#[derive(Debug, Clone, Copy)]
enum BackendMode {
    CapnWeb,
    Worker,
}

struct ClientTarget {
    mode: BackendMode,
    url: String,
}

fn usage() {
    eprintln!(
        "Usage: cargo run --bin client -- [OPTIONS] [HOST_OR_URL]\n\n\
                 Options:\n\
                     --worker     Use the deployed Cloudflare Worker (Cap'n Web over HTTPS)\n\
           --capn       Force Cap'n Web RPC protocol (default)\n\
           -h, --help   Show this message\n\
\n\
         Provide an optional host or full URL for the backend server.\n\
         Examples:\n\
           cargo run --bin client -- localhost:8081\n\
           cargo run --bin client -- --worker https://example.com/api\n\
           cargo run --bin client -- https://api.example.com/rpc/batch\n\
         You can also set CAPINRS_SERVER_HOST in the environment."
    );
}

fn ensure_scheme(raw: &str, fallback: &str) -> String {
    if raw.contains("://") {
        raw.to_string()
    } else {
        format!("{}{}", fallback, raw)
    }
}

fn normalize_endpoint(raw: &str, default_scheme: &str) -> String {
    let with_scheme = ensure_scheme(raw, default_scheme);
    if with_scheme.ends_with(RPC_PATH) {
        with_scheme
    } else {
        format!(
            "{}/{}",
            with_scheme.trim_end_matches('/'),
            RPC_PATH.trim_start_matches('/')
        )
    }
}

fn resolve_target() -> ClientTarget {
    let mut args = env::args().skip(1);
    let mut mode = BackendMode::CapnWeb;
    let mut host_arg: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                usage();
                std::process::exit(0);
            }
            "--worker" => mode = BackendMode::Worker,
            "--capn" | "--capnweb" => mode = BackendMode::CapnWeb,
            other => {
                if host_arg.is_none() {
                    host_arg = Some(other.to_string());
                } else {
                    eprintln!("Warning: ignoring extra argument `{}`", other);
                }
            }
        }
    }

    let env_override = env::var("CAPINRS_SERVER_HOST").ok();
    let default_host = match mode {
        BackendMode::CapnWeb => DEFAULT_CAPN_BACKEND.to_string(),
        BackendMode::Worker => DEFAULT_WORKER_BACKEND.to_string(),
    };
    let raw_target = host_arg.or(env_override).unwrap_or(default_host);

    let scheme = match mode {
        BackendMode::CapnWeb => "http://",
        BackendMode::Worker => "https://",
    };

    let url = normalize_endpoint(&raw_target, scheme);

    ClientTarget { mode, url }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target = resolve_target();

    let config = ClientConfig {
        url: target.url,
        ..Default::default()
    };
    let client = CapnClient::new(config)?;
    let result = client
        .call(CapId::new(1), "add", vec![json!(10), json!(20)])
        .await?;

    println!("Result: {}", result);
    Ok(())
}
