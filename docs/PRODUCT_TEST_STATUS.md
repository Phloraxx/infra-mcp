# Product verification — 14 October 2026

## Verified on Oracle Linux ARM64

- Backend integration journey: landing page/assets, wrong-password handling, CSRF origin rejection, session cookie attributes, pairing name validation, one-time code use, private credential responses, OAuth DCR/PKCE, MCP discovery, shell exit codes, filesystem CRUD, saved background output, agent restart, gateway restart, scoped credentials, revocation and duplicate local agent prevention.
- Linux user installation without sudo, including a path containing spaces.
- Actual systemd user-service restart and device reconnect.
- An active background job retains output across an actual agent-service restart.
- Reinstallation reuses the saved credential.
- Uninstall removes the test startup service and preserves local state.
- Missing-Node installation downloads and verifies an official private ARM64 runtime. This case uses manual startup; automatic startup is separately exercised by the systemd user installation case.
- Browser tests with fixture API responses: wrong-password recovery, sign-in, default Oracle selection, exactly one command submission, file read/edit/save, pairing instructions, disabled offline controls, 390-pixel mobile layout, sign-out and absence of JavaScript exceptions.
- Desktop, phone-width and pairing-dialog screenshots were visually reviewed.

The fixture checks are complemented by a live HTTPS browser check on `https://mcp.phloraxx.us.to`: real sign-in, real Oracle command execution, real file editing and saving, and phone-width rendering all passed. A live background job retained output across an actual Oracle agent-service restart. No JavaScript exceptions were observed.

A short 30-command check from Oracle through the public HTTPS domain measured 19 ms median and 32 ms p95 for an HTTP shell request. This does not measure ChatGPT connector or model latency. Sampled RSS after live work was 61 MiB for the agent and 87 MiB for the gateway; these are workload samples, not long-term idle measurements.

OAuth refresh-token support was added after live connector compatibility exposed a legacy scope mismatch. Access, refresh and dashboard token separation and refresh client binding are tested in the backend suite.

## Cross-platform CI verified

GitHub Actions run `38609102738` on commit `203a35d0580b3f314ae3523f8721360b7bdf5400` passed:

- Complete backend/agent integration on Windows, macOS ARM64 and Ubuntu.
- PowerShell installer parsing on Windows.
- Browser UI interaction checks on Ubuntu.

The first macOS run exposed an agent startup guard that compared a symlink path with a real path. The guard now compares canonical paths; the integration test starts the agent through a directory link to retain regression coverage.

The existing scaffold CI passed as well. These checks do not replace actual installation, permission and reboot tests on the user's devices.

## Not yet verified

- macOS installation, permissions, logout/login and full reboot on the user's Mac.
- Windows installation with and without administrator access, Scheduled Task/Run-key fallback, logout/login and full reboot on Rose.
- Linux/macOS/Windows system installation. Noninteractive sudo was unavailable on the inspected Oracle session; the privileged installer case was explicitly skipped.
- Full Oracle VM reboot. Existing services were not rebooted for this release.
- GUI automation, Android agents, interactive PTY sessions and resumable arbitrary tasks across a device reboot: not implemented in 0.3.0.
- Sleep/wake, network changes and slow connections on actual laptops/phone.
- Cold boot and long idle resource consumption on every supported device.

## Acceptance before calling the complete product ready

1. Verify an installation on clean Windows, macOS and Linux accounts, both with a preinstalled runtime and without one.
2. Verify user-mode startup after login and system-mode startup at boot; clearly report the available permissions and desktop session state.
3. Verify credential revocation, expired codes, interrupted downloads, disk-full errors and update rollback.
4. Verify GUI observation/actions with native helpers on Mac and Windows.
5. Verify Android pairing, reconnect, permissions and battery behaviour on the Motorola device.
6. Record measured RSS, idle CPU, network traffic, tool latency and response size, including long-running work.

This is a working core release, not completion of the entire multi-device/GUI/phone product.
