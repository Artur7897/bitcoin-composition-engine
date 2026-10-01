# BCE Invariants

Bitcoin Composition Engine (BCE) is built around a small set of physical invariants.

These invariants define the execution model of the engine and are intentionally independent of application-specific semantics.

## 1. Bitcoin is the physical source of truth

BCE treats the current Bitcoin UTXO state as authoritative.

Requests, plans, specifications, cached data and previous verification results express intent or derived state. They are not execution truth.

Immediately before PSBT construction, BCE re-reads the relevant external inputs from Bitcoin Core and ord.

If the current on-chain state no longer matches the expected state, the operation is rejected.

## 2. Postage is never invented

BCE does not use a default postage.

Postage must originate from real UTXO value or from verified physical boundaries inside an existing UTXO.

BCE never silently creates, replaces or normalizes postage values.

## 3. Offsets follow physical boundaries

Offsets describe physical positions inside a UTXO.

For consecutive physical spans:

    next_offset = current_offset + current_postage

A valid physical boundary is defined by:

- a distinct inscription satpoint offset, or
- the end of the UTXO.

BCE does not cut a physical span at an arbitrary intermediate offset.

## 4. Physical value is conserved

Ordinal-bearing value is preserved through BCE operations.

Physical spans are moved, grouped or separated without shrinking or inventing the value that carries them.

Transaction fees are handled separately from ordinal-bearing composition geometry.

## 5. Compose preserves source geometry

Compose combines complete source UTXOs in deterministic physical order.

Existing physical structure inside a source UTXO is preserved.

The engine does not reinterpret or rewrite internal source geometry during compose.

## 6. Multiple inscription IDs may share one satpoint

Inscription IDs identify inscriptions.

Satpoints identify physical positions.

Multiple inscription IDs may therefore occupy the same physical satpoint.

Such inscriptions remain distinct IDs, but they represent one physical position and one physical span.

BCE must never double-count postage or value because multiple IDs share the same satpoint.

## 7. Structural composition is translated before Core execution

BCE Core operates on canonical physical geometry:

- inscription IDs
- UTXOs
- offsets
- postages
- physical order
- verified groups and ranges

Higher-level structural rules are interpreted by Spec and Verify before they reach the Core operation layer.

The Core does not require application-specific meaning in order to execute a physical UTXO operation.

## 8. Execution state is validated immediately before PSBT construction

Every external input used by a `*-build-psbt` operation is validated against current Bitcoin Core and ord state.

This includes, where applicable:

- outpoint existence
- unspent state
- UTXO value
- scriptPubKey
- address/script consistency
- inscription membership
- inscription satpoints
- physical span placement
- payment inputs containing no inscriptions

Unavailable or contradictory execution state causes rejection.

There is no fallback to stale request or cached state.

## 9. Signing does not change the committed transaction identity

BCE separates:

- transaction planning
- PSBT construction
- external signing
- transaction finalization
- broadcast

For broadcast-capable flows, BCE commits to the unsigned transaction ID produced during PSBT construction.

Before broadcast, the finalized transaction is decoded and its transaction ID is compared with the expected transaction ID.

A mismatch is rejected.

## 10. BCE fails closed

When required physical state cannot be resolved or validated, BCE rejects the operation.

It does not:

- estimate missing offsets
- invent postage
- infer missing physical boundaries
- continue with contradictory on-chain state
- silently substitute stale execution data

The engine prefers rejection over ambiguous execution.

---

These invariants are the foundation of Compose, Split, Extract and Insert.

Higher-level verification may add structural constraints, but it must not violate the physical invariants defined here.
