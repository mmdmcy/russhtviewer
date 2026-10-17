# Security

Russht Viewer adds no access of its own: it runs `ssh` (or `lmice open`,
which runs ssh) with whatever keys and host-key checks you already have. It
listens on nothing and stores no secret.

- Device names from the fleet or `~/.ssh/config` are validated before use,
  so a name can never become an ssh option.
- Version 0.1 used a shared bearer token and agents with shells; if you ran
  it, stop those agents and rotate that token.

Report a vulnerability through a private security advisory on GitHub.
