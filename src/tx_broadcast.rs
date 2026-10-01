use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::extract_broadcast::{broadcast_or_accept_known, decode_raw_tx, decoded_txid};

#[derive(Debug, Deserialize)]
pub struct TxBroadcastRequest {
    pub raw_tx: String,
    pub expected_txid: String,
}

#[derive(Debug, Serialize)]
pub struct TxBroadcastResponse {
    pub ok: bool,
    pub txid: String,
    pub mempool_url: Option<String>,
}

pub fn run_tx_broadcast(req: TxBroadcastRequest) -> Result<TxBroadcastResponse> {
    if req.raw_tx.trim().is_empty() {
        bail!("missing raw transaction");
    }

    if req.expected_txid.trim().is_empty() {
        bail!("missing expected txid");
    }

    /*
     * Generic fail-closed Bitcoin transaction broadcast.
     *
     * The caller commits to the transaction identity before broadcast.
     * BCE decodes the finalized raw transaction, derives its actual txid,
     * verifies it against the expected txid and only then submits it.
     */
    let decoded = decode_raw_tx(&req.raw_tx)?;
    let txid = decoded_txid(&decoded)?;

    if txid != req.expected_txid {
        bail!(
            "tx broadcast txid mismatch: expected {}, got {}",
            req.expected_txid,
            txid
        );
    }

    let broadcast_txid = broadcast_or_accept_known(&req.raw_tx, &txid)?;

    if broadcast_txid != txid {
        bail!(
            "tx broadcast result mismatch: expected {}, got {}",
            txid,
            broadcast_txid
        );
    }

    Ok(TxBroadcastResponse {
        ok: true,
        txid: txid.clone(),
        mempool_url: Some(format!("https://mempool.space/tx/{txid}")),
    })
}
