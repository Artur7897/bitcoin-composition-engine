# Shared Satpoints

Bitcoin inscriptions are identified by inscription IDs.

Physical position inside a UTXO is identified by satpoint offset.

These are different concepts.

Multiple inscription IDs may share the same satpoint.

## IDs and physical positions

An inscription ID identifies one inscription.

A satpoint identifies one physical position.

Therefore:

    multiple inscription IDs
    may share
    one physical satpoint

This does not create multiple physical spans.

## Physical span accounting

Consider:

    UTXO value: 1546 sats

    offset 0
    IDs:
      A
      B

    offset 1000
    ID:
      C

The physical spans are:

    0..1000
    1000..1546

Therefore:

    offset 0
    postage 1000

    offset 1000
    postage 546

A and B share the first physical span.

The value of that span is counted once.

It is not:

    1000 sats for A
    +
    1000 sats for B

The correct physical value remains:

    1000 sats total

## Why this matters

If shared satpoints were treated as separate physical units, BCE could:

- double-count postage
- invent physical value
- derive false boundaries
- generate invalid ranges
- corrupt composition geometry

Shared satpoints must therefore be normalized into one physical position.

## Preserving inscription identity

Grouping physical position does not mean merging inscription identity.

A and B remain separate inscription IDs.

BCE must preserve the fact that both inscriptions exist.

Therefore:

    structural identity count != physical position count

The engine must keep both truths.

## Observed state

When BCE resolves current state, all inscription IDs observed at a satpoint should remain visible in the resolved state.

For example:

    offset: 0
    ids:
      - A
      - B
    postage: 1000

This representation preserves:

- both inscription identities
- one physical position
- one physical span

## Composition intent

Observed on-chain state and composition intent are different.

An additional inscription sharing a satpoint may physically exist without being selected as an independent structural node.

For example:

    observed state:
      A
      B

    composition intent:
      A

B still physically exists.

It must not be discarded from observed state.

At the same time, B does not automatically become an additional composition node merely because it shares the satpoint with A.

## Verification

Verify must preserve shared-satpoint truth while translating structural composition.

It must not:

- count the same physical span twice
- invent another boundary
- automatically promote every co-satpoint ID into the structural composition
- erase co-satpoint inscription information

The structural composition determines which IDs participate as selected nodes.

The observed physical state preserves everything that exists on-chain.

## Execution Guard

Immediately before PSBT construction, Execution Guard resolves current satpoints again.

Inscription IDs sharing one offset are grouped into one current physical unit.

For each distinct offset, BCE derives:

- all inscription IDs at that offset
- physical postage to the next distinct offset
- or postage to the end of the UTXO for the final span

This ensures current physical value is not duplicated.

## Compose

A source UTXO containing shared-satpoint inscriptions remains one complete physical source UTXO.

Compose preserves its internal geometry.

If the expected inscription is present on the shared satpoint, the existence of an additional co-satpoint inscription does not by itself invalidate the physical source.

## Split and Extract

Split and Extract operate on verified physical ranges.

A range containing a shared satpoint contains the complete physical span associated with that position.

BCE cannot split two inscription IDs that occupy exactly the same physical satpoint into separate physical spans unless Bitcoin itself provides distinct physical boundaries.

## Insert

An inserted source UTXO may also contain shared-satpoint inscriptions.

The source value is derived from the physical UTXO state.

Multiple IDs at one satpoint do not multiply the inserted physical value.

## Boundary derivation

Physical boundaries are derived from distinct offsets.

For example:

    IDs A and B -> offset 0
    ID C       -> offset 1000
    UTXO end   -> 1546

The boundaries are:

    0
    1000
    1546

Not:

    0
    0
    1000
    1546

Duplicate inscription positions do not create duplicate physical boundaries.

## Core principle

Shared satpoints demonstrate a fundamental BCE distinction:

    IDs identify inscriptions.
    Satpoints identify physical positions.

BCE preserves both.

It never converts multiple identities into multiple physical value when Bitcoin provides only one physical position.

---

Multiple IDs may occupy one satpoint.

They remain distinct inscriptions.

They share one physical span.
