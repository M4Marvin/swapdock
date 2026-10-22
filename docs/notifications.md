# Failure notifications

**Status: one webhook for job failures and three monitor checks; not a full
alerting system.** `SWAPDOCK_WEBHOOK_URL` sends a JSON `POST` when an API-triggered
job fails (deploy, build, sync, backup, restore, apply, transfer), a CLI backup
fails, or `swapdock monitor` finds an app down, a certificate close to expiry,
or the root filesystem above the threshold. It does not retry and intentionally
does not send email: the one-user v1 uses a webhook only.

## Configure

```bash
# /etc/swapdock/api.env, mode 0600 (same file as the token)
SWAPDOCK_WEBHOOK_URL=https://hooks.example.com/swapdock
```

Restart the service after changing it:

```bash
sudo systemctl restart swapdock
```

An absent or empty value disables the webhook. The URL must be `http://` or
`https://` and must not contain whitespace or control characters. Treat the URL
as a secret if it embeds a token; it is passed to `curl` in its argv but not
written to the run log.

## Payload

```json
{
  "event": "job_failed",
  "service": "swapdock",
  "app": "chat",
  "run_id": "01J...",
  "error": "health gate failed: ...",
  "sent_at": "2026-10-22T04:00:00Z"
}
```

`error` is redacted with the same basic rules as the run log and truncated to
16 KiB. The webhook is sent **after** the run is closed; a slow endpoint cannot
change the run's status.

## Monitor checks

`swapdock monitor` is read-only and is what the optional monitor timer runs:

```bash
sudo swapdock --registry /srv/swapdock/apps monitor
sudo swapdock --registry /srv/swapdock/apps monitor --json
```

It checks:

- **App down** — every app with a `live_port` is probed on its health path.
- **Certificate** — each path in `SWAPDOCK_TLS_CERTS` is checked with
  `openssl x509 -checkend` for a 14-day window.
- **Disk** — `df -P /` is compared with `SWAPDOCK_DISK_THRESHOLD` (default 90%).

Environment (put it in `/etc/swapdock/api.env`):

```bash
SWAPDOCK_TLS_CERTS=/etc/letsencrypt/live/console.example.com/fullchain.pem
SWAPDOCK_DISK_THRESHOLD=90
SWAPDOCK_ALERT_REPEAT_SECS=21600   # suppress the same key for 6h
```

Alerts are de-duplicated in `/srv/swapdock/green/monitor-state.json`; a healthy
check clears the key so a later failure alerts immediately. The installer can
install the timer with `--with-monitor-timer`, but it does **not** enable it:

```bash
sudo systemctl enable --now swapdock-monitor.timer
systemctl list-timers swapdock-monitor.timer
journalctl -u swapdock-monitor.service -n 50
```

## Test it

```bash
# local receiver
python3 -m http.server 9000
# in another shell, with the server configured:
SWAPDOCK_WEBHOOK_URL=http://127.0.0.1:9000/hook
```

Trigger a known failure against a disposable registry, or use `curl` directly:

```bash
curl -fsS -X POST -H 'Content-Type: application/json' \
  -d '{"event":"job_failed","service":"swapdock","app":"demo","run_id":"test"}' \
  http://127.0.0.1:9000/hook
```

## Still open

- app-down / certificate-expiry / disk-threshold monitors and a systemd timer;
- retries, queueing, signing/HMAC and per-app routing;
- a UI for notification settings.

See [LAUNCH.md](../LAUNCH.md) G4.9.
