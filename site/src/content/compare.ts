export const COMPARE = [
  ['Change routing', 'Edit nginx by hand and hope', 'Generated, validated, tested'],
  ['Deploy', 'SSH, compose up, watch', 'One command, health-gated'],
  ['Roll back', 'Find the old image, reconstruct', 'swapdock rollback'],
  ['SQLite apps', 'Downtime or corruption', 'Replace, enforced by rule'],
  ['Crash mid-deploy', 'Unknown state, guess', 'Journal, reconcile, refuse'],
  ['Audit', 'Shell history', 'JSONL run log, resumable'],
  ['Two apps at once', 'Port and config races', 'Locks, derived ports'],
] as const
