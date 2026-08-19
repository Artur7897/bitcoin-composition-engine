# Structured Spec

The Structured Spec describes semantic structure.

BCE Core does not execute semantic hierarchy directly. Spec and Verify translate semantic structure into canonical physical UTXO geometry before Core execution.

The same UTXO can therefore be represented in two different ways:

- as a structured hierarchy
- as physical Bitcoin offset geometry

These are two representations of the same underlying UTXO state.

## One UTXO, two representations

### UTXO 1234 — Structured Spec

    Structured Spec


            A
            |
            |__________________B__________________B
                               |                  |
                    C-_________|_________C+       |__________C+__________C+


    Offset  0       546       1092       1638     2184       2730       3276


The Structured Spec describes hierarchy.

In this example:

- A is the semantic root level.
- Two nodes exist on level B.
- The first B node has one C child in negative direction and one C child in positive direction.
- The second B node has two C children in positive direction.

Nodes on the same level do not need to have identical child arrangements.

A level defines structural depth. Relations, direction and capacity define which structures are permitted.

### UTXO 1234 — BCE Physical Geometry

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

It does not need to understand the semantic hierarchy itself.

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

## Levels

The canonical structural language supports levels A through J.

Levels represent semantic depth.

A level is not:

- an inscription type
- an offset
- a postage value
- a physical UTXO class

The letters exist only in the structural description.

Verify translates the hierarchy into physical BCE geometry.

## Root level

The Structured Spec defines a semantic root level.

The semantic root does not need to be located at physical offset 0.

Physical offset 0 is the beginning of the UTXO.

Semantic root position is determined by the verified structure.

Therefore:

    physical UTXO origin != semantic root

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

The relation defines semantic structure.

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

- semantic root
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

BCE Core then operates only on the verified physical representation.

## Separation of responsibilities

Structured Spec describes hierarchy.

Verify translates hierarchy.

BCE Core executes physical Bitcoin geometry.

This separation allows the semantic structure to remain expressive without adding application-specific concepts to the Core engine.
