# Bitcoin Composition Engine (BCE)

Bitcoin Composition Engine (BCE) is a deterministic Bitcoin UTXO composition and verification
engine written in Rust.

It implements four operations:

- Compose
- Split
- Extract
- Insert

## Core principle

BCE separates structural description from physical execution.

The structural language is based on three primary concepts:

- NUMBER
- LEVEL
- DIRECTION

Higher-order concepts such as parent, child, group, hierarchy, nesting and subtree are derived from these primitives.

Verify reduces structural intent into deterministic physical geometry.

BCE Core executes the resulting linear representation using:

- order
- offset
- postage

Structural information required for deterministic downstream interpretation must be preserved even when BCE Core itself consumes only the reduced physical representation.

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
verify-compose validates a future structural composition and compares it with
the independently generated Core plan.

The structural root does not need to be located at physical offset 0.

Structural Specs support:

- ordered level identifiers such as A, B, C, ...
- numbered structural instances such as B1, B2, B3
- direction + and -
- multiple directional spaces
- nested contiguous subtrees
- deterministic capacity limits

A level expresses structural depth. Number distinguishes and orders instances within a level. Direction determines on which physical side of a parent a related child structure is linearized.

Terms such as rootLevel, parentLevel and childLevel describe roles assumed by levels within structural relations; they are not separate primitives.

## Information preservation

Structural reduction may translate logical structure into physical geometry, but it must preserve the information required for deterministic downstream interpretation.

BCE Core may consume only the reduced physical representation, while normalized structural information remains available to compatible resolvers and interpreters.

Observed on-chain information must not be silently discarded merely because it is not selected as an independent structural node.

## Network fees

Planning and verification are independent from transaction funding.

During PSBT construction, BCE estimates transaction size from the concrete input and output counts and applies the requested `fee_rate`.

The resulting network fee is used when balancing payment inputs, outputs and payment change.

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
