# Devin Session Usage Tracking

This change adds local Kubernetes ingestion for token metrics written by the
Devin CLI session database in each OpenAB agent pod. The collector reads
`/home/node/.local/share/devin/cli/sessions.db` from the configured `openab`
container by default, with per-pod overrides available in `config.yaml`.

The first release intentionally supports local clusters only. Cloud clusters
are detected and skipped because their pod-local databases are not available
to the dashboard collector. Cloud ingestion will be added separately.

Each imported assistant message is identified by its Devin session and message
identifier, so collector runs and database snapshots cannot double-count it.

The imported metrics are model-reported token usage. Devin's local cost and
credit fields are not used because they are currently recorded as zero.

`total_tokens` is `input_tokens + output_tokens`. Cache read and cache creation
token counts are retained in event metadata instead of being added to the total.
