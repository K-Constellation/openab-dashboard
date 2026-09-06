# Devin Session Usage Tracking

This change adds local Kubernetes ingestion for token metrics written by the
Devin CLI session database in each OpenAB agent pod.

The first release intentionally supports local clusters only. Cloud clusters
are detected and skipped because their pod-local databases are not available
to the dashboard collector. Cloud ingestion will be added separately.

Each imported assistant message is identified by its Devin session and message
identifier, so collector runs and database snapshots cannot double-count it.

The imported metrics are model-reported token usage. Devin's local cost and
credit fields are not used because they are currently recorded as zero.
