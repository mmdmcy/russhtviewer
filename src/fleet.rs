use anyhow::{Context, Result, anyhow};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpStream,
    process::Command,
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const DEFAULT_PORT: u16 = 45090;
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
pub const OFFLINE_AFTER: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetDevice {
    pub device_id: String,
    pub display_name: String,
    pub os: String,
    pub shell_url: String,
    pub capabilities: Vec<String>,
    pub last_seen_unix_ms: u128,
    pub online: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub device_id: String,
    pub display_name: String,
    pub os: String,
    pub shell_url: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    pub device_id: String,
    #[serde(default)]
    pub shell_url: Option<String>,
}

#[derive(Debug, Default)]
pub struct FleetRegistry {
    devices: RwLock<HashMap<String, Entry>>,
}

#[derive(Debug, Clone)]
struct Entry {
    device: FleetDevice,
    last_seen: Instant,
}

impl FleetRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, request: RegisterRequest) -> FleetDevice {
        let device = FleetDevice {
            device_id: request.device_id.clone(),
            display_name: request.display_name,
            os: request.os,
            shell_url: normalize_url(&request.shell_url),
            capabilities: if request.capabilities.is_empty() {
                vec!["shell".into()]
            } else {
                request.capabilities
            },
            last_seen_unix_ms: unix_ms(),
            online: true,
        };
        self.devices.write().insert(
            request.device_id,
            Entry {
                device: device.clone(),
                last_seen: Instant::now(),
            },
        );
        device
    }

    pub fn heartbeat(&self, request: HeartbeatRequest) -> Result<FleetDevice> {
        let mut devices = self.devices.write();
        let entry = devices
            .get_mut(&request.device_id)
            .ok_or_else(|| anyhow!("unknown device; register first"))?;
        entry.last_seen = Instant::now();
        entry.device.last_seen_unix_ms = unix_ms();
        entry.device.online = true;
        if let Some(url) = request.shell_url {
            entry.device.shell_url = normalize_url(&url);
        }
        Ok(entry.device.clone())
    }

    pub fn list(&self) -> Vec<FleetDevice> {
        let now = Instant::now();
        let mut devices = self.devices.write();
        let mut listed = Vec::with_capacity(devices.len());
        for entry in devices.values_mut() {
            entry.device.online = now.duration_since(entry.last_seen) < OFFLINE_AFTER;
            listed.push(entry.device.clone());
        }
        listed.sort_by(|a, b| {
            b.online
                .cmp(&a.online)
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
        listed
    }
}

pub fn local_identity(port: u16, device_id: Option<&str>) -> RegisterRequest {
    let hostname = hostname().unwrap_or_else(|| "russht-host".into());
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok();
    let display_name = match username.as_deref() {
        Some(user) => format!("{user}@{hostname}"),
        None => hostname.clone(),
    };
    let device_id = device_id
        .map(str::to_string)
        .unwrap_or_else(|| hostname.to_ascii_lowercase());
    RegisterRequest {
        device_id,
        display_name,
        os: os_label(),
        shell_url: preferred_url(port),
        capabilities: vec!["shell".into()],
    }
}

pub fn preferred_url(port: u16) -> String {
    if let Some(ip) = first_tailscale_ipv4() {
        return format!("http://{ip}:{port}/");
    }
    format!("http://127.0.0.1:{port}/")
}

pub fn first_tailscale_ipv4() -> Option<String> {
    let output = Command::new("tailscale")
        .args(["ip", "-4"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
}

pub fn spawn_self_register(registry: Arc<FleetRegistry>, port: u16, device_id: Option<String>) {
    thread::spawn(move || loop {
        let request = local_identity(port, device_id.as_deref());
        registry.register(request);
        thread::sleep(HEARTBEAT_INTERVAL);
    });
}

pub fn spawn_agent_loop(registry_url: String, token: String, port: u16, device_id: Option<String>) {
    thread::spawn(move || loop {
        let identity = local_identity(port, device_id.as_deref());
        if let Err(err) = register_remote(&registry_url, &token, &identity) {
            tracing::warn!(error = %err, "fleet register failed");
        } else if let Err(err) = heartbeat_remote(
            &registry_url,
            &token,
            &HeartbeatRequest {
                device_id: identity.device_id.clone(),
                shell_url: Some(identity.shell_url.clone()),
            },
        ) {
            tracing::warn!(error = %err, "fleet heartbeat failed");
        }
        thread::sleep(HEARTBEAT_INTERVAL);
    });
}

pub fn fetch_devices(registry_url: &str, token: &str) -> Result<Vec<FleetDevice>> {
    let body = http_json("GET", &join(registry_url, "/v1/fleet/devices"), token, None)?;
    #[derive(Deserialize)]
    struct Payload {
        devices: Vec<FleetDevice>,
    }
    Ok(serde_json::from_str::<Payload>(&body)?.devices)
}

fn register_remote(url: &str, token: &str, request: &RegisterRequest) -> Result<()> {
    let body = serde_json::to_string(request)?;
    let _ = http_json("POST", &join(url, "/v1/fleet/register"), token, Some(&body))?;
    Ok(())
}

fn heartbeat_remote(url: &str, token: &str, request: &HeartbeatRequest) -> Result<()> {
    let body = serde_json::to_string(request)?;
    let _ = http_json("POST", &join(url, "/v1/fleet/heartbeat"), token, Some(&body))?;
    Ok(())
}

fn join(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

fn normalize_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.ends_with('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/")
    }
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn hostname() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
}

fn os_label() -> String {
    if cfg!(target_os = "windows") {
        "Windows".into()
    } else if cfg!(target_os = "macos") {
        "macOS".into()
    } else if cfg!(target_os = "linux") {
        "Linux".into()
    } else {
        std::env::consts::OS.into()
    }
}

fn http_json(method: &str, url: &str, token: &str, body: Option<&str>) -> Result<String> {
    let parsed = parse_http_url(url)?;
    let mut stream = TcpStream::connect((parsed.host.as_str(), parsed.port))
        .with_context(|| format!("connect {}:{}", parsed.host, parsed.port))?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok();
    let payload = body.unwrap_or("").as_bytes();
    let content_type = if body.is_some() {
        "Content-Type: application/json\r\n"
    } else {
        ""
    };
    let host_header = if parsed.port == 80 {
        parsed.host.clone()
    } else {
        format!("{}:{}", parsed.host, parsed.port)
    };
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host_header}\r\nAuthorization: Bearer {token}\r\nAccept: application/json\r\n{content_type}Content-Length: {len}\r\nConnection: close\r\n\r\n",
        path = parsed.path,
        len = payload.len()
    );
    stream.write_all(request.as_bytes())?;
    if !payload.is_empty() {
        stream.write_all(payload)?;
    }
    stream.flush()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let boundary = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .context("invalid HTTP response")?;
    let (head, body) = response.split_at(boundary);
    let head = std::str::from_utf8(head)?;
    let status: u16 = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .context("missing status")?
        .parse()?;
    let text = String::from_utf8_lossy(body).trim().to_string();
    if !(200..300).contains(&status) {
        anyhow::bail!("status {status}: {text}");
    }
    Ok(text)
}

struct ParsedUrl {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_url(url: &str) -> Result<ParsedUrl> {
    let rest = url
        .trim()
        .strip_prefix("http://")
        .ok_or_else(|| anyhow!("only http:// fleet URLs are supported"))?;
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (rest, "/".into()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse()?),
        None => (authority.to_string(), 80),
    };
    Ok(ParsedUrl { host, port, path })
}

#[cfg(test)]
mod tests {
    use super::{FleetRegistry, RegisterRequest};

    #[test]
    fn registry_lists_online_device() {
        let registry = FleetRegistry::new();
        registry.register(RegisterRequest {
            device_id: "workstation".into(),
            display_name: "user@workstation".into(),
            os: "Linux".into(),
            shell_url: "http://100.64.1.10:45090".into(),
            capabilities: vec![],
        });
        let devices = registry.list();
        assert_eq!(devices.len(), 1);
        assert!(devices[0].online);
        assert_eq!(devices[0].shell_url, "http://100.64.1.10:45090/");
    }
}
