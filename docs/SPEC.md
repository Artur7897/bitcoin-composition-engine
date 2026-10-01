# Structured Spec

The Structured Spec describes structural composition.

BCE Core does not execute structural hierarchy directly. Spec and Verify translate structural composition into canonical physical UTXO geometry before Core execution.

The same UTXO can therefore be represented in two different ways:

- as a structured hierarchy
- as physical Bitcoin offset geometry

These are two representations of the same underlying UTXO state.

## One UTXO, two representations

### UTXO 1234 - Structured Spec

    Structured Spec


            A
            |
            |__________________B__________________B
                               |                  |
                    C-_________|_________C+       |__________C+__________C+


    Offset  0       546       1092       1638     2184       2730       3276


The Structured Spec describes hierarchy.

In this example:

- A is the structural root level.
- Two nodes exist on level B.
- The first B node has one C child in negative direction and one C child in positive direction.
- The second B node has two C children in positive direction.

Nodes on the same level do not need to have identical child arrangements.

A level defines structural depth. Relations, direction and capacity define which structures are permitted.

### UTXO 1234 - BCE Physical Geometry

    BCE Physical Geometry


            A         C          B          C          B          C          C
            |_________|__________|__________|__________|__________|__________|

    Offset  0         546        1092       1638       2184       2730       3276


This is the same UTXO shown as physical geometry.

BCE sees:

- physical offsets
- postage spans
- physical order
- inscription IDs
- verified groups and ranges

It does not need to understand the structural hierarchy itself.

## Why Verify translates direction

Direction is part of the structural relation.

Supported directions are:

- `+`
- `-`

Direction does not mean positive or negative satoshis and does not create negative offsets.

It defines on which physical side of a parent relation a child subtree belongs.

In the first B subtree above:

    C-  ->  B  ->  C+

The negative C child must appear physically before its B parent.

The positive C child must appear physically after its B parent.

Without direction, the hierarchy alone would not uniquely determine the physical order.

Verify therefore translates the structured relation into the deterministic physical sequence consumed by BCE.

The second B node demonstrates a different valid arrangement on the same level:

    B  ->  C+  ->  C+

Both nodes are level B, but their child structure differs.

This is intentional.

Level identifies structural depth, not a fixed object type or fixed physical arrangement.

## Structural primitives

The structural language is based on three primary concepts:

- `number`
- `level`
- `direction`

Higher-order concepts such as root, parent, child, relation, group, hierarchy, nesting and subtree are derived from these primitives.

## Levels

A level expresses structural depth.

Ordered identifiers may be represented as:

`A`, `B`, `C`, `D`, ...

These identifiers describe structural depth. They do not define a fixed global number of levels.

A level is not:

- an inscription type
- an offset
- a postage value
- a physical UTXO class

Terms such as `rootLevel`, `parentLevel` and `childLevel` describe roles assumed by levels within structural relations; they are not separate primitives.

## Number

`number` provides deterministic distinction and ordering among structural instances within the same level.

For example, `B1`, `B2` and `B3` represent three distinguishable B-level instances.

The level identifies structural depth; the number identifies the ordered instance within that level.

A numbered structural instance may also be described as a group.

Verify translates the resulting logical structure into physical BCE geometry.

## Structural specification and instance

The Structured Spec defines the permitted structural grammar.

A structural instance is one concrete realization of that grammar.

For example:

    Structural Spec
    = permitted structural grammar

    Structural Instance
    = concrete realization of that grammar

A grammar such as `A -> B` and `B -> C` may therefore produce many valid concrete instances while preserving the same structural rules.

## Information preservation

Structural reduction may translate logical structure into deterministic physical geometry, but it must preserve the information required for deterministic downstream reconstruction.

Conceptually:

    logical structure
        |
        v
    deterministic reduction
        |
        v
    physical representation

The reduction must not silently discard structural information merely because BCE Core no longer needs that information for transaction execution.

Normalized structural information may therefore remain available to compatible resolvers and interpreters alongside the verified physical result.

## Root level

The Structured Spec defines a structural root level.

The structural root does not need to be located at physical offset 0.

Physical offset 0 is the beginning of the UTXO.

Structural root position is determined by the verified structure.

Therefore:

    physical UTXO origin != structural root

## Relations

A relation describes how one structural level may contain another.

A canonical relation may contain:

- `parentLevel`
- `childLevel`
- `direction`
- `maxChildren`

Example:

    {
      "parentLevel": "A",
      "childLevel": "B",
      "direction": "+",
      "maxChildren": 8
    }

The relation defines structural composition.

It does not itself define a Bitcoin offset.

Verify resolves the relation into physical placement.

## Nested structure

Relations may be nested across multiple levels.

For example:

    A
    |
    B
    |
    C

A B-level node may itself contain C-level children.

Each parent-child relation is evaluated independently.

This allows nodes on the same level to participate in different valid nested arrangements while preserving one common structural grammar.

## Capacity

Relations may define deterministic child limits using `maxChildren`.

Capacity is a structural constraint.

It does not change:

- postage
- satpoints
- physical boundaries
- UTXO value

Capacity is validated before the structure reaches BCE Core execution.

## Spec and Verify

Spec defines the structural grammar.

Verify applies that grammar to actual or intended UTXO state.

Verify translates:

- structural root
- levels
- relations
- direction
- nested subtrees
- capacity

into:

- deterministic physical order
- verified groups
- verified ranges
- canonical offset and postage geometry

The normalized canonical grammar is preserved in the verification response as `validatedSpecs`. This retains `version`, `rootLevel`, every structural relation, direction and `maxChildren` without forwarding application or presentation metadata.

BCE Core then operates only on the verified physical representation. It does not consume `validatedSpecs`, but downstream resolvers and interpreters may do so.

## Separation of responsibilities

Structured Spec describes hierarchy.

Verify translates hierarchy.

BCE Core executes physical Bitcoin geometry.

This separation allows the structural model to remain expressive without adding application-specific concepts to the Core engine.
