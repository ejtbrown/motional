# Motional

Motional helps computers react sensibly to motion or presence in their physical environment.

The project has two parts:

- `hubitat/Motional.groovy`: a Hubitat app that exposes selected motion and presence sensors through token-scoped HTTP endpoints.
- `motional-service-protocol.md`: a source-neutral TCP protocol for Motional service implementations.
- `apps/motional-lock`: cross-platform clients plus a per-user background service that speaks Motional Service Protocol.

## Hubitat App

Install `hubitat/Motional.groovy` as a Hubitat app, select the motion or presence sensors that should be exposed, then create one or more Motional bearer tokens. Each Motional token is granted access to specific sensor API names.

Hubitat app endpoints normally live under the hub's app API path:

```text
http://<hubitat-host>/apps/api/<app-id>/<sensor-name>?access_token=<hubitat-app-access-token>
```

The Motional authorization token is sent separately:

```sh
curl \
  -H "Authorization: Bearer <motional-token>" \
  "http://hubitat.local/apps/api/123/office?access_token=<hubitat-app-access-token>"
```

Example response:

```json
{
  "sensor": "office",
  "displayName": "Office Motion",
  "active": false,
  "attribute": "motion",
  "value": "inactive",
  "secondsSinceTriggered": 183,
  "lastTriggeredAt": "2026-06-14T08:25:10Z"
}
```

Hubitat custom apps do not provide a supported way to bind an arbitrary listener port such as `7080` directly from Groovy app code. For broader VLAN exposure, prefer a separate Motional service that speaks `motional-service-protocol.md` on port `7080` and treats Hubitat as only one upstream sensor source.

## Motional Clients and Service

Build locally:

```sh
cd apps/motional-lock
cargo build --release --bins
```

Run the cross-platform GUI:

```sh
./target/release/motional-gui
```

Motional automation now runs in `motional-service`. The GUI and CLI are clients for configuring that service and controlling its per-user service registration.

The GUI lets users add any number of Motional server entries, choose one sensor per entry, configure action lists for connection, motion, and absence transitions, and install, remove, start, stop, or restart the service.

Each GUI server entry also supports connection-state actions. A typical resilient setup is:

- On Server Connected: `Disable Timed Screen Lock`, `Disable Timed Sleep`
- On Server Disconnected: `Enable Timed Screen Lock`, `Enable Timed Sleep`

That lets Motional control lock and sleep while the upstream MSP server is reachable, then falls back to the operating system's normal delay-based behavior if the MSP server becomes unavailable.

When the service changes timed screen lock or timed sleep settings, it records the original operating system values first and restores them when the service exits. The foreground CLI watch mode still restores those values on Ctrl-C.

Install and start the per-user service from the CLI:

```sh
./target/release/motional-cli service install --start
```

Control it later:

```sh
./target/release/motional-cli service status
./target/release/motional-cli service restart
./target/release/motional-cli service stop
./target/release/motional-cli service remove
```

The service uses the same JSON configuration that the GUI edits. CLI config helpers are also available:

```sh
./target/release/motional-cli config path
./target/release/motional-cli config show
./target/release/motional-cli config write --file motional-config.json
```

Service installation is intentionally per-user:

- Linux: `systemd --user` unit.
- macOS: LaunchAgent.
- Windows: logon Scheduled Task.

This keeps desktop automation in the logged-in user's session instead of a privileged system account.

Use the CLI to list sensors:

```sh
./target/release/motional-cli list \
  --server 127.0.0.1:7080 \
  --token "<motional-token>"
```

Watch one sensor from the CLI:

```sh
./target/release/motional-cli watch \
  --server 127.0.0.1:7080 \
  --token "<motional-token>" \
  --sensor office \
  --on-motion power-on-display \
  --on-absence power-off-display
```

MSP is a plaintext TCP protocol. Run it only on trusted networks or behind a secure transport such as WireGuard, TLS termination, or SSH port forwarding.

CLI action specs:

- `power-off-display`
- `power-on-display`
- `shut-down-system`
- `disable-timed-screen-lock`
- `enable-timed-screen-lock`
- `disable-timed-sleep`
- `enable-timed-sleep`
- `rest|METHOD|URL|BODY`
- `rest|METHOD|URL|@/path/to/body-file`

Linux also supports `logout-local-terminal-users`.

The GUI action set includes Lock Screen, Unlock Screen, Power Off Display, Power On Display, Shut Down System, Key Press, and REST API Call. Unlock Screen is only available where the operating system exposes a safe unlock command; macOS and Windows intentionally report it as unsupported.
