# Changelog

All notable changes to swapdock. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.1.0] — 2026-10-05

First public release. A complete deploy loop for one Docker host:

- Registry: one TOML file per app as the single source of truth, with
  per-app and cross-app validation that refuses typos, port collisions,
  duplicate hostnames and swaps on stateful apps.
- Renderer: the nginx front-door config as a pure, deterministic function.
- Apply: atomic file replacement gated by `nginx -t`, with restore on
  failure and a reload-vs-restart check on the master pid.
- Strategies: `swap` (blue/green on a port pair) and `replace` (stop and
  restart on one port), chosen by the registry, with rollback through the
  same health gate.
- Health gate: container health plus a direct HTTP probe; a missing
  healthcheck fails the deploy instead of passing it.
- Verify: the access-log verdict — failures, per-upstream counts and the
  exact flip instant.
- Builder: releases built on a pluggable host (local, SSH, none), named by
  commit, never `latest`.
- Observability throughout: one subprocess chokepoint, an append-only JSONL
  run log, secret redaction before anything reaches a file, and dry-run
  rendering of every mutating command.
