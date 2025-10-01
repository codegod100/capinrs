use capnweb_client::{Client as CapnClient, ClientConfig};
use capnweb_core::CapId;
use serde_json::json;
use std::env;

const DEFAULT_CAPN_BACKEND: &str = "http://localhost:8080";
const RPC_PATH: &str = "/rpc/batch";
const CALCULATOR_CAP_ID: u64 = 1;

struct ClientTarget {
    url: String,
    fetch_stats: bool,
}

fn usage() {
    eprintln!(
        "Usage: cargo run --bin client -- [OPTIONS] [HOST_OR_URL]\n\n\
         Options:\n\
             --stats      Fetch durable-object style state after the RPC call\n\
             -h, --help   Show this message\n\
\n\
         Provide an optional host or full URL for the Cap'n Web backend server.\n\
         Examples:\n\
             cargo run --bin client -- localhost:8081\n\
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
    let mut host_arg: Option<String> = None;
    let mut fetch_stats = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                usage();
                std::process::exit(0);
            }
            "--stats" => fetch_stats = true,
            other => {
                if other.starts_with('-') {
                    eprintln!("Warning: unrecognized flag `{}`", other);
                    continue;
                }

                if host_arg.is_none() {
                    host_arg = Some(other.to_string());
                } else {
                    eprintln!("Warning: ignoring extra argument `{}`", other);
                }
            }
        }
    }

    let env_override = env::var("CAPINRS_SERVER_HOST").ok();
    let default_host = DEFAULT_CAPN_BACKEND.to_string();
    let raw_target = host_arg.or(env_override).unwrap_or(default_host);

    let url = normalize_endpoint(&raw_target, "http://");

    ClientTarget { url, fetch_stats }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target = resolve_target();

    let config = ClientConfig {
        url: target.url.clone(),
        ..Default::default()
    };
    let client = CapnClient::new(config)?;
    let result = client
        .call(CapId::new(CALCULATOR_CAP_ID), "add", vec![json!(10), json!(20)])
        .await?;

    println!("Result: {}", result);

    if target.fetch_stats {
        let stats = client
            .call(CapId::new(CALCULATOR_CAP_ID), "stats", Vec::new())
            .await?;

        println!("Durable object stats:");
        let pretty = serde_json::to_string_pretty(&stats).unwrap_or_else(|_| stats.to_string());
        println!("{}", pretty);
    }
    Ok(())
}
