# Extract

Extract removes one or more selected verified groups from an existing composition while preserving the remaining physical structure.

Unlike Split, Extract is selective.

Selected groups become independent outputs.

Unselected physical ranges remain part of the composition.

## Purpose

Extract is used when only part of a verified composition should be removed.

It preserves:

- selected group boundaries
- remaining physical geometry
- inscription placement
- postage
- physical value

The operation does not rebuild semantic structure from scratch.

It operates on verified physical ranges.

## Input model

Extract operates on:

- one existing composed UTXO
- one or more selected verified groups
- current physical boundaries
- current inscription placement
- payment inputs where required

The selected groups must already correspond to valid verified ranges.

## Selected groups

Each selected group defines a physical range.

For example:

    composition:

    0..546
    546..1546
    1546..2092

If the middle group is selected:

    extract:
    546..1546

then the selected output contains the complete 1000 sat physical range.

BCE does not shrink or reinterpret that span.

## Root extraction

Extract does not allow removal of the protected root group where the operation would violate the required remaining composition structure.

Invalid root extraction requests are rejected during planning.

The exact semantic root may exist at a non-zero physical offset.

Root semantics are resolved before the Extract Core operates on verified physical groups.

## Multiple extracts

Extract may remove more than one selected group in one high-level operation.

For example:

    existing groups:

    A
    B
    C
    D
    E

    selected:

    B
    D

The selected groups become independent outputs.

The unselected groups form remaining physical runs.

## Remainder runs

After selected groups are removed, BCE identifies contiguous unselected physical ranges.

These are remainder runs.

Example:

    existing:

    A B C D E

    selected:

      B   D

    remainder runs:

    A
    C
    E

If the remaining composition can be represented by one contiguous remainder run, no Recompose child transaction is required.

If multiple remainder runs must become one resulting composition again, BCE constructs a dependent Recompose child transaction.

## Recompose

Recompose is an internal dependent transaction mechanism used when Extract creates multiple remainder runs.

The parent Extract transaction produces:

- extracted outputs
- remainder outputs
- payment change where applicable

The Recompose child consumes only the required remainder outputs and combines them into the final remaining composition.

Recompose is not a separate high-level user operation.

It exists to complete the Extract operation deterministically.

## Parent and child dependency

When Recompose is required, the child transaction references deterministic outputs from the newly constructed parent.

The child uses:

- unsigned parent transaction ID
- exact parent output indices
- exact parent output values

These parent outputs are internal deterministic state.

They do not require a second live blockchain lookup before child construction.

External inputs remain subject to Execution Guard validation.

## Physical boundaries

Extract selections must align with valid physical boundaries.

A boundary is:

- a distinct inscription satpoint offset, or
- the end of the UTXO

Extract does not cut inside a physical span.

If boundaries are:

    0
    546
    1546
    2092

then:

    546..1546

is valid.

A range ending at an arbitrary intermediate offset is rejected.

## Shared satpoints

A selected range may contain a shared satpoint.

Multiple inscription IDs at one satpoint remain physically inseparable at that position.

They share one physical span.

Extract preserves all inscription IDs physically contained within the selected range.

Shared IDs do not duplicate postage or physical value.

## Planning

Extract planning determines:

- selected physical groups
- extracted outputs
- remainder runs
- whether Recompose is required
- parent transaction structure
- child transaction structure where required
- payment requirements

Planning does not represent execution truth.

## Execution validation

Immediately before `extract-build-psbt`, Execution Guard re-reads:

- the current composition UTXO
- current inscription placement
- current physical boundaries
- payment inputs

from Bitcoin Core and ord.

The current composition value must match the expected total value.

Expected inscriptions must still exist inside the required verified ranges.

Payment inputs must contain no inscriptions.

## Parent PSBT

The Extract parent PSBT is built only after successful live-state validation.

The composition input uses current Bitcoin Core:

- value
- scriptPubKey

Payment witnesses also use current Core state.

## Child PSBT

If Recompose is required, the child transaction is derived from the parent transaction just constructed.

The child does not trust an externally supplied reconstruction of the parent outputs.

It derives the required inputs from deterministic parent transaction data.

## Signing order

When Extract produces parent and child transactions, signing and broadcasting must preserve dependency order:

    parent
      |
      v
    child

The child cannot be mined independently of the parent outputs it spends.

## Transaction identity

The parent transaction has a committed unsigned transaction ID.

Dependent child construction references that exact parent transaction identity.

Broadcast validates the signed transaction against the expected transaction ID before submission to Bitcoin Core.

## Broadcast

`extract-broadcast`:

- accepts a signed PSBT or raw transaction
- resolves the final raw transaction
- decodes it
- verifies the transaction ID
- compares it with the expected transaction ID
- rejects mismatches
- broadcasts through Bitcoin Core JSON-RPC
- verifies the returned transaction ID

Known or already-accepted transactions may be treated as successful only when the expected transaction identity matches.

## Fees

Extract is one high-level operation.

If a Recompose child is required, the operation still represents one Extract request.

The caller supplies `primary_miner_fee_sats` for the parent. When Recompose is required, the caller also supplies `recompose_miner_fee_sats`. BCE does not estimate either fee; it only reserves the exact supplied amounts while calculating payment change.


## Result

A successful Extract operation produces:

- one independent output for each selected group
- preserved physical value for each extracted range
- preserved remaining composition geometry
- an optional deterministic Recompose child transaction
- transaction identities committed before broadcast

---

Extract removes selected verified physical groups.

It preserves everything that remains.
