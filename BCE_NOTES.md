# Bitcoin Composition Engine (BCE) Notes

Bitcoin Composition Engine (BCE) is a deterministic Bitcoin UTXO composition and transaction engine.

These notes describe implementation state, operational constraints and remaining integration work.

## Environment

Production integration remains separate from BCE development until explicit deployment approval.

BCE lives in:

    /home/ordifiadmin/dev/bitcoin-composition-engine

Current development binary:

    /home/ordifiadmin/dev/bitcoin-composition-engine/target/debug/bce

Production integration should not point to BCE until API migration and real wallet end-to-end tests are completed explicitly.

## Current status

BCE implements the four deterministic UTXO operations:

- Compose
- Split
- Extract
- Insert

The verification boundary is integrated through:

- verify-composition
- verify-compose

The complete automated suite currently passes:

58 passed
0 failed

cargo check and cargo build complete without warnings or errors.

BCE is functionally complete at the engine level. Remaining work belongs to integration,
wallet testing and controlled production migration.

## Architecture

OrdiFi uses progressively smaller languages:

Viewer
semantic, visual and interactive language

Composer
user intent and desired structure

Spec
root levels, relations, direction and capacity

Verify
semantic intent plus resolved on-chain state

Core
IDs, offsets, postages and physical order

Bitcoin
inputs, outputs and signatures

All layers describe the same intended result.

The Core deliberately understands no visual or application concepts.

Core does not understand:

cases
grids
albums
suitcases
pages
slots
reserved visual gaps
organizer applications
agent layers
## Canonical Core geometry

Core operates only on:

inscription IDs
offsets
postages
physical order
source UTXOs

The fundamental geometry rules are:

first_offset = 0
next_offset = current_offset + current_postage
total_value = last_offset + last_postage

Core never:

inserts a default postage
invents a missing offset
trusts Composer-generated transaction geometry
interprets rendering coordinates
rearranges visual slots

Postage must come from resolved UTXO values or verified offset boundaries.

## Structural Spec

spec.rs is the dictionary and grammar of the semantic structure language.

The canonical structure format is:

{
  "structure": {
    "version": 1,
    "rootLevel": "A",
    "relations": [
      {
        "parentLevel": "A",
        "childLevel": "B",
        "direction": "+",
        "maxChildren": 48
      }
    ]
  }
}

Supported concepts:

levels A through J
positive direction +
negative direction -
multiple directional spaces per level
deterministic child capacity
nested semantic subtrees

The semantic root does not have to be located at physical offset 0.

Legacy adapters remain available for the existing:

Case
Display Grid
Suitcase
Album

Visual fields and rendering coordinates are ignored by Spec and Verify.

## Verify

verify-composition validates an existing composed UTXO.

It verifies:

exact inscription membership
inscription outpoints
physical offsets
postage derived from boundaries
semantic root and parent relationships
positive and negative directions
maximum child counts
subtree contiguity
complete UTXO coverage

It translates the semantic tree into flat verified groups for:

- Split
- Extract
- Insert

verify-compose validates a future composition.

It:

reads the semantic intent tree,
reads every structural Spec,
verifies all source UTXOs independently,
preserves already composed subtrees,
derives the expected physical order,
invokes the existing Compose Core,
compares Core-generated offsets with the independently verified intent.

The Composer does not supply trusted transaction geometry.

## Compose

Compose combines ordered source UTXOs into one composition.

The physical first input begins at offset 0.

This physical root may differ from the semantic root. For example, a Case uses:

ORDINAL offset 0
CASE    offset ordinal_postage

Verify translates the semantic Case root into the required physical order before
the Core builds its plan.

## Split

Split completely dissolves a composition.

Each verified group becomes an independent output UTXO while preserving its
postage.

Selective removal is handled by Extract.

## Extract

Extract removes one or more selected non-root groups.

Each selected group becomes an independent output.

Unselected contiguous groups become remainder outputs.

If multiple remainder runs exist, Core builds a dependent Recompose child
transaction. If only one remainder run exists, no Recompose transaction is
required.

Multi-Extract is supported as one high-level operation.

## Insert

Insert adds one or more external groups to an existing composition.

Two modes exist:

Direct Append
Split and Insert

Direct Append requires one transaction.

Split and Insert creates a parent transaction for the required existing runs and
a child transaction that produces the final composition.

Existing groups cannot be reordered, and no group may be inserted before the
physical root.

## Parent and child transactions

Extract and Insert may require two dependent transactions.

The child references:

the unsigned parent transaction ID
exact parent output indices
exact parent output values

Signing and broadcasting must preserve dependency order:

parent first
child second

A parent/child package remains one high-level operation.

## Fees

Every high-level operation charges:

1500 sats service fee

This applies equally to:

- Compose
- Split
- Extract
- Insert

Network fees are calculated separately from the actual number of transaction
inputs and outputs.

Extract and Insert charge only one service fee even when a parent and child
transaction are required.

The future price of 75 credits is reserved, but credits are not implemented.
Credit payment requests are currently rejected.

There is no default postage.

## Known verification constraints

The structural verifier currently requires:

- every verified subtree to occupy a contiguous physical range where contiguity is required
- a maximum semantic depth of level J
- explicit child direction when a Spec permits both + and -
- every Compose source UTXO to represent a complete semantic subtree

Shared satpoints are valid Bitcoin state:

    multiple inscription IDs
    may share
    one physical offset

They must represent one physical position and one physical span while preserving all inscription identities.

Verify supports shared-satpoint observations by preserving all observed inscription IDs while deriving physical value from distinct satpoint positions.

Additional co-satpoint IDs do not automatically become semantic nodes.

This behavior is covered by regression tests.

## Remaining integration work

The BCE engine is functionally complete at the engine level. Remaining work is integration, deployment and real wallet end-to-end testing.

Before production migration:

- build the Extract webapp API
- build the Insert webapp API
- migrate webapp payloads without default postage assumptions
- test Compose, Split, Extract and Insert with real wallet signing
- test dependent parent/child broadcasts end to end
- confirm failure and rebroadcast behavior
- switch production integration to BCE only after explicit approval