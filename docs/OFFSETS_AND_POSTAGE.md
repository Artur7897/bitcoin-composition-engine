# Offsets and Postage

Offsets and postage define the physical geometry BCE operates on.

BCE does not invent these values.

They are derived from actual UTXO value and real inscription positions.

## Offsets

An offset is a physical position inside a UTXO.

Offset 0 is the beginning of the UTXO.

For consecutive physical spans:

    next_offset = current_offset + current_postage

Offsets therefore form a deterministic physical sequence.

## Postage

Postage is the physical value of a span.

For a span beginning at offset `n`, its postage extends until the next distinct physical boundary.

For the final span, postage extends until the end of the UTXO.

Example:

    UTXO value: 2092 sats

    offset 0
    postage 546

    offset 546
    postage 1000

    offset 1546
    postage 546

Therefore:

    0 + 546 = 546
    546 + 1000 = 1546
    1546 + 546 = 2092

The total physical value is preserved.

## No default postage

BCE has no default postage.

A missing or invalid postage value is not replaced with a guessed constant.

Postage must be derived from:

- the source UTXO value, or
- verified physical boundaries inside an existing UTXO

If postage cannot be resolved safely, BCE rejects the operation.

## Physical boundaries

A valid physical boundary is:

- a distinct inscription satpoint offset, or
- the end of the UTXO

BCE does not create arbitrary intermediate boundaries.

For example:

    physical boundaries:
    0
    546
    1546

Valid ranges may include:

    0..546
    546..1546
    0..1546

The following is not a valid physical range:

    546..1092

unless 1092 is itself a real physical boundary.

## Empty sats inside a physical range

A valid physical range may contain empty sats between inscription positions.

This does not make the range invalid.

The important rule is that the start and end of the range align with real physical boundaries.

BCE operates on physical spans, not on the assumption that every sat inside a span contains an inscription.

## Shared satpoints

Multiple inscription IDs may exist at one physical offset.

This does not create additional postage.

Example:

    offset 0
    IDs: A, B
    postage 1000

    offset 1000
    ID: C
    postage 546

A and B share one physical span.

The first span is still 1000 sats, not 2000 sats.

## Physical units and semantic groups

A physical unit is defined by distinct physical boundaries.

A semantic group may contain one or more adjacent physical units.

For example:

    physical units:

    0..546
    546..1546

A verified semantic group may cover:

    0..1546

This is valid because the group begins and ends on real physical boundaries.

Semantic grouping may combine physical units.

It may not create a new physical boundary in the middle of an existing span.

## Compose

Compose preserves complete source UTXO geometry.

When source UTXOs are appended into one new composition, their internal physical spans remain intact.

A source UTXO is not silently resized to fit a semantic structure.

Existing offsets become relative to the source position inside the resulting composition.

## Split

Split separates verified physical groups into independent output UTXOs.

Each resulting output preserves the complete physical value of its selected range.

Split does not shrink the final span of a group merely because another semantic node is expected nearby.

## Extract

Extract removes one or more verified physical ranges.

Selected ranges become independent outputs.

Remaining contiguous physical ranges are preserved as remainder outputs.

If multiple remainder ranges must be recombined, BCE may create a dependent Recompose transaction.

## Insert

Insert adds new physical groups into an existing composition.

Appending preserves the existing composition and adds new source geometry after it.

Insertion between existing groups may require a parent split transaction followed by a child composition transaction.

Existing physical groups are not resized or reordered to create space.

## Value conservation

For ordinal-bearing composition geometry:

    input physical value = resulting physical value

BCE may change which output contains a physical span, but not the value of that span.

Network fees and service fees are accounted for separately from ordinal-bearing physical geometry.

## Why postage is fundamental

Offsets alone identify positions.

Postage defines the physical extent between positions.

Together they allow BCE to:

- preserve physical boundaries
- reconstruct groups
- split compositions
- extract ranges
- insert new groups
- maintain deterministic reversibility

Without preserved postage, an offset sequence would not define complete physical geometry.

---

Offsets define where a span begins.

Postage defines how far that span extends.

BCE preserves both.
