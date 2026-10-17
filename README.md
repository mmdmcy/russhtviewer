# Russht Viewer

Click between shells on your own devices.

Russht Viewer opens a tmux window with your devices in a sidebar on the
left and the shell of the one you picked on the right. Click another device
and the right side switches to it. Sessions keep running on each device, so
switching back is instant.

```
┌ devices ─────────┬──────────────────────────────────────────┐
│ ● here           │ admin@server:~$                          │
│ ● server         │                                          │
│ ○ old-laptop     │                                          │
│ ● laptop-wsl     │                                          │
└──────────────────┴──────────────────────────────────────────┘
```

No agent, no token, no open port. It opens plain SSH:

- through LinuxMice (`lmice open DEVICE`) when
  `lmice` is installed: device names and reachability come from the fleet,
  and LinuxMice handles keys, pinned host keys and jumps;
- otherwise through the `Host` entries in `~/.ssh/config`.

## Use

```bash
russhtviewer                   # open the viewer; click a device
russhtviewer devices [--json]  # list the devices it knows
russhtviewer connect DEVICE    # one device's shell here, without the viewer
```

In the sidebar: click a device, or press 1-9; `d` opens the current
device's desktop in your browser (`lmice desktop`, RustOp Viewer); `r` checks reachability
again (it also does so every minute); `q` closes the viewer. Every other
key goes to the shell on the right, so a tmux or Mobailmux on the remote
device keeps its own Ctrl-b.

## Requirements

tmux, and SSH access to your devices. Runs wherever tmux does: Linux,
macOS, and WSL on Windows (no Windows build needed; Windows Terminal shows
it fine).

## Build

```bash
cargo build --release   # one binary, no dependencies
```

Version 0.1 had its own agents, registry and shared bearer token; it is kept
at the git tag `agent-era`. 0.2 replaced all of that with SSH.

## License

MIT or Apache-2.0.
