# Security

Russht Viewer grants interactive shell access to whoever holds the shared Masterdale bearer token.

## Rules

- Keep `DALE_TOKEN` / `RUSSH_MASTERDALE_TOKEN` private and rotate if leaked
- Prefer Tailscale-only reachability; do not publish port 45090 to the public internet
- Agents spawn the user shell in the home directory; treat the token like root SSH access for that user
- Idle sessions should be closed when finished

## Reporting

Open a private security advisory on the GitHub repository if you find a vulnerability.
