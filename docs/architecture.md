# Architecture

## Roles

- **Host** (`russhtviewer host`): fleet registry + local shell agent + browser dashboard
- **Agent** (`russhtviewer agent --host URL`): local PTY server that registers/heartbeats to the host
- **Client** (TUI / `connect` / browser): lists devices from the host, opens WebSocket PTY sessions on the chosen agent

## Protocol

- `POST /v1/fleet/register`
- `POST /v1/fleet/heartbeat`
- `GET /v1/fleet/devices`
- `GET /v1/device`
- `WS /v1/pty` with `Authorization: Bearer <token>`

PTY frames are JSON text messages:

```json
{"type":"input","data":"..."}
{"type":"resize","cols":120,"rows":40}
{"type":"output","data":"..."}
{"type":"exit","code":0}
```

## Auth

SHA-256 hashed bearer compared in constant time style via hash equality of the configured token.
