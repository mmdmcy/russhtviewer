mod auth;
mod fleet;
mod pty;
mod server;
mod tui;

use anyhow::{Context, Result};
use fleet::{DEFAULT_PORT, FleetRegistry, fetch_devices, preferred_url};
use server::{AppState, Role};
use std::{
    collections::HashMap,
    env, fs,
    io::ErrorKind,
    sync::Arc,
    thread,
    time::Duration,
};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    init_logging();
    let env_map = load_dotenv()?;
    let token = env_value(&env_map, "RUSSH_MASTERDALE_TOKEN")
        .or_else(|| env_value(&env_map, "DALE_TOKEN"));
    let fleet_host = env_value(&env_map, "RUSSH_FLEET_HOST");
    let device_id = env_value(&env_map, "RUSSH_DEVICE_ID");
    let port = env_value(&env_map, "RUSSH_PORT")
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let cli = parse_cli(fleet_host.clone())?;
    match cli.command {
        Command::Help => {
            print_help();
            Ok(())
        }
        Command::Devices { host } => {
            let token = token.context("DALE_TOKEN or RUSSH_MASTERDALE_TOKEN required")?;
            let host = host.context("pass --host or set RUSSH_FLEET_HOST")?;
            let devices = fetch_devices(&host, &token)?;
            println!("{:<16} {:<8} {:<12} URL", "DEVICE", "ONLINE", "OS");
            for device in devices {
                println!(
                    "{:<16} {:<8} {:<12} {}",
                    device.device_id,
                    if device.online { "yes" } else { "no" },
                    device.os,
                    device.shell_url
                );
            }
            Ok(())
        }
        Command::Connect { device, host } => {
            let token = token.context("DALE_TOKEN or RUSSH_MASTERDALE_TOKEN required")?;
            let host = host.context("pass --host or set RUSSH_FLEET_HOST")?;
            let devices = fetch_devices(&host, &token)?;
            let found = devices
                .into_iter()
                .find(|entry| {
                    entry.device_id.eq_ignore_ascii_case(&device)
                        || entry.display_name.eq_ignore_ascii_case(&device)
                })
                .with_context(|| format!("device {device:?} not found"))?;
            println!("{}", found.shell_url);
            println!("Open that URL in a browser and paste the Masterdale token, or:");
            println!(
                "  websocat -H='Authorization: Bearer $DALE_TOKEN' '{}v1/pty'",
                found.shell_url
            );
            Ok(())
        }
        Command::Status => {
            println!("port={port}");
            println!("preferred_url={}", preferred_url(port));
            println!(
                "masterdale_token={}",
                if token.is_some() {
                    "configured"
                } else {
                    "missing"
                }
            );
            if let Some(host) = fleet_host {
                println!("fleet_host={host}");
            }
            Ok(())
        }
        Command::Tui => {
            let token = token.context("DALE_TOKEN or RUSSH_MASTERDALE_TOKEN required")?;
            let host = fleet_host
                .or_else(|| Some(format!("http://127.0.0.1:{port}")))
                .unwrap();
            tui::run(&host, &token)
        }
        Command::Host { headless } | Command::Agent { headless, .. } => {
            let token = token.context("DALE_TOKEN or RUSSH_MASTERDALE_TOKEN required")?;
            if token.chars().count() < 16 {
                anyhow::bail!("token must be at least 16 characters");
            }
            let role = match &cli.command {
                Command::Host { .. } => Role::Host,
                Command::Agent { .. } => Role::Agent,
                _ => unreachable!(),
            };
            let fleet = matches!(role, Role::Host).then(|| Arc::new(FleetRegistry::new()));
            let state = AppState {
                token_hash: Some(auth::hash_token(&token)),
                port,
                role,
                fleet: fleet.clone(),
                device_id: device_id.clone(),
            };
            server::spawn_server(state);

            if let Some(registry) = fleet {
                fleet::spawn_self_register(registry, port, device_id.clone());
            }
            if let Command::Agent { host, .. } = &cli.command {
                fleet::spawn_agent_loop(host.clone(), token.clone(), port, device_id.clone());
            }

            if !headless && matches!(role, Role::Host) {
                let host = format!("http://127.0.0.1:{port}");
                thread::sleep(Duration::from_millis(300));
                tui::run(&host, &token)
            } else {
                tracing::info!(
                    role = ?role,
                    port,
                    url = %preferred_url(port),
                    "russhtviewer runtime active"
                );
                loop {
                    thread::park_timeout(Duration::from_secs(3600));
                }
            }
        }
    }
}

#[derive(Debug)]
enum Command {
    Help,
    Host { headless: bool },
    Agent { host: String, headless: bool },
    Tui,
    Devices { host: Option<String> },
    Connect { device: String, host: Option<String> },
    Status,
}

struct Cli {
    command: Command,
}

fn parse_cli(default_host: Option<String>) -> Result<Cli> {
    let mut args = env::args().skip(1).peekable();
    let mut headless = false;
    let mut host = default_host;
    let mut command = None;
    let mut positional = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => command = Some(Command::Help),
            "--headless" => headless = true,
            "--host" => {
                host = Some(args.next().context("--host requires a URL")?);
            }
            "host" if command.is_none() => command = Some(Command::Host { headless: false }),
            "agent" if command.is_none() => {
                command = Some(Command::Agent {
                    host: String::new(),
                    headless: true,
                });
            }
            "devices" if command.is_none() => {
                command = Some(Command::Devices { host: None });
            }
            "connect" if command.is_none() => {
                command = Some(Command::Connect {
                    device: String::new(),
                    host: None,
                });
            }
            "status" if command.is_none() => command = Some(Command::Status),
            other if other.starts_with('-') => anyhow::bail!("unknown argument: {other}"),
            other => positional.push(other.to_string()),
        }
    }

    let mut command = command.unwrap_or(Command::Tui);
    match &mut command {
        Command::Host {
            headless: host_headless,
        } => *host_headless = headless,
        Command::Agent {
            host: agent_host,
            headless: agent_headless,
        } => {
            *agent_headless = true;
            *agent_host = host
                .clone()
                .context("agent requires --host URL or RUSSH_FLEET_HOST")?;
            if !agent_host.starts_with("http://") {
                *agent_host = format!("http://{agent_host}");
            }
        }
        Command::Devices {
            host: devices_host,
        } => {
            *devices_host = host.clone();
        }
        Command::Connect {
            device,
            host: connect_host,
        } => {
            *device = positional
                .first()
                .cloned()
                .context("connect requires a device id")?;
            positional.clear();
            *connect_host = host.clone();
        }
        _ => {}
    }

    if !positional.is_empty() {
        anyhow::bail!("unexpected arguments: {}", positional.join(" "));
    }

    Ok(Cli { command })
}

fn print_help() {
    println!(
        "\
Russht Viewer

Usage:
  russhtviewer host [--headless]
  russhtviewer agent --host URL
  russhtviewer
  russhtviewer devices [--host URL]
  russhtviewer connect <device> [--host URL]
  russhtviewer status

Auth:
  Set DALE_TOKEN or RUSSH_MASTERDALE_TOKEN (same Masterdale fleet token).
  Optional RUSSH_FLEET_HOST / RUSSH_PORT / RUSSH_DEVICE_ID.
"
    );
}

fn load_dotenv() -> Result<HashMap<String, String>> {
    for path in [".env", ".private/russhtviewer.env"] {
        match fs::read_to_string(path) {
            Ok(content) => return parse_dotenv(&content),
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(err) => return Err(err).with_context(|| format!("read {path}")),
        }
    }
    Ok(HashMap::new())
}

fn parse_dotenv(content: &str) -> Result<HashMap<String, String>> {
    let mut values = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'').to_string();
        values.insert(key.trim().to_string(), value);
    }
    Ok(values)
}

fn env_value(values: &HashMap<String, String>, key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .or_else(|| values.get(key).cloned())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}
