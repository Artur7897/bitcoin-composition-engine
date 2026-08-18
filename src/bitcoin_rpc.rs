use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::env;

pub fn call(method: &str, params: Value) -> Result<Value> {
    let rpc_url = env::var("BITCOIN_RPC_URL").map_err(|_| anyhow!("BITCOIN_RPC_URL missing"))?;

    let rpc_user = env::var("BITCOIN_RPC_USER").map_err(|_| anyhow!("BITCOIN_RPC_USER missing"))?;

    let rpc_pass = env::var("BITCOIN_RPC_PASS").map_err(|_| anyhow!("BITCOIN_RPC_PASS missing"))?;

    let client = reqwest::blocking::Client::new();

    let response = client
        .post(&rpc_url)
        .basic_auth(rpc_user, Some(rpc_pass))
        .json(&json!({
            "jsonrpc": "1.0",
            "id": "bce",
            "method": method,
            "params": params,
        }))
        .send()?;

    if !response.status().is_success() {
        return Err(anyhow!("bitcoin rpc HTTP error: {}", response.status()));
    }

    let value: Value = response.json()?;

    if !value["error"].is_null() {
        return Err(anyhow!("bitcoin rpc error: {}", value["error"]));
    }

    value
        .get("result")
        .cloned()
        .ok_or_else(|| anyhow!("bitcoin rpc method {} returned no result", method))
}
