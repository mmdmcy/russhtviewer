# Fleet deploy

## Registry host

```bash
cargo install --path . --root "$HOME/.local"
mkdir -p ~/.config/russhtviewer
# same DALE_TOKEN as Masterdale
cat > ~/.config/russhtviewer/masterdale.env <<'EOF'
DALE_TOKEN=<same-as-masterdale>
RUSSH_FLEET_HOST=http://<registry-tailscale-ip>:45090
RUSSH_DEVICE_ID=registry
EOF
russhtviewer host --headless
```

## Agents

```bash
russhtviewer agent --host http://<registry-tailscale-ip>:45090
```

Use the same `DALE_TOKEN` on every agent. Build with the local Rust toolchain on each OS (`cargo install --path .`).

## Checks

```bash
russhtviewer devices --host http://<registry-tailscale-ip>:45090
russhtviewer connect workstation --host http://<registry-tailscale-ip>:45090
```

Browser: `http://<registry-tailscale-ip>:45090/` → paste token → open device tabs.
