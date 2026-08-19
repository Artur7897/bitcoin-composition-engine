# Bitcoin Core and ord

BCE derives execution state directly from Bitcoin Core and ord.

Bitcoin Core provides authoritative UTXO state.

ord provides inscription and satpoint state.

BCE cross-checks both sources before transaction construction.

## Responsibility split

Bitcoin Core answers questions such as:

- does this outpoint exist?
- is it unspent?
- what is its current value?
- what is its scriptPubKey?
- what transaction contains it?
- can this transaction be accepted and broadcast?

ord answers questions such as:

- which inscription IDs exist in this output?
- where are their satpoints?
- which inscriptions share one physical offset?
- is the output indexed?
- what inscription state is currently observed?

Neither source is replaced by application state.

## Bitcoin Core as UTXO truth

Bitcoin Core is authoritative for spendable Bitcoin output state.

BCE uses Core to resolve:

- outpoint
- current value
- scriptPubKey
- address information where available
- spent or unspent state

A missing or spent output cannot be used as a valid current external input.

## ord as inscription truth

ord provides inscription-aware state.

BCE uses ord to resolve:

- inscription IDs
- inscription satpoints
- physical offsets
- output indexing state

This allows BCE to map inscription identity onto current physical Bitcoin positions.

## Cross-checking

Where Bitcoin Core and ord expose overlapping information, BCE checks consistency.

For example, the output resolved by ord must correspond to the same physical Bitcoin output resolved by Core.

Contradictory state is rejected.

BCE does not choose whichever source is more convenient.

## Direct Bitcoin JSON-RPC

BCE communicates with Bitcoin Core through direct JSON-RPC.

The engine does not require shell execution of `bitcoin-cli`.

RPC configuration is provided through environment variables:

    BITCOIN_RPC_URL
    BITCOIN_RPC_USER
    BITCOIN_RPC_PASS

Typical node-local configuration may use:

    BITCOIN_RPC_URL=http://127.0.0.1:8332

Credentials should be supplied through secure environment or service configuration.

They should not be embedded into source code.

## ord server configuration

BCE connects to an ord HTTP server.

The server URL is configured through:

    ORD_SERVER_URL

A node-local deployment may use:

    ORD_SERVER_URL=http://127.0.0.1:8080

BCE requires ord state to be available when inscription-aware execution validation is required.

## Output resolution

For a given outpoint, BCE obtains the current Bitcoin output from Core and the corresponding inscription state from ord.

The resolved physical state may include:

    outpoint
    value
    scriptPubKey
    address
    inscription IDs
    satpoint offsets
    physical postage spans

The execution layer uses this resolved state rather than trusting caller-supplied physical values.

## Inscription resolution

An inscription ID is resolved to its current satpoint.

The satpoint identifies:

- transaction ID
- output index
- offset inside the output

BCE uses the offset as physical position information.

The inscription's carrier output value is not automatically interpreted as per-inscription postage.

Postage is derived from distinct physical boundaries.

## Important value distinction

An inscription endpoint may expose the value of the output containing the inscription.

That value is the carrier UTXO value.

It is not necessarily the physical span belonging to that individual inscription.

For example:

    UTXO value: 1546

    inscription A at offset 0
    inscription B at offset 1000

The carrier UTXO value may be 1546 for both inscription queries.

The physical postage spans are instead:

    A position:
    0..1000
    postage 1000

    B position:
    1000..1546
    postage 546

BCE derives physical spans from offsets and UTXO end.

## Shared satpoints

ord may report multiple inscription IDs at the same offset.

BCE groups those IDs into one physical position.

For example:

    offset 0:
      A
      B

    offset 1000:
      C

produces two physical spans, not three.

The first span belongs to one physical satpoint position containing both A and B.

## Physical span derivation

Given:

- total UTXO value
- distinct sorted inscription offsets

BCE derives physical postage.

Example:

    value = 2092

    offsets:
    0
    546
    1546

Then:

    offset 0
    postage 546

    offset 546
    postage 1000

    offset 1546
    postage 546

The final span ends at the UTXO boundary.

## Indexed state

BCE requires ord to have usable indexed state for inscription-aware validation.

If ord cannot provide the required state, BCE fails closed.

It does not continue using stale cached inscription data as execution truth.

## Spent state

Bitcoin Core determines whether an external output remains available for spending.

A previously valid composition request may become stale if another transaction spends one of its inputs.

In that case, Execution Guard rejects PSBT construction.

## Script truth

Bitcoin Core scriptPubKey is authoritative.

Where an address is available and BCE derives a script from that address, the derived script must exactly match Core's scriptPubKey.

An address string alone does not override current script state.

## Payment inputs

Payment inputs are resolved through the same live execution path.

Bitcoin Core provides their current:

- value
- scriptPubKey
- spendable state

ord is used to ensure they do not contain inscriptions.

An inscription-bearing payment input is rejected.

## Execution timing

Bitcoin Core and ord are read immediately before PSBT construction.

The intended sequence is:

    plan
      |
      v
    verify
      |
      v
    resolve current Core + ord state
      |
      v
    validate
      |
      v
    build PSBT

This minimizes the window in which previously resolved state may become stale.

## No external resolver authority

Applications may maintain:

- databases
- caches
- indexers
- API responses
- previous plans
- previous verification results

These may be useful for UX and planning.

They are not BCE execution truth.

Current Bitcoin Core and ord state remains authoritative at the execution boundary.

## Failure model

BCE rejects execution when:

- Bitcoin Core cannot resolve a required external input
- the output is spent
- the current value does not match expectations
- the current script does not match
- ord cannot resolve required inscription state
- expected inscription IDs are missing
- physical satpoints contradict the requested operation
- Core and ord state are inconsistent

There is no stale-state fallback.

## Deployment model

BCE can run on the same machine as Bitcoin Core and ord.

This allows node-local access such as:

    BCE
     |
     +--> Bitcoin Core 127.0.0.1:8332
     |
     +--> ord          127.0.0.1:8080

A remote deployment is also possible where network policy permits secure access to both services.

Local deployment minimizes external infrastructure dependencies.

---

Bitcoin Core defines spendable Bitcoin state.

ord defines current inscription placement.

BCE requires both truths to agree before execution.
