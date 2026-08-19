# Execution Guard

Execution Guard is the final state validation layer immediately before PSBT construction.

Its purpose is simple:

    BCE trusts no previous state as execution truth.

Plans, requests, verification results, cached data and application state may all describe a valid intended transaction.

They are still only claims.

Immediately before BCE builds a PSBT, every external input is re-read from current Bitcoin Core and ord state.

If the current state does not match the expected state, the operation is rejected.

## Position in the execution flow

The Execution Guard runs after planning and verification but before transaction construction.

    request / intent
          |
          v
        Spec
          |
          v
       Verify
          |
          v
    verified intent
          |
          v
    Execution Guard
          |
          v
    current Bitcoin state
          |
          v
     PSBT construction

Verification proves that an intended structure is coherent.

Execution Guard confirms that the physical state required to execute that intent still exists.

## Source of truth

Bitcoin Core is authoritative for Bitcoin UTXO state.

ord provides inscription and satpoint state.

BCE cross-checks both sources where their information overlaps.

Execution Guard does not fall back to:

- request values
- previous plan values
- cached UTXO state
- previous verification output
- application assumptions

If required live state is unavailable, BCE fails closed.

## Bitcoin Core validation

For each required external UTXO, BCE resolves current Bitcoin Core state.

Validation may include:

- outpoint existence
- unspent state
- current output value
- current scriptPubKey
- address and script consistency

A spent or missing outpoint is rejected.

A value mismatch is rejected.

A script mismatch is rejected.

## ord validation

BCE also resolves current inscription state through ord.

Validation may include:

- output indexing state
- inscription IDs present in the output
- inscription satpoints
- physical offsets
- shared satpoint state

The ord result is checked against the corresponding Bitcoin Core output.

Contradictory state is rejected.

## Current physical spans

Execution Guard reconstructs current physical inscription spans from distinct satpoint offsets.

For each physical position, BCE determines:

- the offset
- all inscription IDs sharing that offset
- the physical postage span extending to the next distinct boundary

The last physical span extends to the end of the UTXO.

Example:

    UTXO value: 1546

    offset 0
      IDs: A, B
      postage: 1000

    offset 1000
      ID: C
      postage: 546

A and B are two inscription IDs but one physical position.

The 1000 sat span must be counted once.

## Shared satpoints

Shared satpoints are treated as one physical unit.

Execution Guard preserves all inscription IDs observed at the physical position while preventing duplicate physical value accounting.

Therefore:

    multiple IDs
    same satpoint
    =
    one physical span

An expected inscription may be valid even when additional inscription IDs share the same satpoint.

The presence of an additional co-satpoint inscription does not create another physical span.

## Operation-specific validation

All four BCE PSBT construction paths use current execution state.

### Compose

Before `compose-build-psbt`, BCE validates:

- root source UTXO
- every additional source UTXO
- every payment input

Source values and witness data are derived from current state.

Expected inscription IDs must still exist in the required source UTXOs.

Payment inputs must contain no inscriptions.

### Split

Before `split-build-psbt`, BCE validates:

- the composition UTXO
- requested physical ranges
- expected inscription placement
- payment inputs

The current UTXO value and script are used for PSBT construction.

### Extract

Before `extract-build-psbt`, BCE validates:

- the composition UTXO
- selected extract ranges
- expected inscription placement
- payment inputs

If a dependent child transaction is required, child inputs derived from the newly constructed parent transaction do not require another chain lookup.

They are deterministic outputs of the parent transaction being constructed.

### Insert

Before `insert-build-psbt`, BCE validates all external inputs once:

- existing composition UTXO
- inserted source UTXOs
- payment inputs

For split-and-insert operations, child inputs originating from the newly constructed parent are deterministic internal state.

External inserted UTXOs remain validated against live chain state.

## Payment inputs

Payment inputs are separate from ordinal-bearing composition geometry.

Execution Guard requires payment inputs to be free of inscriptions.

This prevents an ordinary fee input from silently introducing inscription-bearing value into the transaction.

## Script validation

Bitcoin Core scriptPubKey is treated as authoritative.

Where BCE derives a script from an address, the derived script must match the current Bitcoin Core scriptPubKey exactly.

A mismatch is rejected.

This prevents an address string from replacing the physical script truth returned by Bitcoin Core.

## No fallback behavior

Execution Guard deliberately has no optimistic fallback.

If Bitcoin Core or ord cannot provide the required current state, BCE does not continue using older data.

If state has changed since planning or verification, BCE does not attempt to repair the operation automatically.

The caller must resolve the new state and create a new valid operation.

## Why this layer exists

Bitcoin state may change between:

- application planning
- verification
- wallet interaction
- PSBT construction

A previously valid request can therefore become stale.

Execution Guard reduces the trusted execution boundary to the smallest practical point:

    immediately before transaction construction

This does not replace BCE invariants.

It confirms that the physical inputs to which those invariants will be applied still match current Bitcoin reality.

## Failure model

Execution Guard rejects execution when required state is:

- missing
- spent
- stale
- contradictory
- unresolved
- physically inconsistent with the request

BCE prefers rejection over constructing a transaction from ambiguous or stale state.

---

Verification proves the intended geometry.

Execution Guard confirms current physical reality.

PSBT construction begins only after both conditions are satisfied.
