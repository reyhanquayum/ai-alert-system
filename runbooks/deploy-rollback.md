# Deploy Rollback Procedure

Applies to: any service deployed through the standard pipeline.

## When to use

Any incident where the symptom onset correlates with a deploy — rolling back is
almost always faster than debugging forward under pressure.

## Steps

1. Identify the last known-good release: `deployctl history <service>`.
2. Roll back: `deployctl rollback <service> --to <release>`.
3. Watch error rate and latency for 10 minutes; confirm recovery.
4. Mark the bad release as blocked so auto-deploy doesn't re-promote it.
5. File a ticket linking the incident and the offending commit for follow-up.

## Notes

- Database migrations are NOT rolled back automatically — check whether the bad release shipped one before rolling back.
