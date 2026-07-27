use anyhow::{anyhow, Result};
use serde_json::Value;
use std::process::Command;

use crate::compose_types::{BroadcastRequest, ComposeBroadcastResponse};

fn bitcoin_cli(args: &[&str]) -> Result<String> {
    let output = Command::new("bitcoin-cli")
        .args(args)
        .output()
        .map_err(|e| anyhow!("failed to run bitcoin-cli: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(anyhow!("bitcoin-cli failed: {}", stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn finalize_psbt_to_raw_tx(signed_psbt: &str) -> Result<String> {
    let out = bitcoin_cli(&["finalizepsbt", signed_psbt])?;

    let json: Value = serde_json::from_str(&out)
        .map_err(|e| anyhow!("failed to parse finalizepsbt response: {}", e))?;

    let complete = json
        .get("complete")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if !complete {
        return Err(anyhow!("PSBT is not complete"));
    }

    let hex = json
        .get("hex")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("finalizepsbt returned no hex"))?;

    if hex.trim().is_empty() {
        return Err(anyhow!("finalizepsbt returned empty hex"));
    }

    Ok(hex.to_string())
}

fn broadcast_raw_tx(raw_tx: &str) -> Result<String> {
    let txid = bitcoin_cli(&["sendrawtransaction", raw_tx])?;

    if txid.trim().is_empty() {
        return Err(anyhow!("sendrawtransaction returned empty txid"));
    }

    Ok(txid)
}

pub fn run_compose_broadcast(req: BroadcastRequest) -> Result<ComposeBroadcastResponse> {
    let signed_psbt = req.signed_psbt.unwrap_or_default();
    let raw_tx = req.raw_tx.unwrap_or_default();

    if signed_psbt.trim().is_empty() && raw_tx.trim().is_empty() {
        return Err(anyhow!("signedPsbt or rawTx is required"));
    }

    let final_raw_tx = if !raw_tx.trim().is_empty() {
        raw_tx
    } else {
        finalize_psbt_to_raw_tx(&signed_psbt)?
    };

    let txid = broadcast_raw_tx(&final_raw_tx)?;

    Ok(ComposeBroadcastResponse {
        ok: true,
        txid,
        mempool_url: None,
    })
}
