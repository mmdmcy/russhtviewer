# Russht Viewer

**Russht Viewer** (RUSSH) is a Rust tool for easy multi-device shell access on a private Tailscale fleet.

It is the shell sibling of [rustopviewer](https://github.com/mmdmcy/rustopviewer): one always-on host registry, agents on each machine, and a shared Masterdale bearer token instead of SSH keys.

## Surfaces

- CLI / TUI device picker and multi-session terminals
- Browser multi-tab terminals (xterm.js)
- Headless host and agent daemons

## Quick start

Share the same token everywhere:

```env
DALE_TOKEN=<same-as-masterdale>
# or RUSSH_MASTERDALE_TOKEN=...
```

On the registry host:

```bash
russhtviewer host --headless
```

On each other device:

```bash
russhtviewer agent --host http://<registry-tailscale-ip>:45090
```

From any device with the token:

```bash
russhtviewer                  # TUI: pick a device, open shells
russhtviewer devices
russhtviewer connect workstation
```

Browser: open `http://<registry-tailscale-ip>:45090/`, paste the Masterdale token, pick a device, open tabs.

## Auth model

- Fleet register / list / PTY require `Authorization: Bearer <token>`
- Token is the same `DALE_TOKEN` used by Masterdale / RustOpViewer
- Default network posture: listen on loopback + Tailscale IPs (not public WAN)

## Non-goals (v1)

- Real OpenSSH / SSH certificates
- Relaying PTY bytes through the host (clients connect to the agent URL directly after discovery)
- File transfer / clipboard

## License

MIT OR Apache-2.0
