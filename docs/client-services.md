# Running Motional as a Service

Motional automation runs in `motional-service`. The GUI and CLI edit the shared configuration and manage the per-user service registration.

Install and start the service:

```sh
motional-cli service install --start
```

Control it:

```sh
motional-cli service status
motional-cli service restart
motional-cli service stop
motional-cli service remove
```

The GUI exposes the same service controls in its top toolbar.

The service is intentionally installed in the logged-in user's session:

- Linux: `systemd --user` unit named `com.ejtbrown.motional.service`.
- macOS: LaunchAgent named `com.ejtbrown.motional.service`.
- Windows: logon Scheduled Task named `Motional Service`.

This keeps lock, display, sleep, key press, and per-user settings actions in the desktop session they need to control.

MSP is a plaintext TCP protocol. Run it only on trusted networks or behind a secure transport such as WireGuard, TLS termination, or SSH port forwarding.
