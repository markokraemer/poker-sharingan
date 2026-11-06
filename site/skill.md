---
name: poker-sharingan
description: Build and evaluate poker agents locally with Poker Sharingan.
---

# Poker Sharingan: local research workflow

Read the repository README, docs/METHOD.md and docs/EVALUATION.md first.
Use the local engine, SDK and evaluation harness. For an API, self-host the
Rust inference server following sharingan-1/serve/README.md.
Do not assume a project-hosted endpoint is available.

Record the code revision, model checksum, action menu, bucket tables, seeds,
tracking options and sample sizes. Use identical seeds for comparisons and
report 95% confidence intervals. Distinguish raw from AIVAT scores, blueprint
from search, and SHARINGAN-1/C@2e10 from final SHARINGAN-2/C@1.83e11.

Both model releases have known profitable attacks. Read docs/CHECKPOINT.md and
docs/RESULTS.md before making strength or robustness claims. Never describe a
finite attacker evaluation as proof of unexploitable play. Do not launch paid
compute or publish/deploy a service without the user's authorization.
