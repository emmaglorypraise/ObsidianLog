# Architecture Decision Records

This directory holds Architecture Decision Records (ADRs) — short documents that
capture a significant architectural decision, its context, and its consequences.
The Month 1 deliverables include "an ADR documenting finalized storage
decisions"; this is where those live.

We follow the lightweight [Michael Nygard format](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions).

## Index

- [0001 — Record architecture decisions](0001-record-architecture-decisions.md)
- [0002 — Encryption and nonces](0002-encryption-and-nonces.md)
- [0003 — Per-service chains and serialized writes](0003-per-service-chains-and-serialized-writes.md)
- [0004 — Workspace layout and storage abstraction](0004-workspace-layout-and-storage-abstraction.md)
- [0005 — Storage data model](0005-storage-data-model.md)
- [0006 — Sia integration (feature-gated, pinned SDK)](0006-sia-integration.md)
- [0007 — Indexer topology: hosted-default, bring-your-own-indexer](0007-indexer-topology.md) _(Proposed — Month 3)_

## Adding an ADR

Copy [`0000-template.md`](0000-template.md), increment the number, and set the
status to `Proposed`.
Once agreed, change it to `Accepted`. Superseding an old decision? Mark the old
one `Superseded by NNNN` and link both ways.
