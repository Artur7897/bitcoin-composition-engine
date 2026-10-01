# BCE CLI

The BCE command-line interface exposes the engine's planning, verification, PSBT construction, broadcast and diagnostic operations.

The binary name is:

    bce

Commands accept a JSON payload as the operation request and return JSON on success.

## Command overview

Current commands:

    inspect-current-state

    compose-plan
    compose-build-psbt
    compose-broadcast

    split-plan
    split-build-psbt
    split-broadcast

    extract-plan
    extract-build-psbt
    extract-broadcast

    insert-plan
    insert-build-psbt
    insert-broadcast

    tx-broadcast

    verify-composition
    verify-compose

## Command structure

The general invocation form is:

    bce <command> '<json-payload>'

Example shape:

    bce compose-plan '{"...":"..."}'

Exact request and response fields depend on the selected operation.

BCE returns serialized JSON results for successful commands.

## inspect-current-state

`inspect-current-state` resolves current physical state for one Bitcoin outpoint.

It is primarily a diagnostic command.

The request contains:

    outpoint

Conceptual request:

    {
      "outpoint": "<txid>:<vout>"
    }

The result may include:

- outpoint
- current UTXO value
- scriptPubKey
- address where available
- inscription IDs
- physical satpoint offsets
- derived postage spans

This command reads current Bitcoin Core and ord state.

It does not construct a transaction.

## compose-plan

`compose-plan` creates a deterministic Compose plan.

It determines the intended transaction geometry before PSBT construction.

Planning may include:

- ordered composition inputs
- source values
- physical output structure

A plan is not execution truth.

Current external state is validated again during `compose-build-psbt`.

## compose-build-psbt

`compose-build-psbt` validates current execution state and constructs the Compose PSBT.

Before construction, BCE re-resolves external inputs through Bitcoin Core and ord.

Successful output includes the constructed PSBT data and the unsigned transaction identity required by the broadcast commitment flow.

## compose-broadcast

`compose-broadcast` validates and broadcasts a signed Compose transaction.

The request includes the expected transaction ID committed during PSBT construction.

BCE resolves the finalized transaction, verifies its identity and broadcasts through Bitcoin Core JSON-RPC.

## split-plan

`split-plan` creates the deterministic plan for splitting a verified composition.

It determines:

- source composition
- verified physical groups
- output ranges
- output values
- transaction structure

The plan does not replace current execution-state validation.

## split-build-psbt

`split-build-psbt` re-validates current composition state and constructs the Split PSBT.

The current UTXO value and script are obtained from Bitcoin Core.

Current inscription placement is checked through ord.

## split-broadcast

`split-broadcast` validates the signed Split transaction against the expected transaction identity and broadcasts it through Bitcoin Core.

## extract-plan

`extract-plan` plans selective removal of verified physical groups.

Planning determines:

- selected extract groups
- extracted outputs
- remainder runs
- whether a Recompose child is required
- parent and dependent transaction structure

## extract-build-psbt

`extract-build-psbt` validates current external inputs and constructs the Extract transaction flow.

Depending on the plan, the result may contain:

- one parent transaction
- an additional deterministic Recompose child transaction

Child inputs originating from the newly constructed parent are deterministic internal state.

## extract-broadcast

`extract-broadcast` validates transaction identity and broadcasts a signed Extract transaction through Bitcoin Core JSON-RPC.

Dependent transaction flows must preserve parent-before-child execution order.

## insert-plan

`insert-plan` determines how new physical source groups are inserted into an existing composition.

The planner may select:

- Direct Append
- Split and Insert

Direct Append requires one transaction.

Split and Insert may require a deterministic parent and dependent child transaction.

## insert-build-psbt

`insert-build-psbt` resolves all current external execution inputs before construction.

This includes:

- existing composition UTXO
- inserted source UTXOs
- payment inputs

The command then constructs the required PSBT transaction flow.

## insert-broadcast

`insert-broadcast` validates the finalized Insert transaction identity and submits it through Bitcoin Core JSON-RPC.

For dependent flows, parent and child transactions must be broadcast in dependency order.

## verify-composition

`verify-composition` verifies an existing structured composition.

It translates structural composition into canonical physical BCE geometry.

Verification may evaluate:

- hierarchy
- direction
- physical order
- group ranges
- nested subtree structure
- physical boundaries

Successful output includes `items`, physical `groups` and `validatedSpecs`. The latter contains only normalized canonical structural fields.

It does not perform transaction execution.

## verify-compose

`verify-compose` verifies intended Compose structure before physical transaction construction.

It validates whether the requested structural composition can be translated unambiguously into valid physical composition geometry.

Successful output preserves the aggregated normalized Structural Specs as `validatedSpecs`, even though the Compose planning layer uses only the derived physical geometry.

## Planning versus execution

Planning commands determine intended transaction structure.

Build commands perform live execution-state validation.

The distinction is:

    plan
      |
      v
    intended geometry

    build-psbt
      |
      v
    current Bitcoin Core + ord validation
      |
      v
    PSBT construction

A valid plan may later become stale.

BCE therefore does not trust plan output as execution truth.

## Verification versus execution

Verification proves structural coherence.

Execution Guard proves current physical state.

The sequence is:

    structural intent
          |
          v
       Verify
          |
          v
    verified geometry
          |
          v
    build-psbt
          |
          v
    Execution Guard
          |
          v
    current physical state
          |
          v
    transaction construction

## tx-broadcast

`tx-broadcast` provides a generic fail-closed Bitcoin transaction broadcast path.

The request contains:

- `raw_tx`
- `expected_txid`

BCE decodes the finalized raw transaction, derives its transaction ID and requires:

    decoded_txid == expected_txid

Only then is the transaction submitted through Bitcoin Core.

The returned transaction ID must also match the committed transaction identity.

A successful response includes:

- `ok`
- `txid`
- optional `mempool_url`

## Broadcast commitment

Broadcast commands require the expected transaction identity produced during construction.

BCE verifies:

    decoded_txid == expected_txid

before network submission.

It also verifies:

    returned_txid == expected_txid

after Bitcoin Core accepts or recognizes the transaction.

## JSON interface

The CLI is designed as a machine-readable interface.

Requests and successful responses use JSON.

This makes the binary suitable for integration with:

- local applications
- services
- scripts
- APIs
- other process supervisors

Application-specific behavior remains outside BCE.

## Environment

Commands requiring current Bitcoin state use:

    BITCOIN_RPC_URL
    BITCOIN_RPC_USER
    BITCOIN_RPC_PASS
    ORD_SERVER_URL

Example node-local endpoints:

    BITCOIN_RPC_URL=http://127.0.0.1:8332
    ORD_SERVER_URL=http://127.0.0.1:8080

RPC credentials should be supplied securely through the execution environment.

## Failure behavior

BCE commands fail closed.

Operations are rejected when required state is invalid, stale, missing or contradictory.

Transaction-building commands do not fall back to previous cached state when Bitcoin Core or ord validation fails.

Broadcast commands do not submit transactions whose identity differs from the committed expected transaction ID.

---

Plan describes intent.

Verify proves structure.

Build validates current Bitcoin state and constructs the transaction.

Broadcast proves transaction identity and submits it.
