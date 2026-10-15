# Platform implementation notes

Infra presents consistent MCP semantics while using native platform behavior underneath.

## Shared requirements

Every platform implementation should expose the same high-level capability model but may report unsupported actions explicitly.

Primary build targets:

- Linux ARM64
- macOS ARM64
- Windows x64

Secondary later targets:

- Linux x64
- Windows ARM64

## Linux

### Startup

- user install: systemd user service where available
- optional system install/helper: systemd system service

Support non-systemd/manual mode later only if it provides real value.

### Terminal/process

- Unix PTY
- process groups for tree cancellation
- `/proc` when useful, with portable fallbacks where practical

### Files/search

Normal POSIX semantics. Be explicit about symlinks, mount boundaries, permissions, and trash support.

### Desktop

Later phase:

- AT-SPI accessibility where a desktop session exposes it
- compositor/session-specific screenshot/input fallbacks as needed

Headless server support must never depend on desktop libraries.

### Privileged helper

Unix domain socket with strong filesystem permissions plus peer-credential checks where available.

## macOS

### Startup

Use launchd. User agent should survive normal logins/restarts and reconnect after sleep/wake.

### Terminal

Unix PTY with correct zsh/bash environment behavior. Avoid assuming interactive shell startup files are loaded for background launchd processes.

### Credentials

Store long-lived device identity in Keychain when feasible.

### Desktop

Later phase:

- Accessibility API for semantic UI tree/actions
- native screen-capture APIs
- native input events
- permission detection and clear `doctor` guidance for Accessibility/Screen Recording permission

A small Swift/Objective-C shim is acceptable where Rust bindings are fragile. Keep the public agent protocol in Rust.

### Privileged helper

Use an Apple-supported service/helper strategy rather than running the entire agent as root.

## Windows

### Startup

User mode may use Scheduled Task or an equivalent robust login startup mechanism. Full/system mode may install a Windows service for the helper.

### Terminal

Use ConPTY for interactive terminal sessions. Process-tree cancellation must account for Windows job objects/process groups rather than only killing the immediate child.

### Credentials

Use DPAPI/Credential Manager or another platform-protected facility appropriate for the identity material.

### Files

Normalize behavior carefully for:

- drive-letter paths
- UNC paths if supported
- case-insensitivity
- file locking
- long paths
- recycle bin vs permanent delete

### Desktop

Later phase:

- Windows UI Automation
- native screen/window capture
- SendInput or appropriate controlled input API

### Privileged helper

Windows service communicating over authenticated/restricted named pipe or equivalent local IPC.

## Capability reporting

Agent hello should advertise capability flags derived from runtime checks, not merely platform name.

Examples:

- `pty`
- `docker`
- `git`
- `desktop.capture`
- `desktop.accessibility`
- `clipboard`
- `privileged_helper`

This lets the gateway fail cleanly before dispatching unsupported actions.
