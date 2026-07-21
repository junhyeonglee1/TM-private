# STEP 14 durable scheduler operations

STEP 14 adds an in-process scheduler backed by the same SQLite database and Railway Volume as TM. It does not call OpenAI and does not add user-facing assistant features.

## Approved policy

- The cloud-authenticated production process polls every 30 seconds.
- A lease lasts 5 minutes. An expired lease is recovered after the normal retry delay.
- A run gets at most 5 attempts, with delays of 30 seconds, 2 minutes, 10 minutes, and 1 hour.
- Exhausted or explicitly non-retryable work enters `dead_letter`.
- Repeating work is coalesced after downtime. At most the newest eligible occurrence runs; an occurrence older than 24 hours is recorded as `skipped` instead of being replayed.
- Timestamps are stored in UTC. Daily schedules are resolved with IANA timezone data and the production policy uses `Asia/Seoul`.
- Future user-facing notifications must respect quiet hours from 22:00 to 07:00 KST. Internal maintenance may run during quiet hours.

## Initial production jobs

| Job | Schedule | Effect | OpenAI cost |
| --- | --- | --- | --- |
| `scheduler.canary` | Hourly | Writes one idempotent scheduler effect record | None |
| `memory.maintenance` | Daily at 03:10 KST | Applies the approved STEP 13 expiry and source-deletion policy | None |

The effect record and the business effect are committed in one SQLite transaction. A crash before commit leaves no effect; a crash after commit leaves both the immutable effect and the succeeded run. Duplicate claims therefore cannot apply an effect twice.

## Monitoring

The authenticated `GET /api/v1/ops/status` response includes a `scheduler` object with job, queue, running, retry, dead-letter, latency, last-success, last-failure, and effect counts. `status` becomes `degraded` when a dead-letter exists or the oldest queued occurrence is more than five minutes late.

Railway logs emit a structured scheduler cycle event whenever work is scheduled, recovered, completed, skipped, or failed. No token, job result, memory text, or error body is written to the public API response.

## Backup and restore

All four scheduler tables are part of full exports, migration manifests, local backups, and encrypted remote backups. Restore copies the pre-restore scheduler ledger back over the selected snapshot after migration, so completed effects and attempt history cannot be rewound.

## Production verification

Run the following from `app` after deployment:

```powershell
& '.\scripts\verify-step14-production.ps1'
```

The script reads the bearer token from Windows Credential Locker and waits for schema 8, two enabled jobs, one successful canary effect, zero dead letters, and a verified schema 8 encrypted remote backup. It performs no OpenAI request and no business-data mutation.
