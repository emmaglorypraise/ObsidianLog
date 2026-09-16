# 0015 — One bundled credential item for single-prompt setup

- Status: Accepted
- Date: 2026-09-16

## Context

PR #58 cut `obsidianlog init`'s macOS keychain calls from six down to three
(four when the Sia backend is chosen) by removing genuinely redundant
round-trips. Even so, a fresh `init` still produces multiple separate macOS
authorization prompts — one per keychain call, not one per logical "set up
ObsidianLog" action — which reads as broken to a user even though each
individual call is doing real, justified work: today's design stores the
encryption key and the Sia app key as two independent keychain items, and
for each one has to check whether it already exists before deciding whether
to write it.

Trimming calls further cannot close this gap. Skipping a wasted early
existence check on the Sia key (a real, separate fix — see below) only
brings a fresh local install from three keychain calls down to two, and a
fresh Sia install from four down to three. Neither can reach one, because
the current design still performs two logically distinct operations — check
existence, then write — for at least one secret, and macOS treats a plain
write (`SecKeychainAddGenericPassword` via the `set_generic_password`
wrapper this project currently uses) as an implicit find-then-add-or-update:
reading `apple-native-keyring-store`'s and `security-framework`'s source
directly confirms `set_generic_password` performs its own internal
existence check before adding or updating, so it silently upserts and
never reports whether an item was already there. There is no way to safely
merge "does this exist" and "write it" through that API without either
losing the ability to detect an existing key (breaking the #72 guarantee
that a repair path must reuse an existing key rather than silently
rotating it) or accepting the two-call cost the upsert already pays
internally.

The lower-level `security-framework` API does, however, expose
`add_generic_password` directly — described in its own documentation as
adding a credential "without checking if it exists already" — separate from
`set_generic_password`'s find-then-add-or-update wrapper. Calling it
directly returns a specific, recognizable "duplicate item" error
(`errSecDuplicateItem`) when the item is already present, instead of
silently overwriting it. That is the mechanism this ADR is built on: a
single call that either creates a new item (the fresh case) or reports,
distinctly, that one is already there (the repair case) — never both a
check and a write.

Separately, storing the encryption key and the Sia app key as two
independent keychain items means a fresh Sia setup inherently needs at
least two credential-bearing operations no matter how each one is
optimized, since macOS keychain authorization is scoped per item. The only
way to reach one prompt for a fresh Sia install too is to store both
secrets as one item.

## Decision

We will store the encryption key and the optional Sia app key together as
one bundled credential item, replacing the two independent items
(`encryption-key`, `sia-app-key`) used today, and change how that item is
written on macOS.

**One bundled item.** The bundle holds the encryption key, an optional Sia
app key, and an internal format version, serialized together and stored
under a single new account name. The two old account names are simply
never read again by this version of the code — no migration, no active
deletion of the old items (see "No migration" below).

**macOS: create-only write, not check-then-write.** A fresh install calls
the lower-level create-only keychain operation directly. Success means the
item was created — one call, one prompt. Failure with the specific
duplicate-item error means the item already exists, and is the signal to
preserve it rather than overwrite it — still one call, and without
reintroducing the silent-rotation bug #72 fixed. This is scoped to macOS
only: Windows and Linux have different credential-store write semantics and
no evidence of the same multi-prompt problem, so they keep using the
existing check-then-write path for the bundle, gaining the one-item
simplification without a specific claim about matching macOS's single-call
behavior.

**Rotation is a read-modify-write.** Forced key rotation reads the current
bundle, replaces only the encryption-key field, and writes the result back,
leaving a previously stored Sia key untouched — preserving the existing,
already-tested guarantee that rotating the encryption key never disturbs a
stored Sia key.

**The "one prompt" guarantee is scoped precisely, not claimed universally.**
It covers a clean fresh install, and a repair (config file gone, credential
still present) where the local backend is re-chosen and nothing new is
being added. It does not cover a repair where Sia is chosen: with no config
file present, there is no way to know in advance whether an existing bundle
already holds a Sia key without actually reading it, so any repair where
Sia is chosen always goes through a read-then-update path, never the
create-only path, regardless of what the bundle turns out to actually
contain. A bare duplicate-item signal only proves something is there, not
what it contains, so safely adding or confirming a Sia key without
disturbing the preserved encryption key genuinely requires reading the
bundle first. This is intentional: that combination is rarer than the
plain local case, and correctness — never risking the loss or corruption of
the preserved encryption key — matters more than shaving its prompt count.

**The config file carries an explicit format marker, checked structurally.**
`Config` gains a new, optional field recording which credential-storage
format the file expects, following the same pattern already used for the
existing optional Sia-indexer settings field: a file missing the field
parses as legacy rather than failing at the parsing stage. The default,
in-memory configuration (used when there's no file at all) sets this field
to the current format number, so any freshly saved config always carries
it. The check itself lives inside the shared, strict config-loading
function used by `serve`, `query`, `verify`, and backend resolution — none
of those gain any special handling, and none of them may treat a legacy or
unrecognized config as anything but a hard failure before any keychain
access happens. This composes cleanly with how loading already works: the
"no file at all" fallback returns before any parsing happens, so it never
needs the check; only a file that was actually parsed does.

**Setup gets its own, more permissive loading path — used nowhere else.**
`init`'s entire purpose is handling exactly the states the strict path
refuses, so its hard failure cannot be the only way in for `init` too, or
there would be no route from "a legacy config was detected" back to "start
fresh." `init` uses a second, distinct loading function that returns one of
three named outcomes — no config file yet, a current-format config, or a
legacy config (carrying its parsed contents, not just a bare signal) —
sharing the same underlying parsing logic as the strict path rather than
duplicating it. On a legacy config, `init` prints exactly what continuing
would abandon and requires the same explicit confirmation flag already used
for ordinary key rotation before proceeding to a fresh setup, reusing that
flag's existing "yes, I understand this is destructive" meaning rather than
inventing a new one.

**No migration tool.** A config missing the format marker fails with a
specific, actionable message before any keychain access at all: stay on
the previous release to keep reading existing archives, or knowingly start
fresh and accept the encryption key rotates. A config carrying a marker
this version doesn't recognize (a future format) fails with a distinct
forward-compatibility message. This mirrors the precedent already
established in ADR-0009's `MANIFEST_VERSION` bump: fail loudly rather than
silently reinterpret an old format, with no automatic migration, on the
grounds that there are no known production deployments to migrate. Unlike
that internal manifest format, though, this one is hit directly by every
upgrading user rather than being a format almost nobody encounters
directly — so, unlike ADR-0009, it's worth a purpose-built message rather
than relying on a raw parsing failure.

**Breaking change, versioned accordingly.** This changes both the on-disk
config format and the local credential storage format, so it ships as
0.2.0 per this project's own rule that a breaking change bumps the minor
version pre-1.0, with the corresponding commit marker and a changelog
entry.

**A separate, independent fix ships alongside this:** the function that
resolves which credential store to use treated any keychain error,
including the user cancelling or denying an authorization prompt, as
"unavailable, fall back to a plain file." That's now narrowed to only fall
back on genuine unavailability, surfacing a cancellation or denial as a
real error instead of silently changing where a secret ends up. This fix
doesn't depend on anything else in this ADR and ships first, on its own.

## Consequences

- A clean fresh local install produces exactly one macOS keychain prompt; a
  clean fresh Sia install produces exactly one, separate from the Sia
  browser-approval step, which is unaffected by any of this.
- A plain re-run against an already-complete setup still performs one
  existence check against the bundle before declaring "already
  initialized" — this is a retained integrity check, not a removed one,
  catching a credential that was manually deleted from the keychain after
  setup. It should normally be silent once macOS trusts the binary; this
  ADR does not claim zero keychain calls on every re-run, only zero calls
  beyond what correctness already requires.
- A repair that also introduces new credential material (adding Sia to a
  previously local-only bundle) costs two keychain operations, not one, and
  is not claimed to be single-prompt. This is a deliberate scope boundary,
  not an oversight.
- Anyone who already ran `init` under the old two-item format must either
  stay on the previous release to keep reading archives created under it,
  or knowingly re-initialize and accept the encryption key rotates. There
  is no tooling to read the old format forward or convert it.
- The create-only write path is macOS-specific and adds a direct,
  explicit dependency on the lower-level macOS security library this
  project already depended on transitively; Windows and Linux keep using
  the existing general-purpose credential-store code path for the bundle.
- If Windows or Linux are later found to have a similar multi-prompt
  problem, closing it is a follow-up decision informed by each platform's
  actual write semantics, not an extension assumed by this ADR.
