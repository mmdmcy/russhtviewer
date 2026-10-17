# Architecture

One binary, the standard library only, three parts:

- **The viewer** (`russhtviewer`): a tmux session `russh` with two panes. Its
  key table passes every key to the right pane; only mouse clicks on the
  sidebar are handled, by running `russhtviewer __click ROW`.
- **The sidebar** (`__sidebar`): lists `here` plus the devices, checks
  reachability every minute, keeps the list in a tmux option for clicks,
  and highlights the current device.
- **The connection** (`__connect DEVICE`): `lmice open DEVICE`, or
  `ssh -t DEVICE`, in the right pane; switching respawns that pane.

State lives in tmux session options (`@russh_view`, `@russh_list`,
`@russh_current`); nothing is written to disk. Device names are checked
(letters, digits, `-_.@`, never starting with `-`) before they reach a
command line.
