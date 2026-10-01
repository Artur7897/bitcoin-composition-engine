# BCE Release Checklist

## Current status

Bitcoin Composition Engine (BCE) is the canonical and actively maintained composition engine.

## Completed

- [x] Compose implementation
- [x] Split implementation
- [x] Extract implementation
- [x] Insert implementation
- [x] Execution Guard
- [x] Bitcoin Core + ord execution-state revalidation
- [x] Shared-satpoint handling
- [x] Build -> Sign -> Broadcast transaction commitments
- [x] CLI
- [x] API integration
- [x] Apache-2.0 licensing
- [x] Dependency/security checks
- [x] Public-facing BCE documentation
- [x] Web application Split end-to-end test through BCE API and CLI backend

## Manual release smoke tests

Before the first public BCE release, run and record one final mainnet smoke test for each operation:

- [ ] Compose
- [ ] Split
- [ ] Extract
- [ ] Insert

Each smoke test should verify the full execution path:

1. Build the operation from current Bitcoin Core + ord state.
2. Build the PSBT.
3. Sign with the wallet.
4. Broadcast through BCE.
5. Verify the resulting transaction and UTXO state.

## Release note

The manual smoke-test checklist is intentionally tracked separately from the automated test suite.

A successful application-level end-to-end Split transaction has already been completed through a BCE-backed integration path.
