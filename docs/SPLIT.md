# Split

Split dissolves a verified composition into independently spendable physical groups.

BCE preserves the physical value and boundaries of every verified group.

## Purpose

Split is used when an existing composed UTXO should be separated into its verified groups.

Unlike Extract, Split operates on the complete verified composition.

Every verified group becomes an independent output UTXO.

## Input model

Split operates on:

- one existing composed UTXO
- verified physical groups
- current inscription placement
- current physical boundaries
- payment inputs where required

The composition may contain:

- multiple inscription IDs
- multiple physical satpoints
- shared-satpoint inscriptions
- groups spanning several adjacent physical units

## Verified groups

Split does not invent its own semantic grouping.

Verify translates the Structured Spec into flat physical groups.

Each group defines a physical range inside the composition.

A group may contain one or more adjacent physical spans.

For example:

    physical boundaries:
    0
    546
    1546

A verified group may cover:

    0..1546

even though it contains more than one physical span.

## Physical boundaries

Group boundaries must align with real physical boundaries.

A valid boundary is:

- a distinct inscription satpoint offset, or
- the end of the UTXO

Split must not cut inside a physical span.

For example, if the physical boundaries are:

    0
    546
    1546

then:

    546..1546

is valid.

But:

    546..1092

is invalid unless 1092 is itself a real physical boundary.

## Physical value preservation

Each split output preserves the complete physical value of its verified range.

BCE does not shrink postage to match semantic expectations.

If a group spans:

    546..1546

its physical value is:

    1000 sats

That value is preserved in the resulting output.

## Shared satpoints

Multiple inscription IDs may share one physical position.

Split preserves all inscription IDs physically contained in the selected range.

Shared IDs do not create additional value or duplicate boundaries.

A shared satpoint remains one physical span.

## Planning

`split-plan` constructs the deterministic split plan.

Planning determines:

- composition input
- verified group ranges
- resulting output ordering
- resulting output values
- payment requirements
- transaction structure

Planning is not execution truth.

## Execution validation

Immediately before `split-build-psbt`, Execution Guard re-reads:

- the composition UTXO
- its current inscription state
- its current physical boundaries
- payment inputs

from Bitcoin Core and ord.

The current total UTXO value must match the expected composition value.

Expected inscription placement must remain inside the required verified ranges.

Payment inputs must contain no inscriptions.

## PSBT construction

`split-build-psbt` constructs the split transaction only after successful live-state validation.

The composition witness uses current Bitcoin Core:

- output value
- scriptPubKey

Payment witnesses also use current Core values and validated scripts.

## Output structure

Each verified group becomes one independent output UTXO.

Output ordering follows the verified physical group order.

Split preserves:

- physical range value
- inscription placement relative to the resulting output
- physical ordering within each group
- shared-satpoint state

## Signing

BCE does not hold private keys.

The generated PSBT is signed by an external wallet.

Supported SegWit inputs preserve the committed transaction ID across signing.

## Transaction identity

The build result includes the unsigned transaction ID.

This becomes the expected transaction ID used during broadcast.

The finalized signed transaction must match that commitment.

## Broadcast

`split-broadcast`:

- accepts the signed PSBT or raw transaction
- resolves the final raw transaction
- decodes the transaction
- verifies the decoded transaction ID
- compares it with the expected transaction ID
- rejects mismatches
- broadcasts through Bitcoin Core JSON-RPC
- verifies the returned transaction ID

## Split and Extract

Split and Extract are different operations.

Split:

    separates the complete verified composition

Extract:

    removes selected verified groups
    while preserving the remaining composition

Selective removal should therefore use Extract rather than Split.

## Result

A successful Split operation produces:

- one independent output per verified group
- preserved physical group value
- preserved physical boundaries
- preserved inscription state
- deterministic output ordering
- a transaction matching the committed transaction identity

---

Split separates verified physical groups.

It does not invent new geometry.
