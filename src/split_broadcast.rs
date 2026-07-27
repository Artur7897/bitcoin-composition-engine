use anyhow::{anyhow, Result};
use std::process::Command;

use crate::split_types::{SplitBroadcastRequest, SplitBroadcastResponse};

pub fn run_split_broadcast(req: SplitBroadcastRequest) -> Result<SplitBroadcastResponse> {
    let raw_tx = if let Some(raw_tx) = req.raw_tx {
        raw_tx
    } else if let Some(signed_psbt) = req.signed_psbt {
        finalize_psbt_to_raw_tx(&signed_psbt)?
    } else {
        return Err(anyhow!("signedPsbt or rawTx is required"));
    };

    let txid = broadcast_raw_tx(&raw_tx)?;

    Ok(SplitBroadcastResponse {
        ok: true,
        txid: txid.clone(),
        mempool_url: Some(format!("https://mempool.space/tx/{txid}")),
    })
}

fn finalize_psbt_to_raw_tx(signed_psbt: &str) -> Result<String> {
    let out = bitcoin_cli(&["finalizepsbt", signed_psbt])?;
    let json: serde_json::Value = serde_json::from_str(&out)?;

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

    Ok(hex.to_string())
}

fn broadcast_raw_tx(raw_tx: &str) -> Result<String> {
    bitcoin_cli(&["sendrawtransaction", raw_tx])
}

fn bitcoin_cli(args: &[&str]) -> Result<String> {
    let output = Command::new("bitcoin-cli")
        .args(args)
        .output()
        .map_err(|e| anyhow!("failed to run bitcoin-cli: {}", e))?;

    if !output.status.success() {
        return Err(anyhow!(
            "bitcoin-cli failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
