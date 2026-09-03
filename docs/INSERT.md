# Insert

Insert adds one or more external verified groups to an existing composition.

BCE preserves the physical geometry of both the existing composition and the inserted source UTXOs.

## Purpose

Insert is used when new ordinal-bearing physical groups should become part of an existing composition.

Two execution modes exist:

- Direct Append
- Split and Insert

The required mode depends on the requested physical insertion position.

## Input model

Insert operates on:

- one existing composition UTXO
- one or more external source UTXOs
- verified existing groups
- verified insertion positions
- current inscription placement
- payment inputs where required

Every external inserted source remains a complete physical UTXO input.

## Existing composition

The existing composition has an established physical order.

Insert may extend that structure, but existing groups are not arbitrarily reordered.

Their physical value and internal geometry remain intact.

## Inserted sources

Each inserted source UTXO may contain:

- one or more inscription IDs
- one or more physical satpoints
- existing internal offsets
- postage spans
- shared-satpoint inscriptions

Insert preserves the complete source geometry.

The source is not resized to fit the destination structure.

## Direct Append

Direct Append is the simplest Insert mode.

When all new groups belong after the existing physical composition, BCE can append them directly.

Conceptually:

    existing composition
            +
    inserted source groups
            =
    final composition

Direct Append requires one transaction.

## Split and Insert

Insertion between existing groups requires a dependent transaction flow.

The parent transaction separates the existing composition into the physical runs required around the insertion point.

The child transaction then combines:

- required existing parent outputs
- new inserted source UTXOs
- required payment state

into the final composition.

Conceptually:

    existing composition
            |
            v
        split parent
            |
            +------ existing run
            |
            +------ existing run

    existing runs + inserted sources
            |
            v
        insert child
            |
            v
      final composition

## Existing order preservation

Insert does not use insertion as a mechanism for arbitrary reordering.

Existing verified groups preserve their relative physical order.

For example:

    existing:
    A B C

A valid insertion may produce:

    A X B C

or:

    A B X C

but Insert does not reinterpret the request as:

    C X A B

Existing composition order remains stable.

## Insertion before the physical beginning

A group cannot be inserted before the protected physical beginning of the existing composition where doing so violates the operation geometry.

Invalid insertion requests are rejected during planning.

Semantic root position and physical UTXO origin remain separate concepts.

The insertion planner operates on verified physical groups.

## Physical boundaries

Insertion positions are derived from verified group boundaries.

BCE does not split an existing physical span merely to create an insertion point.

A valid insertion point must correspond to established physical geometry.

## Shared satpoints

Inserted source UTXOs may contain multiple inscription IDs sharing one satpoint.

These remain separate inscription identities but one physical position.

Their source UTXO value is counted once.

Existing composition ranges containing shared satpoints are also preserved without duplicate value accounting.

## Planning

Insert planning determines:

- insertion position
- existing groups that remain together
- new inserted groups
- whether Direct Append is possible
- whether Split and Insert is required
- parent transaction structure
- child transaction structure where required

Planning does not represent execution truth.

## Execution state

Immediately before `insert-build-psbt`, BCE resolves all external inputs.

This includes:

- the existing composition UTXO
- every inserted source UTXO
- every payment input

Current state is read from Bitcoin Core and ord.

## Existing composition validation

The current existing composition must still match the expected physical state.

Validation includes, where applicable:

- outpoint
- total value
- scriptPubKey
- inscription placement
- physical boundaries
- expected verified ranges

A stale or contradictory existing composition is rejected.

## Inserted source validation

Each external inserted source is independently validated.

Expected inscription IDs must still exist in the required source UTXO.

Current source value must match the claimed source span.

Shared-satpoint state is preserved.

A missing or moved expected inscription causes rejection.

## Payment validation

Payment inputs are validated independently from ordinal-bearing inputs.

They must:

- exist
- remain unspent
- match current Bitcoin Core value and script
- contain no inscriptions

An inscription-bearing payment input is rejected.

## Parent transaction

For Split and Insert, the parent transaction creates deterministic outputs representing the required existing runs.

Parent outputs are constructed from the current validated composition state.

The parent transaction has a deterministic unsigned transaction identity.

## Child transaction

The Insert child consumes two kinds of inputs:

- deterministic outputs from the newly constructed parent
- live external inserted source UTXOs

Parent-derived child inputs are internal deterministic state.

They do not require a second blockchain lookup.

External inserted sources remain grounded in the execution state resolved before construction.

## Parent output references

The child may reference:

- unsigned parent transaction ID
- exact parent output indices
- exact parent output values

This prevents reconstruction of parent state from an unrelated external claim.

## Signing order

For Split and Insert:

    parent
      |
      v
    child

The parent must be signed and broadcast before the child can be independently accepted by the Bitcoin network.

Direct Append has no dependent child transaction.

## Transaction identity

BCE commits transaction identity before broadcast.

For dependent flows, child construction uses the committed parent transaction identity.

The finalized signed transaction must match the expected transaction ID.

## Broadcast

`insert-broadcast` validates the transaction identity before submission.

Broadcast processing includes:

- resolving the final raw transaction
- decoding the transaction
- verifying its transaction ID
- comparing it with the expected transaction ID
- rejecting mismatches
- broadcasting through Bitcoin Core JSON-RPC
- verifying the transaction ID returned by Bitcoin Core

## Fees

Insert is one high-level operation.

A Split-and-Insert flow may contain parent and child transactions, but it remains one logical Insert request.

The caller supplies `primary_miner_fee_sats`. Split-and-Insert additionally requires `secondary_miner_fee_sats`; Direct Append rejects a secondary fee. BCE does not estimate either fee and uses the supplied amounts only to calculate payment change.


## Result

A successful Insert operation produces:

- the final composition with new physical groups included
- preserved existing group order
- preserved existing physical geometry
- preserved inserted source geometry
- one transaction for Direct Append
- or a deterministic parent/child flow for Split and Insert
- committed transaction identities for broadcast validation

---

Insert adds physical groups.

It does not invent space by modifying existing postage.
