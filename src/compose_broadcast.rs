use anyhow::{bail, Result};

use crate::compose_types::{BroadcastRequest, ComposeBroadcastResponse};
use crate::extract_broadcast::{
    broadcast_or_accept_known, decode_raw_tx, decoded_txid, resolve_raw_tx,
};

pub fn run_compose_broadcast(req: BroadcastRequest) -> Result<ComposeBroadcastResponse> {
    if req.expected_txid.trim().is_empty() {
        bail!("missing expected txid");
    }

    /*
     * Finalize/decode happens before broadcast.
     * The signed transaction must still be exactly the transaction
     * BCE committed to during compose-build-psbt.
     */
    let raw_tx = resolve_raw_tx(req.signed_psbt.as_deref(), req.raw_tx.as_deref(), "compose")?;

    let decoded = decode_raw_tx(&raw_tx)?;
    let txid = decoded_txid(&decoded)?;

    if txid != req.expected_txid {
        bail!(
            "compose txid mismatch: expected {}, got {}",
            req.expected_txid,
            txid
        );
    }

    let broadcast_txid = broadcast_or_accept_known(&raw_tx, &txid)?;

    if broadcast_txid != txid {
        bail!(
            "compose broadcast txid mismatch: expected {}, got {}",
            txid,
            broadcast_txid
        );
    }

    Ok(ComposeBroadcastResponse {
        ok: true,
        txid: txid.clone(),
        mempool_url: Some(format!("https://mempool.space/tx/{txid}")),
    })
}
