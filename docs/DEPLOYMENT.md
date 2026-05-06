# Deployment Guide

## VPS Deployment (Systemd)

1. Install runtime using [`scripts/install-runtime.sh`](../scripts/install-runtime.sh).
2. Copy config:

```bash
cp "$HOME/.local/runtime-server.toml.example" "$HOME/runtime-server.toml"
```

3. Login providers on machine:

```bash
codex login
claude login
```

4. Create user service `~/.config/systemd/user/gg-runtime.service`:

```ini
[Unit]
Description=GG Standalone Agent Runtime
After=network.target

[Service]
Type=simple
ExecStart=%h/.local/bin/gg-runtime-server --config %h/runtime-server.toml
Restart=on-failure
RestartSec=2
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
```

5. Enable and start:

```bash
systemctl --user daemon-reload
systemctl --user enable --now gg-runtime.service
journalctl --user -u gg-runtime.service -f
```

## Mac LaunchAgent (Simple)

For local always-on use, run in `tmux`/`screen` or use a LaunchAgent. Minimal manual start:

```bash
~/.local/bin/gg-runtime-server --config ~/runtime-server.toml
```

## Upgrade

```bash
./scripts/install-runtime.sh latest
systemctl --user restart gg-runtime.service
```

Set `GG_RUNTIME_REPO` only if you intentionally want to upgrade from a forked release source.

## Security Notes

- Set `auth.token` in config for a stable API token, or keep token-file bootstrap.
- Bind `server.bind_address` to localhost if frontends are on the same host.
- Put TLS/reverse-proxy in front if exposing to external networks.
