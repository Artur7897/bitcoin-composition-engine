# BCE Architecture

Bitcoin Composition Engine (BCE) is a deterministic Bitcoin UTXO composition engine.

It separates structural composition, physical verification, execution-state validation and transaction construction into distinct layers.

The engine implements four high-level operations:

- Compose
- Split
- Extract
- Insert

The architecture is intentionally application-agnostic.

## System overview

BCE operates through a layered flow:

    structural intent
         |
         v
    Structured Spec
         |
         v
       Verify
         |
         v
    verified physical geometry
         |
         v
    Execution Guard
         |
         v
    current Bitcoin Core + ord state
         |
         v
    operation planning
         |
         v
    PSBT construction
         |
         v
    external wallet signing
         |
         v
    transaction finalization
         |
         v
    txid commitment validation
         |
         v
    Bitcoin Core broadcast

Each layer has one defined responsibility.

## Structured Spec

Structured Spec describes structural composition.

Its primary conceptual primitives are:

- `number`
- `level`
- `direction`

Higher-order concepts such as root, parent, child, relation, group, hierarchy, nesting, subtree and capacity are derived from these primitives.

Structured Spec does not define trusted transaction geometry.

It defines structural grammar that must still be translated and verified.

See:

    docs/SPEC.md

## Verify

Verify translates structural composition into physical geometry.

It resolves:

- structural hierarchy
- physical order
- valid groups
- valid ranges
- direction
- nested subtree placement

Verify produces the canonical physical representation required by BCE operations.

Verify also preserves the normalized canonical grammar in `validatedSpecs`. This allows resolvers and interpreters to consume the verified structural information even though the Core operation layer does not use it.

Application, presentation and other unrelated metadata are not included in `validatedSpecs`.

It does not replace Bitcoin as execution truth.

See:

    docs/VERIFICATION.md

## Canonical physical geometry

The Core operation layer understands physical Bitcoin structure.

Its canonical geometry includes:

- source UTXOs
- inscription IDs
- satpoints
- offsets
- postage spans
- physical order
- verified groups
- verified ranges

The operation layer does not require structural hierarchy once verification has completed.

## Offsets and postage

Offsets define physical positions inside a UTXO.

Postage defines the physical span extending from one valid boundary to the next.

For consecutive physical spans:

    next_offset = current_offset + current_postage

BCE does not invent default postage or synthetic physical boundaries.

See:

    docs/OFFSETS_AND_POSTAGE.md

## Shared satpoints

Multiple inscription IDs may occupy one physical satpoint.

They remain separate inscription identities but share one physical position and one physical span.

Physical value must therefore be counted once per distinct satpoint position, not once per inscription ID.

See:

    docs/SHARED_SATPOINTS.md

## Execution Guard

Execution Guard is the final live-state validation layer before PSBT construction.

It re-reads every external input from:

- Bitcoin Core
- ord

It validates current physical state rather than trusting previous request, plan, cache or verification state.

If current execution state no longer matches the intended operation, BCE rejects the operation.

See:

    docs/EXECUTION_GUARD.md

## Bitcoin Core and ord

BCE does not maintain its own blockchain index.

Bitcoin Core provides authoritative UTXO state.

ord provides inscription and satpoint state.

BCE reads both sources directly and cross-checks their overlapping physical information where required.

No external command-line wrapper is required for Bitcoin Core RPC.

BCE communicates through direct JSON-RPC.

## Operation planning

Each high-level operation has its own planning logic.

### Compose

Compose combines complete source UTXOs into one deterministic physical composition.

Existing source geometry is preserved.

### Split

Split dissolves a verified composition into independently spendable physical groups.

### Extract

Extract removes one or more selected verified groups.

Remaining physical ranges are preserved.

A dependent Recompose child transaction may be required when multiple remainder runs must be recombined.

### Insert

Insert adds one or more external groups to an existing composition.

Direct append may use one transaction.

Insertion between existing groups may require a parent transaction followed by a dependent child transaction.

## Parent and child transactions

Extract and Insert may create dependent transaction flows.

A child transaction may reference:

- the unsigned parent transaction ID
- specific parent output indices
- specific parent output values

Inputs produced by a just-built parent transaction are deterministic internal state.

They do not require another blockchain lookup before constructing the dependent child.

External inputs still require live-state validation.

## PSBT construction

BCE builds PSBTs after:

- structural verification
- physical geometry validation
- live execution-state validation

Private keys remain outside BCE.

External wallets are responsible for signing.

BCE does not store wallet secrets.

## Transaction identity commitment

For supported broadcast flows, BCE records the unsigned transaction ID produced during PSBT construction.

Signing must not change that committed transaction identity.

Before broadcast, BCE:

- finalizes or resolves the signed transaction
- decodes the raw transaction
- verifies the decoded transaction ID
- compares it with the expected transaction ID
- rejects mismatches
- broadcasts through Bitcoin Core JSON-RPC

The transaction ID returned by Bitcoin Core must also match the expected transaction ID.

## Bitcoin execution boundary

BCE deliberately minimizes the trusted execution boundary.

The sequence is:

    structural composition
        |
        v
    verified physical intent
        |
        v
    live Bitcoin state
        |
        v
    deterministic transaction construction

No earlier application state is accepted as physical execution truth.

## Network fees

BCE separates composition geometry from transaction funding.

Planning and structural verification are independent from network-fee calculation.

During PSBT construction, BCE determines the concrete input and output counts, estimates virtual size, and applies the requested `fee_rate`.

The resulting network fee is used when balancing payment inputs, outputs and payment change.

For parent-child flows, each transaction has its own network-fee calculation based on its own transaction shape.

Ordinal-bearing physical spans are preserved independently from:

- network fees
- payment change

Payment inputs are validated separately and must not contain inscriptions.

## Failure model

BCE fails closed.

If required physical state cannot be resolved or validated, the operation is rejected.

BCE does not:

- guess missing offsets
- invent postage
- invent physical boundaries
- continue with stale UTXO state
- silently repair contradictory execution state

## Module layout

The current Rust implementation separates responsibilities across modules including:

    bitcoin_rpc.rs
    execution_guard.rs

    spec.rs
    verify.rs
    verify_compose.rs

    compose_plan.rs
    compose_psbt.rs
    compose_broadcast.rs

    split_plan.rs
    split_psbt.rs
    split_broadcast.rs

    extract.rs
    extract_psbt.rs
    extract_broadcast.rs

    insert.rs
    insert_psbt.rs
    insert_broadcast.rs

    recompose_psbt.rs
    fees.rs
    models.rs

This module separation reflects the architectural boundaries described above.

---

Spec describes structure.

Verify translates structure into physical geometry.

Execution Guard confirms current Bitcoin reality.

BCE operations construct deterministic transactions from that validated physical state.
