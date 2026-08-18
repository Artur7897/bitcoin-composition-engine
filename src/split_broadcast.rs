use anyhow::{bail, Result};

use crate::extract_broadcast::{
    broadcast_or_accept_known, decode_raw_tx, decoded_txid, resolve_raw_tx,
};
use crate::split_types::{SplitBroadcastRequest, SplitBroadcastResponse};

pub fn run_split_broadcast(req: SplitBroadcastRequest) -> Result<SplitBroadcastResponse> {
    if req.expected_txid.trim().is_empty() {
        bail!("missing expected txid");
    }

    /*
     * The finalized transaction must still match the unsigned transaction
     * committed to by split-build-psbt.
     */
    let raw_tx = resolve_raw_tx(req.signed_psbt.as_deref(), req.raw_tx.as_deref(), "split")?;

    let decoded = decode_raw_tx(&raw_tx)?;
    let txid = decoded_txid(&decoded)?;

    if txid != req.expected_txid {
        bail!(
            "split txid mismatch: expected {}, got {}",
            req.expected_txid,
            txid
        );
    }

    let broadcast_txid = broadcast_or_accept_known(&raw_tx, &txid)?;

    if broadcast_txid != txid {
        bail!(
            "split broadcast txid mismatch: expected {}, got {}",
            txid,
            broadcast_txid
        );
    }

    Ok(SplitBroadcastResponse {
        ok: true,
        txid: txid.clone(),
        mempool_url: Some(format!("https://mempool.space/tx/{txid}")),
    })
}
