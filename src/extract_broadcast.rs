use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;

#[derive(Debug, Deserialize)]
pub struct ExtractBroadcastRequest {
    pub signed_parent_psbt: Option<String>,
    pub parent_raw_tx: Option<String>,
    pub expected_parent_txid: String,

    pub signed_recompose_psbt: Option<String>,
    pub recompose_raw_tx: Option<String>,
    pub expected_recompose_txid: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExtractBroadcastResponse {
    pub ok: bool,

    pub parent_txid: String,
    pub parent_mempool_url: String,

    pub recompose_txid: Option<String>,
    pub recompose_mempool_url: Option<String>,

    /// Nur gesetzt, wenn Parent erfolgreich war,
    /// aber Child nicht gesendet werden konnte.
    pub error: Option<String>,
}

pub fn run_extract_broadcast(req: ExtractBroadcastRequest) -> Result<ExtractBroadcastResponse> {
    if req.expected_parent_txid.trim().is_empty() {
        bail!("missing expected parent txid");
    }

    let parent_raw = resolve_raw_tx(
        req.signed_parent_psbt.as_deref(),
        req.parent_raw_tx.as_deref(),
        "parent",
    )?;

    let parent_decoded = decode_raw_tx(&parent_raw)?;
    let parent_txid = decoded_txid(&parent_decoded)?;

    if parent_txid != req.expected_parent_txid {
        bail!(
            "parent txid mismatch: expected {}, got {}",
            req.expected_parent_txid,
            parent_txid
        );
    }

    let has_recompose_payload =
        has_text(req.signed_recompose_psbt.as_deref()) || has_text(req.recompose_raw_tx.as_deref());

    let has_expected_recompose = req
        .expected_recompose_txid
        .as_deref()
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false);

    if has_recompose_payload != has_expected_recompose {
        bail!("recompose transaction and expected txid must be supplied together");
    }

    let prepared_recompose = if has_recompose_payload {
        let expected_recompose_txid = req
            .expected_recompose_txid
            .as_deref()
            .ok_or_else(|| anyhow!("missing expected recompose txid"))?;

        let raw = resolve_raw_tx(
            req.signed_recompose_psbt.as_deref(),
            req.recompose_raw_tx.as_deref(),
            "recompose",
        )?;

        let decoded = decode_raw_tx(&raw)?;
        let txid = decoded_txid(&decoded)?;

        if txid != expected_recompose_txid {
            bail!(
                "recompose txid mismatch: expected {}, got {}",
                expected_recompose_txid,
                txid
            );
        }

        verify_recompose_spends_parent(&decoded, &parent_txid)?;

        Some((raw, txid))
    } else {
        None
    };

    let broadcast_parent_txid = broadcast_or_accept_known(&parent_raw, &parent_txid)?;

    if broadcast_parent_txid != parent_txid {
        bail!(
            "broadcast parent txid mismatch: expected {}, got {}",
            parent_txid,
            broadcast_parent_txid
        );
    }

    let parent_mempool_url = format!("https://mempool.space/tx/{parent_txid}");

    match prepared_recompose {
        Some((recompose_raw, recompose_txid)) => {
            match broadcast_or_accept_known(&recompose_raw, &recompose_txid) {
                Ok(broadcast_txid) => {
                    if broadcast_txid != recompose_txid {
                        return Ok(ExtractBroadcastResponse {
                            ok: false,
                            parent_txid,
                            parent_mempool_url,
                            recompose_txid: None,
                            recompose_mempool_url: None,
                            error: Some(format!(
                                "recompose broadcast txid mismatch: expected {}, got {}",
                                recompose_txid, broadcast_txid
                            )),
                        });
                    }

                    Ok(ExtractBroadcastResponse {
                        ok: true,
                        parent_txid,
                        parent_mempool_url,
                        recompose_mempool_url: Some(format!(
                            "https://mempool.space/tx/{recompose_txid}"
                        )),
                        recompose_txid: Some(recompose_txid),
                        error: None,
                    })
                }

                Err(error) => Ok(ExtractBroadcastResponse {
                    ok: false,
                    parent_txid,
                    parent_mempool_url,
                    recompose_txid: None,
                    recompose_mempool_url: None,
                    error: Some(format!(
                        "parent broadcast succeeded, recompose failed: {}",
                        error
                    )),
                }),
            }
        }

        None => Ok(ExtractBroadcastResponse {
            ok: true,
            parent_txid,
            parent_mempool_url,
            recompose_txid: None,
            recompose_mempool_url: None,
            error: None,
        }),
    }
}

pub(crate) fn resolve_raw_tx(
    signed_psbt: Option<&str>,
    raw_tx: Option<&str>,
    label: &str,
) -> Result<String> {
    if let Some(raw_tx) = raw_tx {
        if !raw_tx.trim().is_empty() {
            return Ok(raw_tx.trim().to_string());
        }
    }

    if let Some(signed_psbt) = signed_psbt {
        if !signed_psbt.trim().is_empty() {
            return finalize_psbt_to_raw_tx(signed_psbt);
        }
    }

    bail!("missing signed PSBT or raw transaction for {}", label)
}

fn finalize_psbt_to_raw_tx(signed_psbt: &str) -> Result<String> {
    let response = bitcoin_rpc("finalizepsbt", &[signed_psbt])?;

    let json: Value = serde_json::from_str(&response)
        .map_err(|error| anyhow!("failed to parse finalizepsbt response: {}", error))?;

    let complete = json
        .get("complete")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if !complete {
        bail!("PSBT is not complete");
    }

    let raw_tx = json
        .get("hex")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("finalizepsbt returned no hex"))?;

    if raw_tx.trim().is_empty() {
        bail!("finalizepsbt returned empty hex");
    }

    Ok(raw_tx.to_string())
}

pub(crate) fn decode_raw_tx(raw_tx: &str) -> Result<Value> {
    let response = bitcoin_rpc("decoderawtransaction", &[raw_tx])?;

    serde_json::from_str(&response)
        .map_err(|error| anyhow!("failed to parse decoded transaction: {}", error))
}

pub(crate) fn decoded_txid(decoded: &Value) -> Result<String> {
    decoded
        .get("txid")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("decoded transaction contains no txid"))
}

fn verify_recompose_spends_parent(decoded: &Value, parent_txid: &str) -> Result<()> {
    let inputs = decoded
        .get("vin")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("decoded recompose transaction has no inputs"))?;

    if inputs.len() < 3 {
        bail!("recompose transaction requires remainder inputs and payment change");
    }

    for input in inputs {
        let input_txid = input
            .get("txid")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("recompose input contains no parent txid"))?;

        if input_txid != parent_txid {
            bail!("recompose input does not spend expected parent");
        }
    }

    Ok(())
}

pub(crate) fn broadcast_or_accept_known(raw_tx: &str, expected_txid: &str) -> Result<String> {
    match bitcoin_rpc("sendrawtransaction", &[raw_tx]) {
        Ok(txid) => Ok(txid),

        Err(error) => {
            if transaction_is_known(expected_txid) {
                Ok(expected_txid.to_string())
            } else {
                Err(error)
            }
        }
    }
}

fn transaction_is_known(txid: &str) -> bool {
    if bitcoin_rpc("getmempoolentry", &[txid]).is_ok() {
        return true;
    }

    bitcoin_rpc("getrawtransaction", &[txid]).is_ok()
}

fn bitcoin_rpc(method: &str, params: &[&str]) -> Result<String> {
    let rpc_url = env::var("BITCOIN_RPC_URL").map_err(|_| anyhow!("BITCOIN_RPC_URL missing"))?;

    let rpc_user = env::var("BITCOIN_RPC_USER").map_err(|_| anyhow!("BITCOIN_RPC_USER missing"))?;

    let rpc_pass = env::var("BITCOIN_RPC_PASS").map_err(|_| anyhow!("BITCOIN_RPC_PASS missing"))?;

    let client = reqwest::blocking::Client::new();

    let response = client
        .post(&rpc_url)
        .basic_auth(rpc_user, Some(rpc_pass))
        .json(&json!({
            "jsonrpc": "1.0",
            "id": "bce-extract-broadcast",
            "method": method,
            "params": params,
        }))
        .send()?;

    let value: Value = response.json()?;

    if !value["error"].is_null() {
        return Err(anyhow!("bitcoin rpc error: {}", value["error"]));
    }

    let result = value
        .get("result")
        .ok_or_else(|| anyhow!("bitcoin rpc method {} returned no result", method))?;

    match result {
        Value::String(value) => Ok(value.clone()),
        _ => Ok(serde_json::to_string(result)?),
    }
}

pub(crate) fn has_text(value: Option<&str>) -> bool {
    value
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false)
}
