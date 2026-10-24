export const FAQ = [
  {
    q: 'Do I need Kubernetes or a cloud account?',
    a: 'No. swapdock targets one Linux host with Docker Engine, Docker Compose v2, nginx, and git. It is for the box you already pay for.',
  },
  {
    q: 'Is this a PaaS?',
    a: 'No. There is no hosted control plane, no account, and no vendor in the traffic path. You install a binary and keep your files.',
  },
  {
    q: 'Does it replace Docker Compose?',
    a: 'No. It drives Compose and never edits your file. The port and image are passed through environment variables, and an optional generated override pins the container name.',
  },
  {
    q: 'My app uses SQLite. Is it safe?',
    a: 'Yes, by rule. A registry entry with writes_state = true cannot use swap; the validator refuses it. Replace stops the old container before the new one starts, so only one process holds the file.',
  },
  {
    q: 'What happens when a deploy fails?',
    a: 'Before the flip, traffic stays on the old release. After the flip, a swap puts the old port back; a replace restarts the previous release and gates it; a static deploy restores the symlink.',
  },
  {
    q: 'What if the process is killed mid-deploy?',
    a: 'The pending journal records the phase. reconcile proves the nginx config, the image id, and health before advancing the registry. A mismatch is a named refusal, not a guess.',
  },
  {
    q: 'Is it production-ready?',
    a: 'Not yet. v0.2 is pre-launch: the deploy loop and its recovery paths are covered by tests, but there are no release binaries and the launch gates in LAUNCH.md are open. Use it on a host you can afford to learn on.',
  },
  {
    q: 'Does it phone home?',
    a: 'No telemetry and no analytics. The only outbound traffic is what you configure: image pulls, build pushes, and optional webhook alerts.',
  },
  {
    q: 'Can I keep my own TLS and tunnel?',
    a: 'Yes. nginx binds loopback, and the front port is what Cloudflare Tunnel, Tailscale, or plain DNS points at. TLS stays where it already terminates.',
  },
  {
    q: 'Can it manage more than one host?',
    a: 'No, by design. One host, one registry, one front door. That constraint keeps the model and the failure modes small.',
  },
  {
    q: 'How do I install it today?',
    a: 'Build the dist tree from source with scripts/package-dist.sh and run the installer. There is no published release binary or crates.io package yet.',
  },
] as const
