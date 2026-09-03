# Compose

Compose combines complete source UTXOs into one deterministic composition.

BCE preserves the physical geometry of every source UTXO.

## Purpose

Compose is used when multiple ordinal-bearing UTXOs should become one physical UTXO composition.

The operation does not reinterpret the internal geometry of a source UTXO.

Each source UTXO remains physically intact inside the resulting composition.

## Input model

Compose operates on ordered source UTXOs.

Each source may contain:

- one or more inscription IDs
- one or more physical satpoints
- existing internal offsets
- postage spans
- shared-satpoint inscriptions

The full source UTXO value is treated as the physical span being appended.

## Physical order

Compose is deterministic.

The physical order of source UTXOs determines the resulting offset sequence.

If source UTXOs have values:

    546
    1000
    546

then the resulting source boundaries are:

    0
    546
    1546
    2092

No default postage is used.

## Existing internal geometry

A source UTXO may already contain multiple physical inscription positions.

Compose does not flatten or reconstruct those positions.

It preserves the source geometry and relocates the complete source UTXO into the resulting composition.

## Semantic order

Semantic hierarchy is resolved before Core execution.

Verify may determine that semantic structure requires a particular physical order.

Compose receives that final ordered source geometry.

The Core does not interpret semantic levels or direction.

## Planning

`compose-plan` constructs the deterministic physical composition plan.

Planning determines:

- ordered source inputs
- resulting physical offsets
- resulting output value
- transaction structure

Planning does not represent execution truth.

## Execution validation

Immediately before `compose-build-psbt`, Execution Guard re-reads:

- the root source UTXO
- every additional source UTXO
- every payment input

from current Bitcoin Core and ord state.

Expected source value must match current UTXO value.

Expected inscription IDs must still exist in the required source UTXOs.

Payment inputs must contain no inscriptions.

## Shared satpoints

A source UTXO may contain multiple inscription IDs at one satpoint.

Such IDs remain distinct inscriptions but represent one physical position.

Compose counts the physical source UTXO value once.

Additional co-satpoint IDs do not create additional postage.

## PSBT construction

`compose-build-psbt` constructs the transaction only after successful live-state validation.

Witness input values are derived from current Bitcoin Core state.

Bitcoin Core scriptPubKey is authoritative.

Where BCE derives a script from an address, the derived script must match the current Core script exactly.

## Signing

BCE does not hold private keys.

The generated PSBT is signed by an external wallet.

Compose requires supported SegWit transaction inputs for transaction-ID commitment across signing.

## Transaction identity

The build result includes the unsigned transaction ID.

This value becomes the expected transaction ID for broadcast.

Signing must not change the committed transaction identity.

## Broadcast

`compose-broadcast` accepts the signed transaction or signed PSBT together with the expected transaction ID.

Before broadcast, BCE:

- resolves the final raw transaction
- decodes it
- verifies the decoded transaction ID
- rejects any mismatch
- broadcasts through Bitcoin Core JSON-RPC
- verifies the transaction ID returned by Bitcoin Core

## Result

A successful Compose operation produces:

- one composed ordinal-bearing UTXO
- preserved source geometry
- deterministic physical order
- preserved physical value
- a transaction whose identity matches the committed build result

---

Compose appends complete physical source geometry.

It does not invent, shrink or reinterpret postage.
