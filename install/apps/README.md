# App templates

Put one `<name>.toml` registry file per app in this directory before running
`install-swapdock.sh`. The installer copies `apps/*.toml` into
`/srv/swapdock/apps/` and never overwrites a registry file that already exists,
so re-running it is safe.

This directory ships empty on purpose. `examples/apps/` is a fictional estate
used by the validator tests and is **not** an install template: its paths and
images do not exist on your host. Create your own file with:

```bash
swapdock init shop
# edit shop.toml, then:
swapdock --registry . validate
swapdock --registry . doctor
```

See `docs/quickstart.md` in the source repository for the full path from a
source checkout to a validated registry.
