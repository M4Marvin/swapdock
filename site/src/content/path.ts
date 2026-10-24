export type Stage = {
  id: string
  n: string
  title: string
  time: string
  have: string
  run: string[]
  get: string
  detail: string
  warning?: string
}

export const STAGES: Stage[] = [
  {
    id: 'inventory',
    n: '00',
    title: 'Know what you run',
    time: '30 min',
    have: 'A box full of apps and a memory of how they fit together.',
    run: ['docker ps --format "table {{.Names}}\\t{{.Ports}}"', 'grep -R "ports:" -n ~/apps/*/compose.y*ml'],
    get: 'One page: app, hostname, front port, state, health path.',
    detail:
      'You cannot make a deploy boring until the estate is written down. List every app, the hostname it answers on, the loopback port it publishes, whether it writes state, and where its health endpoint lives. This page becomes the registry.',
  },
  {
    id: 'install',
    n: '01',
    title: 'Install swapdock',
    time: '10 min',
    have: 'Docker Engine, Compose v2, nginx, and git on one Linux host.',
    run: ['bash scripts/package-dist.sh', 'sudo bash swapdock-dist/install-swapdock.sh swapdock-dist', 'swapdock --version'],
    get: 'A static binary, /srv/swapdock, the nginx snippets, and a sudoers entry.',
    detail:
      'The installer lays out the registry, state, backup and log directories. It installs the nginx snippet files and runs nginx -t, but it does not reload nginx or start a service until you ask it to.',
  },
  {
    id: 'first-app',
    n: '02',
    title: 'Describe one app',
    time: '15 min',
    have: 'One Compose service you are willing to learn on.',
    run: ['swapdock init blog', 'swapdock --registry . validate', 'swapdock --registry . render', 'swapdock --registry . doctor'],
    get: 'A validated TOML entry, a generated front door you can diff, and host checks that pass.',
    detail:
      'One file names the hostname, front port, slot, Compose directory and service, the port variable, and the health path. validate checks the registry; render shows the exact nginx config; doctor checks the host: compose file, image, ports, and healthcheck.',
  },
  {
    id: 'first-cutover',
    n: '03',
    title: 'Make the first cutover',
    time: '10 min',
    have: 'A validated app and a host that passes doctor.',
    run: ['sudo swapdock --registry . up blog --release <sha>', 'swapdock runs', 'sudo swapdock --registry . rollback blog'],
    get: 'A zero-downtime deploy, a run log you can read, and a rollback you have actually tested.',
    detail:
      'The first deploy starts the release on the free port of the app pair, gates it on health, moves nginx, drains, and stops the old container. The rollback drill is part of the path, not an afterthought: a rollback you have never run is not a rollback.',
  },
  {
    id: 'stateful',
    n: '04',
    title: 'Bring a stateful app',
    time: '15 min',
    have: 'An app with a SQLite file or a named volume.',
    run: ['# writes_state = true', 'swapdock --registry . validate', 'swapdock --registry . up shop --release <sha>'],
    get: 'A replace deploy that stops the old writer before the new one starts.',
    detail:
      'A stateful app must not run two containers against one file. The registry says writes_state = true, the validator refuses swap, and replace stops first, starts on the same port, then gates. A pre-deploy backup hook turns the risk into a snapshot.',
  },
  {
    id: 'recovery',
    n: '05',
    title: 'Practise recovery',
    time: '20 min',
    have: 'A deploy that has moved traffic at least once.',
    run: ['swapdock status blog', 'swapdock reconcile blog', 'swapdock verify blog --since 30m'],
    get: 'A journal you understand, a reconcile you trust, and an access-log verdict.',
    detail:
      'Kill a deploy between the nginx flip and the registry commit in a lab. Then read the pending record, run reconcile, and watch it refuse unless nginx, the image id, and health agree. This is the difference between a tool that guesses and one that proves.',
  },
  {
    id: 'estate',
    n: '06',
    title: 'Run the estate',
    time: 'ongoing',
    have: 'One app under swapdock and a repeatable deploy.',
    run: ['swapdock import --compose-dir ~/apps/shop --service web --name shop', 'swapdock adopt <legacy> --release <sha>', 'swapdock monitor'],
    get: 'Every app behind one front door, one run ledger, one console.',
    detail:
      'Import writes registry entries for Compose services without editing your files. Adopt takes a legacy container at the commit it is already running. Monitor watches ports, certificates, and disk; prune keeps the images a rollback needs.',
  },
]

export const STAGE_SUMMARY = STAGES.map((s) => ({
  n: s.n,
  title: s.title,
  time: s.time,
  get: s.get,
}))
