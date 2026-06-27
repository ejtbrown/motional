# Running Motional Clients at Login

## Linux systemd user service

Create `~/.config/systemd/user/motional-cli.service`:

```ini
[Unit]
Description=Motional CLI automation

[Service]
ExecStart=%h/bin/motional-cli watch --server 127.0.0.1:7080 --token MOTIONAL_TOKEN --sensor office --on-motion power-on-display --on-absence power-off-display
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
```

MSP is a plaintext TCP protocol. Run it only on trusted networks or behind a secure transport such as WireGuard, TLS termination, or SSH port forwarding.

Enable it:

```sh
systemctl --user daemon-reload
systemctl --user enable --now motional-cli.service
```

## macOS LaunchAgent

Create `~/Library/LaunchAgents/com.ejtbrown.motional-gui.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.ejtbrown.motional-gui</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Users/YOU/bin/motional-gui</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
</dict>
</plist>
```

Load it:

```sh
launchctl load ~/Library/LaunchAgents/com.ejtbrown.motional-gui.plist
```

## Windows Scheduled Task

Run from an elevated PowerShell prompt, adjusting paths and tokens:

```powershell
$action = New-ScheduledTaskAction -Execute "C:\Users\YOU\bin\motional-gui.exe"
$trigger = New-ScheduledTaskTrigger -AtLogOn
Register-ScheduledTask -TaskName "Motional GUI" -Action $action -Trigger $trigger -Description "Run Motional GUI automation"
```
