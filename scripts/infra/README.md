# scripts/infra: examples tied to one specific cloud setup

These scripts are what we used to rent, bootstrap, archive and tear down bare-metal servers on one cloud provider (Scaleway Elastic Metal + S3-compatible
object storage). They are kept as **examples**, not as a supported tool: they only make sense with that provider's CLI (`scw`), its rescue mode and a
private bucket. All account ids, project ids, server ids, IP addresses, bucket names and credentials files were removed; everything is parameterised through environment variables.

| path | what it does | needs |
|---|---|---|
| `inject_key.sh SERVER_ID ZONE IP` | installs an SSH public key on a freshly installed Elastic Metal server via rescue mode | `scw` CLI, `SCW_ORG_ID`, `SCW_PROJECT_ID`, API credentials, `sshpass` |
| `archive/archive.py` | uploads big files from a box to a private bucket without putting keys on the box (the controller presigns per-part PUT URLs) | `SCW_ACCESS_KEY`, `SCW_SECRET_KEY`, `ARCHIVE_BUCKET`, `S3_ENDPOINT`, `S3_REGION`, `PLURIBUS_HOST_{A,B,D,DEV}` |
| `archive/arch` | thin wrapper for `archive.py` | the same environment |
| `archive/upload_parts.sh` | runs on the box: streams parts to the presigned URLs | |
| `archive/finish_*.sh` | wait for an upload, verify sizes, then delete the server (destructive: read before use) | `scw` CLI, `SCW_ORG_ID`, `SCW_PROJECT_ID` |
| `finish-c/` | watcher that waits for a training run to reach its target, evaluates it on the box, archives and deletes the server | everything above |

Never commit `archive/state/` (job records contain presigned URLs; it is git-ignored). Host addresses are not hard-coded anywhere: set `PLURIBUS_HOST`, `PLURIBUS_DEV_HOST`
(see `scripts/remote.sh`, `scripts/dev.sh`, `scripts/eval_checkpoint.sh`).
