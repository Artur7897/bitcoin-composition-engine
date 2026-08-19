# Bitcoin Composition Engine (BCE)

Bitcoin Composition Engine (BCE) is a deterministic Bitcoin UTXO composition and verification
engine written in Rust.

It implements four operations:

- Compose
- Split
- Extract
- Insert

## Core principle

The Core understands only canonical Bitcoin geometry:

- inscription IDs
- offsets
- postages
- physical order
- source UTXOs


```text
Viewer / Composer
        ↓
Structural Spec
        ↓
Verify
        ↓
Canonical Core geometry
        ↓
PSBT
        ↓
External wallet signature

## Operations
### Compose

Combines ordered source UTXOs into one composition. Core calculates every new
offset from the supplied postages.

### Split

Completely dissolves a composition into independently spendable verified
groups.

### Extract

Removes one or more selected groups. If multiple remainder ranges are created,
Core builds a dependent Recompose child transaction.

### Insert

Adds one or more external groups. Appending requires one transaction; insertion
between existing groups uses a split parent and composition child transaction.

## Verification

BCE includes two verification commands:

verify-composition validates an existing composed UTXO.
verify-compose validates a future semantic composition and compares it with
the independently generated Core plan.

The semantic root does not need to be located at physical offset 0.

Structural Specs support:

levels A through J
direction + and -
multiple directional spaces
nested contiguous subtrees
deterministic capacity limits
legacy Case, Grid, Album and Suitcase templates
## Fees

Every high-level operation charges one fixed service fee:

1500 sats

Network fees are calculated separately from transaction input and output counts.

Credits are reserved for future use but are not implemented. Credit payment
requests are rejected.

There is no default postage.

## Build

Requirements:

Rust stable toolchain
Bitcoin Core JSON-RPC access
cargo build

The binary is created at:

target/debug/bce
## Test
cargo fmt --check
cargo check
cargo test

Current confirmed result:

60 passed
0 failed
## CLI commands
compose-plan
compose-build-psbt
compose-broadcast

split-plan
split-build-psbt
split-build-psbt
split-broadcast

extract-plan
extract-build-psbt
extract-broadcast

insert-plan
insert-build-psbt
insert-broadcast

verify-composition
verify-compose

Every command receives one JSON payload as its second command-line argument.

## Signing model

Core creates PSBTs and reports the required signing inputs.

Private keys remain outside the Core. Signing is performed by the user's
external wallet.

Dependent transactions must be signed and broadcast in order:

parent first
child second
## Production status

BCE should not be promoted to production deployment until:

Extract and Insert webapp APIs are integrated
all four operations are tested with real wallet signing
dependent parent/child broadcasts are tested end to end
migration is approved explicitly

See BCE_NOTES.md and the `docs/` directory for detailed architectural and operational documentation.
