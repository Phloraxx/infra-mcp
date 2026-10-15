# Reliability and UX acceptance tests

These scenarios are required product tests, not optional manual checks.

## Installation

For each supported primary platform:

- fresh installation from dashboard instructions
- installer rerun on healthy installation, including replacement of a currently-running agent binary
- installer rerun on partially broken installation
- transient download failure
- expired enrollment during setup
- no preinstalled Node/Python package manager
- paths/usernames containing spaces and Unicode
- uninstall then reinstall
- Linux install/repair from a remote non-login shell with `XDG_RUNTIME_DIR` and `DBUS_SESSION_BUS_ADDRESS` absent

Expected: normal recovery path does not require manual deletion of service files/config directories.

## Startup/reboot

- user logs out/in
- normal reboot
- forced restart
- agent process killed
- startup item/service temporarily disabled then repaired

Expected: agent returns automatically when the OS/user-session model allows it, with the same stable device identity.

## Network

- Wi-Fi off/on
- Wi-Fi network change
- DHCP/IP change
- DNS temporarily unavailable
- gateway unavailable for 10 minutes
- TLS connection reset
- laptop sleep for short and long periods
- gateway restart during active PTY/job/transfer

Expected: no duplicate logical devices; reconnect is automatic; active work resumes where specified.

## Connection races

- two agent controller instances briefly start
- stale socket remains during reconnect
- new connection arrives before old timeout
- duplicated transport frame
- delayed response arrives from superseded connection

Expected: newest authenticated generation wins deterministically and old responses cannot corrupt current request state.

## Persistent sessions

Test bash/zsh/PowerShell/cmd where applicable plus Python/Node/Codex-like interactive sessions.

- browser refresh
- ChatGPT reconnect
- gateway restart
- device network loss
- controller agent restart; verify fresh PTY/job activity after reconnect, not merely old transcript visibility
- machine reboot

Expected: survive all documented survivable events; rebooted sessions become explicitly interrupted.

## Transfers

Test text and binary files at small, 100 MB, and 1 GB scales.

- disconnect at 10%, 50%, 95%
- gateway restart
- destination disk full
- checksum mismatch
- destination already exists

Expected: resume safely; never expose a corrupt file as a successful final destination.

## Dashboard

- device state changes update without refresh
- attention state gives an actionable reason
- permission change is detected
- update progress remains visible across page refresh
- activity record exists for consequential actions
- sanitized diagnostic export contains no credentials

## Recovery UX

Simulate:

- missing startup entry
- damaged binary
- revoked credential
- incompatible gateway/agent protocol
- duplicate legacy installation
- permission denied
- wrong system clock impacting TLS

Expected: `infra doctor` identifies the root problem or narrows it to a useful class; dashboard surfaces the same diagnosis and a repair action when safe.

## Performance targets

Initial targets, subject to benchmark revision:

- dashboard online-state propagation: <2 seconds after successful agent connection
- healthy reconnect after ordinary network restoration: typically <3 seconds
- local CLI `infra status`: <500 ms when service is healthy
- no unbounded memory growth during long sessions/transfers
- bounded local transcript/job retention with configurable limits

Desktop Commander remains the comparison baseline, but Infra must also be tested against these explicit behavioral requirements.