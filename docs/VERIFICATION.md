# Verification

Verification translates structural composition into canonical physical BCE geometry.

It sits between Structured Spec and the Core operation layer.

Verification does not replace Bitcoin as execution truth.

It determines whether a structural composition is physically coherent and produces verified groups and ranges that BCE can consume.

## Two verification modes

BCE provides two verification paths:

- `verify-composition`
- `verify-compose`

They solve different problems.

### verify-composition

`verify-composition` validates an existing composed UTXO.

It compares:

- the Structured Spec
- structural intent
- resolved on-chain inscription state
- physical offsets
- physical boundaries
- subtree structure

The result contains both the verified physical representation and the normalized canonical Structural Specs in `validatedSpecs`.

The Core operations consume the physical representation. Resolvers and interpreters may consume `validatedSpecs`; BCE does not discard it merely because the Core does not use it.

### verify-compose

`verify-compose` validates a future composition before execution.

It:

- reads the intended structural hierarchy
- reads the relevant Structured Specs
- resolves the source UTXOs
- preserves existing physical subtrees
- derives the expected physical order
- invokes the Compose planning layer
- compares the independently verified structure with the generated Compose geometry

The structural layer therefore does not directly supply trusted transaction geometry.

## Verification boundary

Verification operates on both structural and physical information.

Structural information may include:

- root level
- levels
- parent-child relations
- direction
- capacity
- nested subtrees

Physical information may include:

- inscription IDs
- UTXOs
- satpoints
- offsets
- postage spans
- physical order
- physical boundaries

Verify translates between these domains.

The Core operation layer receives the physical result.

## Structural primitives

Verification operates on a structural language based on three primary concepts:

- `number`
- `level`
- `direction`

Level expresses structural depth. Number distinguishes and orders structural instances within a level. Direction contributes to the deterministic physical placement of related child structures.

Higher-order concepts such as parent, child, relation, group, hierarchy, nesting and subtree are derived from these primitives.

## Information preservation

Structural reduction must preserve the information required for deterministic downstream interpretation.

Verify may reduce structural hierarchy into physical order, groups, ranges, offsets and postage geometry, but that reduction must not silently discard structural information required by compatible resolvers or interpreters.

Observed on-chain information must also be preserved even when an inscription is not selected as an independent structural node.

## Structural root and physical origin

The structural root does not need to be located at physical offset 0.

Physical offset 0 is the beginning of the UTXO.

The structural root is defined by the Structured Spec and validated against the physical arrangement.

Therefore:

    physical UTXO origin != structural root

Verify must preserve this distinction.

## Nested subtrees

A structural subtree must correspond to a coherent physical range.

Nested relations may create structures such as:

    A
    |
    B
    |
    C

Verify determines how each nested subtree maps onto physical order.

A subtree may contain multiple physical satpoints and multiple structural nodes.

The subtree must remain physically coherent according to the declared structural relations.

Interleaved subtrees are rejected when they violate the required contiguous structure.

## Direction

Direction is interpreted by Verify.

A negative child relation must resolve to physical placement before its parent relation.

A positive child relation must resolve to physical placement after its parent relation.

For example:

    C-  ->  B  ->  C+

The Core does not need to interpret `+` or `-`.

Verify converts direction into concrete physical order before Core execution.

## Levels

Levels describe structural depth.

Nodes on the same level may have different valid child arrangements.

For example, two B-level nodes may resolve as:

    C  ->  B  ->  C

and:

    B  ->  C  ->  C

if both arrangements satisfy the declared relations and directions.

Level equality therefore does not imply identical physical structure.

## Shared satpoints

Multiple inscription IDs may share one physical satpoint.

Verify must preserve this on-chain fact.

A shared satpoint represents:

- multiple distinct inscription IDs
- one physical position
- one physical span

Verification must not double-count physical value because more than one ID exists at the same satpoint.

At the same time, the existence of an additional inscription ID does not automatically make that ID part of the structural composition intent.

This requires a distinction between:

- observed on-chain state
- selected composition intent

Observed state should preserve all resolved inscription information.

Composition intent determines which structural nodes participate in the declared structure.

## Observed state and composition intent

Verification must not treat these two concepts as identical.

Observed state answers:

- which inscription IDs physically exist
- which satpoints they occupy
- which UTXO contains them
- which physical boundaries exist

Composition intent answers:

- which IDs are selected into the structural composition
- which relations apply
- which node is the structural root
- which groups should be exposed to BCE operations

An inscription may be physically present without becoming an independently selected structural node.

Information that exists on-chain must not be discarded merely because it is not selected by the current structural intent.

## Verified groups

Verification translates the structural hierarchy into flat physical groups.

A verified group may contain one or more physical spans.

Groups are used by BCE operations such as:

- Split
- Extract
- Insert

A group defines a verified physical range rather than an application-specific object type.

The Core operates on the resulting range and geometry.

## Physical boundaries

Verify may group multiple adjacent physical spans into one structural group.

However, group boundaries must remain aligned with real physical boundaries.

A valid boundary is:

- a distinct inscription satpoint offset, or
- the end of the UTXO

Verify must not create an arbitrary cut inside a physical span.

For example, if physical boundaries exist at:

    0
    546
    1546

then the following ranges may be valid:

    0..546
    546..1546
    0..1546

A range such as:

    546..1092

is not a valid physical boundary if offset 1092 does not exist as a physical boundary.

## Complete physical coverage

Verification must preserve the complete physical UTXO structure relevant to the operation.

It must not:

- invent missing spans
- shrink postage
- duplicate physical value
- discard resolved physical state
- create overlapping physical groups

Structural grouping may change how physical spans are interpreted, but it may not change the underlying Bitcoin geometry.

## Verification is not execution truth

A successful verification result is not sufficient authority for transaction execution.

State may change after verification.

Immediately before every external `*-build-psbt` operation, BCE uses the Execution Guard to re-read current Bitcoin Core and ord state.

Therefore:

    Spec
      |
      v
    Verify
      |
      v
    verified physical intent
      |
      v
    Execution Guard
      |
      v
    current Bitcoin truth
      |
      v
    PSBT construction

If current execution state contradicts the verified intent, BCE rejects the operation.

## Failure model

Verification fails closed.

It rejects structures that cannot be translated unambiguously into valid physical geometry.

Examples include:

- missing required structural nodes
- invalid parent-child relations
- invalid direction
- exceeded capacity
- broken nested subtree structure
- interleaved subtrees where contiguity is required
- invalid physical boundaries
- contradictory resolved state

Verification does not guess how an ambiguous structural composition should be mapped onto Bitcoin.

---

Spec defines structure.

Verify proves that the structure maps to valid physical geometry.

Execution Guard then confirms that the physical state still exists immediately before transaction construction.
