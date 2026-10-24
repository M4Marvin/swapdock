export type FeatureHref =
  | '/features/zero-downtime/'
  | '/features/stateful-apps/'
  | '/features/recovery/'
  | '/features/operations/'

export type Feature = {
  slug: string
  icon: string
  title: string
  blurb: string
  href?: FeatureHref
}

export const FEATURES: Feature[] = [
  {
    slug: 'zero-downtime',
    icon: 'swap',
    title: 'Zero-downtime cutovers',
    blurb:
      'The new release starts on the free port of the pair. Traffic moves only after it passes container health and the front-door probe.',
    href: '/features/zero-downtime/',
  },
  {
    slug: 'stateful-apps',
    icon: 'database',
    title: 'SQLite-safe by rule',
    blurb:
      'Stateful apps take the replace path. The validator refuses a swap, so two containers can never hold the same file open.',
    href: '/features/stateful-apps/',
  },
  {
    slug: 'rollback',
    icon: 'undo',
    title: 'Rollback is a deploy',
    blurb:
      'The previous image stays tagged until the next commit. Rollback runs the same strategy and gate as a forward deploy.',
  },
  {
    slug: 'nginx',
    icon: 'file',
    title: 'nginx you never edit',
    blurb:
      'The front door is generated from the registry, validated, written with one atomic rename, and tested before reload.',
  },
  {
    slug: 'recovery',
    icon: 'shield',
    title: 'Crashes leave evidence',
    blurb:
      'A pending record is fsynced before the flip. After a crash, reconcile proves nginx, the image, and health agree before it commits.',
    href: '/features/recovery/',
  },
  {
    slug: 'run-log',
    icon: 'log',
    title: 'A deploy you can replay',
    blurb:
      'Every docker, git, and nginx call is one JSONL step: argv, exit code, duration. Resume continues where the run stopped.',
  },
  {
    slug: 'locks',
    icon: 'lock',
    title: 'No interleaved deploys',
    blurb:
      'One estate lock, one lock per app, one for nginx. A second deploy waits; it never renders a stale port and applies it late.',
  },
  {
    slug: 'onboarding',
    icon: 'plug',
    title: 'Bring the apps you have',
    blurb:
      'Import an existing Compose service, or adopt a legacy container at the commit it is already running. Neither deploys anything.',
  },
  {
    slug: 'operations',
    icon: 'gauge',
    title: 'Day two is included',
    blurb:
      'Doctor checks the host, verify reads the access log, monitor watches ports and certificates, prune keeps rollback images.',
    href: '/features/operations/',
  },
]

export type FeatureDetail = {
  slug: string
  kicker: string
  title: string
  lead: string
  description: string
  problem: { title: string; body: string[] }
  mechanism: {
    title: string
    body: string[]
    bullets: string[]
    diagram: 'topology' | 'strategies' | 'recovery' | 'operations'
    code?: { title: string; lines: string[]; prompt?: boolean }
  }
  outcomes: string[]
  related: string[]
}

export const FEATURE_DETAILS: Record<string, FeatureDetail> = {
  'zero-downtime': {
    slug: 'zero-downtime',
    kicker: 'Zero downtime',
    title: 'Traffic moves once.',
    lead:
      'A deploy should not be a small outage with a maintenance window. swapdock starts the new release beside the old one, proves it serves, and only then moves the front door.',
    description:
      'How swapdock moves traffic between two back ports with a health gate, a generated nginx config, and a front-door probe.',
    problem: {
      title: 'Every deploy is a leap of faith',
      body: [
        'The usual single-box deploy is stop, start, and refresh. For the length of a container start, the port is dead. The nginx config may be wrong. The health check is whether the page loads for you.',
        'That is not a deployment strategy. It is a hope that the next 30 seconds are quiet.',
      ],
    },
    mechanism: {
      title: 'Two ports, one gate, one flip',
      body: [
        'Each app owns two back ports. A deploy starts the new release on the port that is not live, in its own Compose project. The old container keeps every connection it has.',
        'Before nginx moves, the new release must pass two checks: the container health status, and a real HTTP request on its port. Then the whole estate is rendered, validated, tested with nginx -t, and reloaded.',
        'After the flip, swapdock probes the public front door with the app Host header, waits 15 seconds for old keepalives to drain, and only then stops the old container.',
      ],
      bullets: [
        'The live and old ports alternate, so the front port never changes.',
        'A failed health gate leaves the old release serving traffic.',
        'A failed front-door probe puts the old port back.',
        'The old container stays until both gates pass.',
      ],
      diagram: 'topology',
      code: {
        title: 'One command, twenty recorded steps',
        lines: ['sudo swapdock --registry . up blog --release 9c1f2ab'],
      },
    },
    outcomes: [
      'No maintenance window for a normal deploy.',
      'The old release is still running if the new one cannot serve.',
      'The run log names every step, so a slow deploy is diagnosable.',
    ],
    related: ['recovery', 'stateful-apps', 'operations'],
  },
  'stateful-apps': {
    slug: 'stateful-apps',
    kicker: 'Stateful apps',
    title: 'One writer, on purpose.',
    lead:
      'A SQLite file cannot be open in two containers. swapdock makes that a rule in the registry, not a thing you remember at 2am.',
    description:
      'How swapdock handles stateful apps with the replace strategy, a validator that refuses unsafe swaps, and pre-deploy backups.',
    problem: {
      title: 'The fast deploy corrupts the database',
      body: [
        'Blue/green assumes the two releases can run side by side. That assumption fails the moment the app writes to a local file or a named volume. Two containers, one file, and the OS does not stop you.',
        'The safe answer is usually downtime. The fast answer is corruption. Neither is a decision you want to make from memory.',
      ],
    },
    mechanism: {
      title: 'The strategy is a fact, not a flag',
      body: [
        'A registry entry with writes_state = true cannot use swap. The validator refuses it before anything starts, and the deploy path has no override flag to click.',
        'Replace stops the old container, starts the new one on the same port, and gates it. There is a short gap, but there is only ever one writer.',
        'Before the stop, swapdock can run a backup command from the registry. It is an argv, never a shell string, with four placeholders: {app}, {release}, {timestamp}, {file}.',
      ],
      bullets: [
        'writes_state = true forces replace.',
        'A named volume or depends_on also forces replace on import.',
        'The backup command runs before any container or traffic change.',
        'A tag conflict is refused, never silently overwritten.',
      ],
      diagram: 'strategies',
      code: {
        title: 'The registry decides',
        prompt: false,
        lines: ['writes_state = true', 'strategy = "replace"'],
      },
    },
    outcomes: [
      'No two containers hold the same database file.',
      'The unsafe option is not available to a tired operator.',
      'A failed replace restarts the previous release and gates it.',
    ],
    related: ['zero-downtime', 'recovery', 'operations'],
  },
  recovery: {
    slug: 'recovery',
    kicker: 'Recovery',
    title: 'Crashes leave evidence.',
    lead:
      'The dangerous window is between the nginx flip and the registry commit. swapdock writes a record before it changes the host, and repairs only from proof.',
    description:
      'How the pending journal and reconcile recover a deploy that died between the nginx flip and the registry commit.',
    problem: {
      title: 'A killed deploy leaves two truths',
      body: [
        'nginx is serving the new release. The registry still names the old one. A status command can lie confidently, and the next deploy starts from the wrong state.',
        'Restarting the tool and hoping it sorts itself out is how a small failure becomes an incident.',
      ],
    },
    mechanism: {
      title: 'A record, then proof',
      body: [
        'Before the flip, swapdock writes app, release, ports, phase, and run id to a pending record. The file is fsynced, then the directory is fsynced.',
        'The record advances through flipping, flipped, and stopping, and is removed only after the registry commit. A process that dies leaves the correct phase behind.',
        'reconcile reads that record and proves the host state: the exact proxy_pass line for the pending port, the image id of the running container, and health. Any mismatch is a named refusal. Only full agreement commits.',
      ],
      bullets: [
        'The journal is per app, small, and human-readable JSON.',
        'Static apps are checked by symlink target instead of a container.',
        'A wrong image is refused, not retagged.',
        'The registry is only advanced when the evidence agrees.',
      ],
      diagram: 'recovery',
      code: {
        title: 'Repair, or get a specific refusal',
        lines: ['swapdock status blog', 'swapdock reconcile blog'],
      },
    },
    outcomes: [
      'No guessing after a crash.',
      'The operator gets the exact reason a repair was refused.',
      'The next deploy starts from a registry that matches the host.',
    ],
    related: ['zero-downtime', 'operations', 'stateful-apps'],
  },
  operations: {
    slug: 'operations',
    kicker: 'Operations',
    title: 'Day two is included.',
    lead:
      'The deploy is the easy part. swapdock also answers the questions a deploy console usually learns too late: is it up, did traffic move, what is on disk, what can be deleted.',
    description:
      'verify, monitor, prune, doctor, backups, and the console: the day-two operations around a swapdock estate.',
    problem: {
      title: 'The deploy ends. The questions do not.',
      body: [
        'Did any request fail with 5xx since the deploy? Which upstream served, and when did traffic move? Is the certificate close to expiry? Is the disk filling? Which image is still needed for a rollback?',
        'None of those are deploy steps, and all of them are the reason a deploy is not finished.',
      ],
    },
    mechanism: {
      title: 'Small tools, one source of truth',
      body: [
        'verify reads the nginx access log and answers three questions: failures, upstreams, and the instant traffic moved. Malformed lines are counted, not fatal.',
        'monitor watches live ports, certificates under 14 days, and a root filesystem over 90%, with a repeat window so a five-minute timer does not spam.',
        'prune selects old images with a pure function: keep the current release, old_release, the newest rollback tags, and anything the tool did not create.',
        'doctor checks the host before a deploy; the console puts the same data in one place.',
      ],
      bullets: [
        'verify uses the system access log, not the tool’s own opinion.',
        'monitor sends findings through the same webhook as deploy failures.',
        'prune never deletes an image a rollback may need.',
        'The console reads the same registry and run log as the CLI.',
      ],
      diagram: 'operations',
      code: {
        title: 'The after-care loop',
        lines: ['swapdock verify blog --since 30m', 'swapdock monitor', 'swapdock prune blog --keep 3'],
      },
    },
    outcomes: [
      'Traffic movement is verified from the log, not assumed.',
      'Certificates and disk do not fail silently.',
      'Rollback images survive pruning.',
    ],
    related: ['recovery', 'zero-downtime', 'stateful-apps'],
  },
}
