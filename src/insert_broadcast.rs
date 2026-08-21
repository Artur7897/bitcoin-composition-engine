use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::extract_broadcast::{
    broadcast_or_accept_known, decode_raw_tx, decoded_txid, has_text, resolve_raw_tx,
};

#[derive(Debug, Deserialize)]
pub struct InsertBroadcastRequest {
    pub signed_primary_psbt: Option<String>,
    pub primary_raw_tx: Option<String>,
    pub expected_primary_txid: String,

    pub signed_child_psbt: Option<String>,
    pub child_raw_tx: Option<String>,
    pub expected_child_txid: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InsertBroadcastResponse {
    pub ok: bool,

    pub primary_txid: String,
    pub primary_mempool_url: String,

    pub child_txid: Option<String>,
    pub child_mempool_url: Option<String>,

    pub error: Option<String>,
}

pub fn run_insert_broadcast(req: InsertBroadcastRequest) -> Result<InsertBroadcastResponse> {
    if req.expected_primary_txid.trim().is_empty() {
        bail!("missing expected primary txid");
    }

    /*
     * Primary and child transactions are fully finalized and
     * verified before any transaction is broadcast.
     */
    let primary_raw = resolve_raw_tx(
        req.signed_primary_psbt.as_deref(),
        req.primary_raw_tx.as_deref(),
        "insert primary",
    )?;

    let primary_decoded = decode_raw_tx(&primary_raw)?;

    let primary_txid = decoded_txid(&primary_decoded)?;

    if primary_txid != req.expected_primary_txid {
        bail!(
            "insert primary txid mismatch: expected {}, got {}",
            req.expected_primary_txid,
            primary_txid
        );
    }

    let has_child_payload =
        has_text(req.signed_child_psbt.as_deref()) || has_text(req.child_raw_tx.as_deref());

    let has_expected_child = req
        .expected_child_txid
        .as_deref()
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false);

    if has_child_payload != has_expected_child {
        bail!("insert child and expected child txid must be supplied together");
    }

    let prepared_child = if has_child_payload {
        let expected_child_txid = req
            .expected_child_txid
            .as_deref()
            .ok_or_else(|| anyhow!("missing expected child txid"))?;

        let child_raw = resolve_raw_tx(
            req.signed_child_psbt.as_deref(),
            req.child_raw_tx.as_deref(),
            "insert child",
        )?;

        let child_decoded = decode_raw_tx(&child_raw)?;

        let child_txid = decoded_txid(&child_decoded)?;

        if child_txid != expected_child_txid {
            bail!(
                "insert child txid mismatch: expected {}, got {}",
                expected_child_txid,
                child_txid
            );
        }

        verify_insert_child_dependency(&child_decoded, &primary_txid)?;

        Some((child_raw, child_txid))
    } else {
        None
    };

    let broadcast_primary_txid = broadcast_or_accept_known(&primary_raw, &primary_txid)?;

    if broadcast_primary_txid != primary_txid {
        bail!(
            "insert primary broadcast txid mismatch: expected {}, got {}",
            primary_txid,
            broadcast_primary_txid
        );
    }

    let primary_mempool_url = format!("https://mempool.space/tx/{primary_txid}");

    match prepared_child {
        Some((child_raw, child_txid)) => match broadcast_or_accept_known(&child_raw, &child_txid) {
            Ok(broadcast_child_txid) => {
                if broadcast_child_txid != child_txid {
                    return Ok(InsertBroadcastResponse {
                        ok: false,
                        primary_txid,
                        primary_mempool_url,
                        child_txid: None,
                        child_mempool_url: None,
                        error: Some(format!(
                            "insert child broadcast txid mismatch: expected {}, got {}",
                            child_txid, broadcast_child_txid
                        )),
                    });
                }

                Ok(InsertBroadcastResponse {
                    ok: true,
                    primary_txid,
                    primary_mempool_url,
                    child_mempool_url: Some(format!("https://mempool.space/tx/{child_txid}")),
                    child_txid: Some(child_txid),
                    error: None,
                })
            }

            Err(error) => Ok(InsertBroadcastResponse {
                ok: false,
                primary_txid,
                primary_mempool_url,
                child_txid: None,
                child_mempool_url: None,
                error: Some(format!(
                    "insert primary broadcast succeeded, child failed: {}",
                    error
                )),
            }),
        },

        None => Ok(InsertBroadcastResponse {
            ok: true,
            primary_txid,
            primary_mempool_url,
            child_txid: None,
            child_mempool_url: None,
            error: None,
        }),
    }
}

fn verify_insert_child_dependency(decoded: &Value, primary_txid: &str) -> Result<()> {
    let inputs = decoded
        .get("vin")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("decoded insert child has no inputs"))?;

    /*
     * SplitAndInsert contains at least:
     * - two existing-run inputs from the parent
     * - a payment change input from the parent
     * - at least one external insert input
     */
    let parent_input_count = inputs
        .iter()
        .filter(|input| input.get("txid").and_then(Value::as_str) == Some(primary_txid))
        .count();

    if parent_input_count < 3 {
        bail!("insert child does not spend all required parent outputs");
    }

    if parent_input_count == inputs.len() {
        bail!("insert child contains no external insert input");
    }

    Ok(())
}
