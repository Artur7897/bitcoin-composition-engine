# PSBT and Broadcast

BCE separates transaction construction, signing and broadcast.

Private keys never enter BCE.

The engine builds deterministic transaction structures, serializes them as PSBT where required, commits to transaction identity and validates the finalized transaction before broadcast.

## Execution sequence

The general flow is:

    verified intent
          |
          v
    Execution Guard
          |
          v
    PSBT construction
          |
          v
    unsigned transaction ID
          |
          v
    external wallet signing
          |
          v
    finalization
          |
          v
    transaction decode
          |
          v
    expected txid check
          |
          v
    Bitcoin Core broadcast
          |
          v
    returned txid check

Each stage has a distinct responsibility.

## PSBT construction

BCE constructs a PSBT only after the relevant external inputs have passed live-state validation.

PSBT construction uses current Bitcoin state for witness information including:

- input value
- scriptPubKey
- required transaction inputs
- deterministic output structure

Requests and previous plans are not accepted as authoritative witness state.

## External signing

BCE does not:

- store private keys
- derive user private keys
- sign user inputs
- manage wallet secrets

The PSBT is signed by an external wallet.

This keeps key custody outside the engine.

## Transaction identity commitment

For broadcast-capable flows, BCE records the transaction ID of the unsigned transaction created during PSBT construction.

This transaction ID becomes the expected transaction identity.

The caller must provide this expected transaction ID during broadcast.

The purpose is to bind the signed result to the transaction BCE originally constructed.

## SegWit requirement

Transaction-ID commitment across signing requires transaction forms whose txid is stable under witness signing.

BCE therefore requires supported SegWit inputs for flows that depend on unsigned transaction-ID commitment.

Unsupported non-SegWit inputs are rejected where signing could alter the committed transaction identity.

## Signed PSBT or raw transaction

Broadcast may receive:

- a signed PSBT
- or a finalized raw transaction

If a PSBT is supplied, BCE resolves the finalized raw transaction before broadcast.

The raw transaction is then decoded through Bitcoin Core.

## Decode before broadcast

BCE does not blindly submit signed transaction bytes.

Before broadcast it decodes the finalized transaction and obtains the actual transaction ID.

The decoded transaction ID must equal the expected transaction ID committed during construction.

If:

    decoded_txid != expected_txid

broadcast is rejected.

## Bitcoin Core broadcast

After transaction identity validation, BCE broadcasts using direct Bitcoin Core JSON-RPC.

BCE does not rely on an external `bitcoin-cli` process for broadcast.

The operation calls Bitcoin Core directly.

The transaction ID returned by Bitcoin Core must also equal the expected transaction ID.

If:

    returned_txid != expected_txid

the result is rejected.

## Known transactions

A transaction may already be known to Bitcoin Core or the network.

Broadcast handling may accept an already-known transaction state only when the transaction identity matches the expected committed transaction ID.

BCE never treats a different transaction as equivalent merely because it appears related to the same operation.

## Compose broadcast

Compose produces an unsigned transaction ID during PSBT construction.

`compose-broadcast` requires the expected transaction ID.

The finalized transaction is decoded and validated before Bitcoin Core broadcast.

## Split broadcast

Split follows the same commitment model.

`split-build-psbt` returns the unsigned transaction ID.

`split-broadcast` requires the matching expected transaction ID.

## Extract broadcast

Extract may involve a parent transaction and an optional dependent child transaction.

Each transaction has its own deterministic identity.

Dependent child construction references the exact unsigned parent transaction ID and output structure.

Broadcast order must preserve dependency:

    parent
      |
      v
    child

## Insert broadcast

Insert may operate as:

- Direct Append
- Split and Insert

Direct Append uses one transaction.

Split and Insert may create a parent and dependent child.

The child references deterministic outputs from the committed parent transaction.

Broadcast order is:

    parent
      |
      v
    child

## Parent and child identity

Dependent transaction construction must not reconstruct parent state from an unrelated external claim.

The child references:

- exact unsigned parent transaction ID
- exact parent output index
- exact parent output value

This binds the dependent flow to the parent transaction BCE actually constructed.

## Finalization responsibility

Signing is external.

Finalization may be performed by the wallet or resolved during BCE broadcast processing where supported.

Regardless of where finalization occurs, BCE validates the finalized transaction identity before network submission.

## Failure model

Broadcast fails closed.

BCE rejects broadcast when:

- expected transaction ID is missing
- finalized transaction cannot be resolved
- transaction decode fails
- decoded transaction ID differs from expected
- Bitcoin Core rejects the transaction
- Bitcoin Core returns a different transaction ID

There is no fallback to broadcasting an unverified transaction.

## Why transaction identity matters

A valid signature alone does not prove that the finalized transaction is the same transaction BCE intended to construct.

Transaction identity commitment provides an additional boundary between:

- deterministic BCE construction
- external wallet signing
- final network broadcast

The engine therefore verifies both:

    before broadcast:
    decoded_txid == expected_txid

and:

    after broadcast:
    returned_txid == expected_txid

## Separation of responsibilities

BCE:

- validates current physical inputs
- constructs deterministic transaction geometry
- builds PSBTs
- commits transaction identity
- validates finalized transaction identity
- broadcasts through Bitcoin Core

External wallet:

- controls private keys
- reviews transaction
- signs required inputs

Bitcoin Core:

- provides current UTXO truth
- decodes finalized transactions
- accepts or rejects network broadcast

---

BCE constructs.

The wallet signs.

Bitcoin Core validates and broadcasts.

The committed transaction identity must remain the same across all three stages.
