# Upgrade

**Status: no upgrade path is supported yet.** No release has been published, so
there is no previous version to upgrade from, no schema migration, and no
upgrade test (G1.4 is open). This page describes the intended procedure and
what to back up; do not treat it as a tested promise.

## Before you start

1. **Read the release notes and** [LAUNCH.md](../LAUNCH.md). If a P0 that
   affects your install is open, do not upgrade.
2. **Back up the registry and state** (see [backup-restore.md](backup-restore.md)):

   ```bash
   stamp="$(date +%F-%H%M)"
   sudo tar -czf "/root/swapdock-state-$stamp.tgz" \
     -C / srv/swapdock/apps srv/swapdock/overrides
   ```

   `/srv/swapdock/green` contains deploy state; include it if you have used
   `apply`/`up`:

   ```bash
   sudo tar -czf "/root/swapdock-green-$stamp.tgz" -C / srv/swapdock/green
   ```

3. **Record the current version and binary:**

   ```bash
   swapdock --version
   sha256sum /usr/local/bin/swapdock
   ```

## Intended upgrade procedure (planned)

The installer is idempotent and is the intended upgrade mechanism. It keeps the
previous binary as `/usr/local/bin/swapdock.bak` and the previous hand-written
nginx files as `.bak`.

```bash
# 1. Build or unpack the new swapdock-dist/ tree.
bash scripts/package-dist.sh

# 2. Read the plan.
bash swapdock-dist/install-swapdock.sh --dry-run swapdock-dist

# 3. Run the installer. It runs nginx -t but never reloads nginx.
sudo bash swapdock-dist/install-swapdock.sh swapdock-dist

# 4. Check the new binary and the registry.
swapdock --version
swapdock validate
swapdock doctor
```

If the API unit is installed, stop it before the upgrade and start it again
afterwards:

```bash
sudo systemctl stop swapdock    # if enabled
# ... install ...
sudo systemctl start swapdock   # only if you accept the security caveats
```

The installer does not reload nginx. A reload is a separate, deliberate action.

## Registry schema and migrations

The registry has **no versioned migration path yet**. `deny_unknown_fields` is
deliberate (a typo should not be silently ignored), but it means a newer binary
that adds a field can make an older binary reject every file. Until G1.4:

- upgrade both the binary and the registry together;
- keep the pre-upgrade registry backup;
- if an older binary is restored, restore the matching registry snapshot too.

`swapdock export` / `import` and automatic migrations are planned (G1.5,
DATA-02, G1.4); they do not exist today.

## Rolling back the upgrade

```bash
# Stop the API if it is running.
sudo systemctl stop swapdock 2>/dev/null || true

# Restore the previous binary, if the installer kept one.
sudo install -m 0755 /usr/local/bin/swapdock.bak /usr/local/bin/swapdock

# Restore the hand-written nginx files from the installer's .bak copies.
for f in /etc/nginx/conf.d/00-swapdock-http.conf \
         /etc/nginx/snippets/proxy-common.conf \
         /etc/nginx/snippets/spa-fallback.conf; do
  if [ -e "$f.bak" ]; then sudo mv -f "$f.bak" "$f"; fi
done

# Restore the registry snapshot you took above.
sudo tar -xzf /root/swapdock-state-<stamp>.tgz -C /
sudo nginx -t
```

The binary rollback is only as good as the `.bak` file. If the installer has
been run several times, `.bak` is the *first* previous binary, not the most
recent one; take your own copy before every upgrade if you need a precise
rollback:

```bash
sudo cp -a /usr/local/bin/swapdock "/root/swapdock-$(date +%F-%H%M)"
```

## What is still missing

| Missing | Tracking |
|---|---|
| A released version to upgrade from | G1.7 / DIST-01 |
| Registry schema version and migration | G1.4 |
| An upgrade test from the previous release | G1.4 |
| `swapdock export` / `import` | G1.5, DATA-02 |
| A tested post-upgrade deploy and rollback | G2 |

Until those exist, the honest description is: **reinstall from source and keep
a backup.** Do not describe upgrades as supported.
