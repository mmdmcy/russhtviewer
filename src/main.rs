//! Russht Viewer: click between shells on your own devices.
//!
//! A tmux window with your devices in a sidebar on the left and the shell
//! of the one you picked on the right. Click another device and the right
//! side switches to it; sessions keep running on each device, so switching
//! back is instant. No agent, no token, no open port: it opens plain SSH,
//! through LinuxMice (`lmice open DEVICE`) when that is installed, else
//! through the hosts in `~/.ssh/config`.
//!
//! Needs tmux. Runs wherever tmux does: Linux, macOS, WSL on Windows.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const SESSION: &str = "russh";
const SIDEBAR_WIDTH: u16 = 24;
/// Lines above the first device in the sidebar.
const HEADER: usize = 2;
/// How often reachability is checked again.
const REFRESH: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq)]
struct Device {
    name: String,
    /// `online`, `offline` or `unknown`.
    state: String,
}

/// Where devices and connections come from.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    /// LinuxMice's fleet: names, reachability, keys and jumps.
    Lmice,
    /// Plain `Host` entries in `~/.ssh/config`.
    SshConfig,
}

fn source() -> Source {
    let lmice = Command::new("lmice")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if lmice { Source::Lmice } else { Source::SshConfig }
}

/// The string after `"key":` in a JSON object; enough for lmice's fleet
/// status without a JSON library (names and states hold no quotes).
fn json_string<'a>(object: &'a str, key: &str) -> Option<&'a str> {
    let pattern = format!("\"{key}\"");
    let start = object.find(&pattern)? + pattern.len();
    let rest = object[start..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    Some(&rest[..rest.find('"')?])
}

/// Devices from `lmice --format json fleet status`.
fn parse_fleet(json: &str) -> Vec<Device> {
    let Some(at) = json.find("\"devices\"") else {
        return Vec::new();
    };
    json[at..]
        .split('{')
        .skip(1)
        .filter_map(|object| {
            let name = json_string(object, "id")?;
            let state = match json_string(object, "state").unwrap_or("") {
                "online" => "online",
                "" => "unknown",
                _ => "offline",
            };
            plain(name).then(|| Device {
                name: name.to_string(),
                state: state.to_string(),
            })
        })
        .collect()
}

/// `Host` names in an ssh config, without patterns.
fn parse_ssh_config(text: &str) -> Vec<Device> {
    let mut devices: Vec<Device> = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        if !words.next().is_some_and(|word| word.eq_ignore_ascii_case("host")) {
            continue;
        }
        for name in words {
            if plain(name) && !devices.iter().any(|device| device.name == name) {
                devices.push(Device {
                    name: name.to_string(),
                    state: "unknown".to_string(),
                });
            }
        }
    }
    devices
}

fn devices(source: Source) -> Vec<Device> {
    match source {
        Source::Lmice => Command::new("lmice")
            .args(["--format", "json", "fleet", "status"])
            .stderr(Stdio::null())
            .output()
            .map(|output| parse_fleet(&String::from_utf8_lossy(&output.stdout)))
            .unwrap_or_default(),
        Source::SshConfig => std::env::var_os("HOME")
            .map(|home| std::path::PathBuf::from(home).join(".ssh/config"))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| parse_ssh_config(&text))
            .unwrap_or_default(),
    }
}

/// A device name that is safe in a command line and cannot read as a flag.
fn plain(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.@".contains(&byte))
}

fn tmux(args: &[&str]) -> Option<String> {
    let output = Command::new("tmux")
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn target() -> String {
    format!("={SESSION}:")
}

fn option(name: &str) -> String {
    tmux(&["show-options", "-qv", "-t", &target(), name]).unwrap_or_default()
}

fn set_option(name: &str, value: &str) {
    let _ = tmux(&["set-option", "-t", &target(), name, value]);
}

fn this_program() -> String {
    std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "russhtviewer".to_string())
}

/// Single quotes for tmux's shell commands.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Builds the viewer session: sidebar left, shell right.
fn build() -> Result<(), String> {
    let me = quote(&this_program());
    let view = tmux(&[
        "new-session", "-d", "-s", SESSION, "-x", "200", "-y", "50", "-P", "-F", "#{pane_id}",
        &format!("{me} __welcome"),
    ])
    .ok_or("could not start a tmux session")?;
    let width = SIDEBAR_WIDTH.to_string();
    let sidebar = tmux(&[
        "split-window", "-h", "-b", "-d", "-P", "-F", "#{pane_id}", "-l", &width, "-t", &view,
        &format!("{me} __sidebar"),
    ])
    .ok_or("could not split the tmux window")?;
    set_option("@russh_view", &view);
    let _ = tmux(&["set-option", "-p", "-t", &sidebar, "@russh_role", "sidebar"]);
    // Every key goes to the shell on the right, so a remote tmux (or
    // Mobailmux) keeps its own Ctrl-b; only mouse clicks are ours.
    for (name, value) in [
        ("prefix", "None"),
        ("prefix2", "None"),
        ("key-table", "russh"),
        ("status", "off"),
        ("mouse", "on"),
        ("pane-border-lines", "single"),
    ] {
        set_option(name, value);
    }
    let _ = tmux(&[
        "set-hook", "-t", &target(), "window-resized",
        &format!("resize-pane -t {sidebar} -x {width}"),
    ]);
    let click = format!("run-shell -b \"{me} __click '#{{mouse_y}}'\"");
    let sidebar_only = "#{==:#{@russh_role},sidebar}";
    let _ = tmux(&[
        "bind-key", "-T", "russh", "MouseDown1Pane", "if-shell", "-F", sidebar_only, &click,
        "select-pane -t = ; send-keys -M",
    ]);
    for event in ["DoubleClick1Pane", "TripleClick1Pane"] {
        let _ = tmux(&[
            "bind-key", "-T", "russh", event, "if-shell", "-F", sidebar_only, &click, "send-keys -M",
        ]);
    }
    let _ = tmux(&["select-pane", "-t", &view]);
    Ok(())
}

/// Opens the viewer, or brings back the one already running.
fn open() -> i32 {
    if tmux(&["-V"]).is_none() {
        eprintln!("russhtviewer needs tmux (apt install tmux, brew install tmux)");
        return 1;
    }
    if tmux(&["has-session", "-t", &format!("={SESSION}")]).is_none() {
        if let Err(error) = build() {
            eprintln!("{error}");
            return 1;
        }
    }
    let verb = if std::env::var_os("TMUX").is_some() { "switch-client" } else { "attach-session" };
    let ok = Command::new("tmux")
        .args([verb, "-t", &format!("={SESSION}")])
        .status()
        .is_ok_and(|status| status.success());
    i32::from(!ok)
}

/// Switches the right side to `device`.
fn select(device: &str) -> i32 {
    if !plain(device) {
        return 2;
    }
    let view = option("@russh_view");
    if view.is_empty() {
        eprintln!("the viewer is not running; start it with: russhtviewer");
        return 1;
    }
    set_option("@russh_current", device);
    let command = format!("{} __connect {}", quote(&this_program()), quote(device));
    let _ = tmux(&["respawn-pane", "-k", "-t", &view, &command]);
    let _ = tmux(&["select-pane", "-t", &view]);
    0
}

/// A shell on `device` in this terminal; says so when it ends.
fn connect(device: &str, in_viewer: bool) -> i32 {
    if !plain(device) {
        eprintln!("not a device name: {device}");
        return 2;
    }
    let status = if device == "here" {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        Command::new(shell).arg("-l").status()
    } else if source() == Source::Lmice {
        Command::new("lmice").args(["open", device]).status()
    } else {
        Command::new("ssh").args(["-t", device]).status()
    };
    let code = status.map(|status| status.code().unwrap_or(1)).unwrap_or(127);
    if in_viewer {
        println!(
            "\n[{device}: disconnected{}] click a device on the left",
            if code == 0 { "" } else { " with an error" }
        );
        let _ = std::io::stdin().read(&mut [0u8; 1]);
    }
    code
}

fn welcome() -> i32 {
    println!(
        "\n  Russht Viewer\n\n  Click a device on the left to open a shell there.\n  Sessions keep running on each device: click to switch back.\n\n  In the sidebar: 1-9 pick, r refresh, q quit."
    );
    let _ = std::io::stdin().read(&mut [0u8; 1]);
    0
}

/// Which device a click on sidebar row `y` (0-based) means.
fn device_at(list: &[Device], y: usize) -> Option<&Device> {
    y.checked_sub(HEADER).and_then(|index| list.get(index))
}

fn encode(list: &[Device]) -> String {
    list.iter()
        .map(|device| format!("{}:{}", device.name, device.state))
        .collect::<Vec<_>>()
        .join(",")
}

fn decode(text: &str) -> Vec<Device> {
    text.split(',')
        .filter_map(|entry| {
            let (name, state) = entry.split_once(':')?;
            plain(name).then(|| Device {
                name: name.to_string(),
                state: state.to_string(),
            })
        })
        .collect()
}

fn click(y: &str) -> i32 {
    let Ok(y) = y.trim().parse::<usize>() else {
        return 2;
    };
    match device_at(&decode(&option("@russh_list")), y) {
        Some(device) => select(&device.name),
        None => 0,
    }
}

fn render(list: &[Device], current: &str, loading: bool) {
    let mut out = String::from("\x1b[H\x1b[2J\x1b[1m devices\x1b[0m");
    out.push_str(if loading { " \x1b[2m(checking)\x1b[0m\r\n\r\n" } else { "\r\n\r\n" });
    for device in list {
        let dot = match device.state.as_str() {
            "online" => "\x1b[32m●\x1b[0m",
            "offline" => "\x1b[2m○\x1b[0m",
            _ => "\x1b[33m•\x1b[0m",
        };
        let name: String = device
            .name
            .chars()
            .take(usize::from(SIDEBAR_WIDTH) - 4)
            .collect();
        if device.name == current {
            out.push_str(&format!(" {dot} \x1b[7m{name}\x1b[0m\r\n"));
        } else {
            out.push_str(&format!(" {dot} {name}\r\n"));
        }
    }
    out.push_str("\r\n\x1b[2m 1-9 pick r refresh q quit\x1b[0m");
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(out.as_bytes());
    let _ = stdout.flush();
}

/// The left side: the list, kept fresh; keys 1-9, r and q.
fn sidebar() -> i32 {
    let _ = Command::new("stty")
        .args(["-icanon", "-echo"])
        .stdin(Stdio::inherit())
        .status();
    let _ = std::io::stdout().write_all(b"\x1b[?25l");
    let source = source();
    let (keys, input) = mpsc::channel::<u8>();
    std::thread::spawn(move || {
        let mut byte = [0u8; 1];
        while std::io::stdin().read(&mut byte).is_ok_and(|n| n == 1) {
            if keys.send(byte[0]).is_err() {
                break;
            }
        }
    });
    let here = Device {
        name: "here".to_string(),
        state: "online".to_string(),
    };
    let mut list = vec![here.clone()];
    let mut stale = true;
    let mut shown: Option<(Vec<Device>, String)> = None;
    let mut waited = Duration::ZERO;
    loop {
        if stale {
            render(&list, &option("@russh_current"), true);
            list = std::iter::once(here.clone()).chain(devices(source)).collect();
            set_option("@russh_list", &encode(&list));
            stale = false;
            waited = Duration::ZERO;
            shown = None;
        }
        let current = option("@russh_current");
        if shown.as_ref() != Some(&(list.clone(), current.clone())) {
            render(&list, &current, false);
            shown = Some((list.clone(), current));
        }
        match input.recv_timeout(Duration::from_secs(1)) {
            Ok(b'q') => {
                let _ = tmux(&["kill-session", "-t", &format!("={SESSION}")]);
                return 0;
            }
            Ok(b'r') => stale = true,
            Ok(digit @ b'1'..=b'9') => {
                if let Some(device) = list.get(usize::from(digit - b'1')) {
                    select(&device.name);
                }
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                waited += Duration::from_secs(1);
                stale = waited >= REFRESH;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return 0,
        }
    }
}

fn list_command(json: bool) -> i32 {
    let list = devices(source());
    if json {
        let items = list
            .iter()
            .map(|device| format!("{{\"name\":\"{}\",\"state\":\"{}\"}}", device.name, device.state))
            .collect::<Vec<_>>();
        println!("[{}]", items.join(","));
    } else {
        for device in &list {
            println!("{:<24} {}", device.name, device.state);
        }
    }
    0
}

const HELP: &str = "Russht Viewer: click between shells on your own devices.

Usage:
  russhtviewer                   open the viewer (click a device on the left)
  russhtviewer devices [--json]  list the devices it knows
  russhtviewer connect DEVICE    a shell on one device here, without the viewer

Devices come from LinuxMice (lmice fleet status; connected with lmice open)
when lmice is installed, else from the Host entries in ~/.ssh/config. Plain
SSH only: no agent, token or open port. Needs tmux.";

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let words = args.iter().map(String::as_str).collect::<Vec<_>>();
    let code = match words.as_slice() {
        [] => open(),
        ["devices"] => list_command(false),
        ["devices", "--json"] => list_command(true),
        ["connect", device] => connect(device, false),
        ["__sidebar"] => sidebar(),
        ["__welcome"] => welcome(),
        ["__click", y] => click(y),
        ["__connect", device] => connect(device, true),
        ["--version" | "-V"] => {
            println!("russhtviewer {}", env!("CARGO_PKG_VERSION"));
            0
        }
        ["help" | "--help" | "-h"] => {
            println!("{HELP}");
            0
        }
        _ => {
            eprintln!("{HELP}");
            2
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fleet_status_becomes_devices() {
        let json = r#"{"action": "fleet.status", "devices": [
          {"id": "lawnmower", "target": "admin@192.0.2.2", "state": "online", "detail": "ok"},
          {"id": "laptop-wsl", "target": "admin@192.0.2.4", "state": "unreachable", "detail": "offline"},
          {"id": "-oProxyCommand=x", "target": "x", "state": "online"}
        ]}"#;
        let list = parse_fleet(json);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], Device { name: "lawnmower".into(), state: "online".into() });
        assert_eq!(list[1].state, "offline");
    }

    #[test]
    fn ssh_config_hosts_without_patterns() {
        let text = "Host server\n  HostName 192.0.2.2\nHost *\n  ServerAliveInterval 30\nhost laptop desk\nHost server\nHostName x\n";
        let names = parse_ssh_config(text).into_iter().map(|d| d.name).collect::<Vec<_>>();
        assert_eq!(names, ["server", "laptop", "desk"]);
    }

    #[test]
    fn clicks_map_to_rows_and_the_list_round_trips() {
        let list = vec![
            Device { name: "here".into(), state: "online".into() },
            Device { name: "mini".into(), state: "offline".into() },
        ];
        assert_eq!(device_at(&list, 0), None);
        assert_eq!(device_at(&list, 2).map(|d| d.name.as_str()), Some("here"));
        assert_eq!(device_at(&list, 3).map(|d| d.name.as_str()), Some("mini"));
        assert_eq!(device_at(&list, 9), None);
        assert_eq!(decode(&encode(&list)), list);
    }

    #[test]
    fn names_and_quotes_are_safe() {
        assert!(plain("laptop-wsl") && plain("admin@host"));
        assert!(!plain("-oProxyCommand=x") && !plain("a;b") && !plain("") && !plain("a b"));
        assert_eq!(quote("it's"), "'it'\\''s'");
    }
}
