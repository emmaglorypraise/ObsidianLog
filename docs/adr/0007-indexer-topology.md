# 0007 — Indexer topology: hosted-default, bring-your-own-indexer

- Status: Accepted <!-- see "Resolution (Month 3)" below for what shipped -->
- Date: 2026-07-13 (resolved 2026-09-01)

## Context

ObsidianLog archives to Sia through an **indexer** (`indexd`): the daemon that
holds a wallet, forms and pays storage contracts with hosts, and stores object
metadata. Apps connect to its Application API (default `:9982`) and authenticate
with a per-user `AppKey` (derived from the user's recovery phrase + our App ID);
see ADR-0006.

Whoever runs a given `indexd` instance funds the wallet that pays hosts for
the storage its users consume — but the Sia Foundation itself operates one
such instance as a product, [`sia.storage`](https://sia.storage), with a free
tier and paid plans covering the cost for its own users. So "who operates the
indexer" isn't only a question of who runs the daemon; it's a question of
whose wallet pays. The forces:

- **Onboarding friction.** Requiring every user to run `indexd` + PostgreSQL,
  sync the chain, and fund a Sia wallet is a hard barrier for most.
- **Custody & compliance.** Our audience is ops/DevOps teams; some (regulated
  shops) will *want* to control their own wallet, keys, and data path rather than
  trust an operator.
- **Economics.** The indexer's wallet pays for storage. If *we* operated the
  indexer, *we'd* pay hosts and have to meter/bill users; pointing at
  `sia.storage` instead means its own users' plans cover their storage, and
  self-hosting means each user pays directly through their own wallet — in
  neither case does ObsidianLog take on storage costs.
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

- **Users do NOT have to run their own indexer.** The default path points at
  `sia.storage`, the Sia Foundation's hosted indexer; users onboard as
  per-user app keys under its own quota/plan system, and run nothing —
  **and ObsidianLog operates no infrastructure or wallet to offer this**,
  since `sia.storage`'s own operator (not us) funds the storage its users
  consume. See "Resolution (Month 3)" below.
- **BYO-indexer is fully supported.** Compliance-sensitive users can point
  ObsidianLog at their own `indexd` (or a partner's). This is a configuration
  concern, not a code fork: `SiaConfig { indexer_url, bucket, app_key }` already
  treats the indexer as a config value, and `indexd` allows app keys across
  multiple indexers.
- **The security promise holds in every mode.** Because encryption is client-side
  under a user-held key, a hosted indexer operator (including us) is
  cryptographically unable to read archived logs. "We host the plumbing but cannot
  read your archives" is the intended posture.

What was explicitly **out of scope until Month 3**: a funded mainnet wallet,
quota/billing, and the hosted onboarding UX. None of this was required for
the Month 1 grant deliverable, which only needed the pipeline proven against
*an* indexer (self-hosted on the Zen testnet — see the Sia backend
integration test). Resolved in Month 3 by pointing "hosted" at `sia.storage`
instead of operating our own indexer — which made the funded-wallet and
quota/billing items moot for the default path (`sia.storage`'s own operator
owns them), leaving only the onboarding UX to build. See "Resolution
(Month 3)" below.

## Consequences

- **No user-run-indexer requirement**, so onboarding stays light for the common
  case while custody-focused users keep full control — neither audience is
  excluded.
- **Config, not architecture.** Because the indexer URL and app key are already
  config, adding hosted vs BYO selection (and later, wiring the ingest server to a
  Sia backend) is incremental. The ingest server currently builds only a
  `LocalBackend`; making its backend selectable is the concrete follow-up.
- **Operating a hosted indexer ourselves would have been a real commitment** —
  a funded wallet, contract maintenance, per-app-key quotas, monitoring —
  and Month 3 chose not to take it on for the default path. It remains a
  documented, deliberately deferred option; see "Future option:
  self-operating our own indexd" below.
- **Alignment with Sia's direction.** As of April 2026 the Foundation directs
  grants to "building on indexd" / "building with SDKs"; using the
  `sia_storage` SDK against `sia.storage` (or a self-hosted/third-party
  indexer for BYO) sits squarely in that lane either way.

## Resolution (Month 3)

Hosted-by-default means **defaulting `obsidianlog init`'s Sia backend to
[`sia.storage`](https://sia.storage)**, not operating our own `indexd`. Its
own docs settle the economics question this ADR originally left open: *"the
indexer operator forms and maintains contracts with storage providers and
pays for the capacity apps consume"* — each user has their own `sia.storage`
account and plan (50GB free tier, paid tiers beyond that), and that plan
pays for their storage. ObsidianLog operates no infrastructure and funds no
wallet to offer this.

The onboarding UX gap is closed by folding the existing indexer-approval flow
(previously a separate `cargo run --example onboard` step) directly into
`obsidianlog init`: choosing the Sia backend now runs the approval +
recovery-phrase registration inline, with the URL prompt defaulting to
`sia.storage`. Entering a different URL at that same prompt is how
bring-your-own-indexer (self-hosted or third-party) is chosen instead —
unchanged as a fully supported, first-class option for compliance-sensitive
users, per this ADR's original decision.

Quota/billing and funded-wallet operation are moot for the shipped
hosted-default path, since they're `sia.storage`'s responsibility, not ours.

### Future option: self-operating our own `indexd`

Not implemented, and not scheduled — documented here so it's a validated
option if a reason to pick it up ever comes up (e.g. removing the remaining
browser-approval step, or a future paid/compliance tier). What it would take:

- **A funded wallet** on the indexer we'd run, to pay storage-provider
  contracts on our users' behalf.
- **Per-app-key quotas**, via indexd's admin API (`PUT /quotas/{key}`,
  fields `maxPinnedData` / `totalUses` / `fundTargetBytes`) — indexd has no
  per-key *bandwidth* quota, only storage and account-creation limits.
- **Silent onboarding**, via indexd's pre-authorization mechanism
  (`POST /apps/preauthorized/keys`): an admin registers a client Ed25519
  pubkey as authorized to auto-approve connection requests up to a
  `totalUses` cap. A request carrying a valid pre-authorization proof is
  approved immediately — no browser, no human step, no recovery phrase —
  which is the one piece `sia.storage`'s user-approval model can't offer.

This is deferred, not untested: the plumbing already exists (the Docker
Compose `sia` profile runs `indexd` + Postgres self-hosted), and a follow-up
task verifies it with a real archive → retrieve → verify round trip against
the Zen testnet — so if this option is ever picked up, it starts from a
confirmed-working base rather than an open technical question.
