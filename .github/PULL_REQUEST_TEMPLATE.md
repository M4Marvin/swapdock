## Summary

<!-- What does this change, and why? Link the gate/defect ID if there is one. -->

Fixes/relates to: <!-- SEC-01, DEP-03, DIST-02, ... or "none" -->

## Type of change

- [ ] Bug fix
- [ ] Security hardening (Gate 0)
- [ ] Deploy correctness / recovery (Gate 2)
- [ ] Install / upgrade / uninstall (Gate 1)
- [ ] Documentation
- [ ] CI, release or packaging
- [ ] Refactor with no behavior change

## What changed

<!-- A short list. Call out anything that changes a default, a file path,
     a file permission, or a safety check. -->

## Test plan

<!-- Be specific: commands run, hosts/versions, what passed. -->

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --locked --all-targets
# web, scripts, docs checks if touched
```

- [ ] Added or updated a test that fails without this change (bug fixes)
- [ ] Ran the installer with `--dry-run` (if install files changed)
- [ ] Ran the uninstaller with `--dry-run` (if uninstall files changed)
- [ ] Kept all subprocess spawns in `src/exec.rs`
- [ ] Did not weaken root/`SUDO_USER`, sudoers, `nginx -t`, or atomic apply checks

## Safety checklist

- [ ] No secrets, tokens, personal IPs/hostnames/emails, or private-fleet
      screenshots in the diff
- [ ] Docs updated; unfinished behavior marked **planned** with a gate/defect ID
- [ ] `shellcheck` and `bash -n` pass for shell changes
- [ ] `python3 scripts/check-docs-links.py` passes for doc changes
- [ ] No new Rust dependency (or the PR explains why one is necessary)
- [ ] Did not edit `LAUNCH.md` or `swapdock-pre-launch-audit.md` in this PR

## DCO

- [ ] I signed off my commits (`git commit -s`) and have the right to submit
      this change under the MIT license. There is no CLA.
