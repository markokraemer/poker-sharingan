# Published model cards

| model | training checkpoint | model card | Hugging Face |
|---|---|---|---|
| SHARINGAN-1 | C@2e10, 124.7 GB | [source](sharingan-1/README.md) | [weights](https://huggingface.co/markokraemer/sharingan-1) |
| SHARINGAN-2 | C@1.83e11, 169.9 GB | [source](sharingan-2/README.md) | [weights](https://huggingface.co/markokraemer/sharingan-2) |

Both use the `medium` action menu and `buckets-200`. Search is optional Rust code,
not part of the weight files. The public `sharingan-1` track uses C@2e10 without
search; it does not serve the full SHARINGAN-2 release.

Publish only the corresponding README after signing in locally with `hf auth login`:

```bash
hf upload markokraemer/sharingan-1 model-cards/sharingan-1/README.md README.md
hf upload markokraemer/sharingan-2 model-cards/sharingan-2/README.md README.md
```

These commands update model descriptions only. They do not upload weights, bucket
tables or checksum manifests. Verify the remote README content after publishing.
`MODEL_CARD.md` mirrors the SHARINGAN-1 card for existing repository links.
