# 0007 — Indexer topology: hosted-default, bring-your-own-indexer

- Status: Proposed <!-- launch-time decision; revisit and Accept in Month 3 -->
- Date: 2026-07-13

## Context

ObsidianLog archives to Sia through an **indexer** (`indexd`): the daemon that
holds a wallet, forms and pays storage contracts with hosts, and stores object
metadata. Apps connect to its Application API (default `:9982`) and authenticate
with a per-user `AppKey` (derived from the user's recovery phrase + our App ID);
see ADR-0006.

The indexer is *not* a free, Foundation-hosted service — whoever runs `indexd`
funds the wallet that pays hosts. So once ObsidianLog is deployed and users
onboard, someone has to operate an indexer, and we must decide who. The forces:

- **Onboarding friction.** Requiring every user to run `indexd` + PostgreSQL,
  sync the chain, and fund a Sia wallet is a hard barrier for most.
- **Custody & compliance.** Our audience is ops/DevOps teams; some (regulated
  shops) will *want* to control their own wallet, keys, and data path rather than
  trust an operator.
- **Economics.** The indexer's wallet pays for storage. A hosted indexer means
  *we* pay hosts and meter/bill users; self-hosting means each user pays directly.
- **What `indexd` supports.** A single `indexd` instance is multi-tenant: one
  wallet serves many apps/users, with **per-app-key quotas and data limits** and
  cryptographic isolation between users (different keys → no cross-access). App
  keys can also target more than one indexer.
- **Our security model (ADR-0002).** Log content is sealed with a user-held
  AES-256-GCM key *before* it reaches the SDK. The indexer only ever sees
  metadata and ciphertext — never plaintext, never the AES key — regardless of
  who operates it.

## Decision

We will support a **hybrid indexer topology, hosted-by-default with
bring-your-own-indexer (BYO) as a first-class option**, and defer standing up the
hosted offering to **Month 3 (Launch & Ecosystem Integration)**.

- **Users do NOT have to run their own indexer.** The default path points at an
  ObsidianLog-operated indexer; users onboard as per-user app keys with quotas,
  and run nothing.
- **BYO-indexer is fully supported.** Compliance-sensitive users can point
  ObsidianLog at their own `indexd` (or a partner's). This is a configuration
  concern, not a code fork: `SiaConfig { indexer_url, bucket, app_key }` already
  treats the indexer as a config value, and `indexd` allows app keys across
  multiple indexers.
- **The security promise holds in every mode.** Because encryption is client-side
  under a user-held key, a hosted indexer operator (including us) is
  cryptographically unable to read archived logs. "We host the plumbing but cannot
  read your archives" is the intended posture.

What is explicitly **out of scope until Month 3**: operating a funded mainnet
wallet, quota/billing, uptime/SLA, and the hosted onboarding UX. None of this is
required for the Month 1 grant deliverable, which only needs the pipeline proven
against *an* indexer (self-hosted on the Zen testnet — see the Sia backend
integration test and its runbook).

## Consequences

- **No user-run-indexer requirement**, so onboarding stays light for the common
  case while custody-focused users keep full control — neither audience is
  excluded.
- **Config, not architecture.** Because the indexer URL and app key are already
  config, adding hosted vs BYO selection (and later, wiring the ingest server to a
  Sia backend) is incremental. The ingest server currently builds only a
  `LocalBackend`; making its backend selectable is the concrete follow-up.
- **Operating a hosted indexer is a real commitment** we take on deliberately at
  launch: a funded wallet, contract maintenance, per-app-key quotas, and
  monitoring. Treated as a Month 3 milestone item, not implied by shipping the
  library.
- **Alignment with Sia's direction.** As of April 2026 the Foundation directs
  grants to "building on indexd" / "building with SDKs"; a hosted-plus-BYO indexd
  integration sits squarely in that lane.
- **To revisit at Month 3:** move this ADR to `Accepted` (or supersede it) once
  the hosted/BYO split is implemented and the economics (who funds mainnet
  storage, how users are metered) are settled.
