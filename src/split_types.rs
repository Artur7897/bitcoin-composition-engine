use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PaymentUtxo {
    pub outpoint: String,
    pub value: u64,
    pub address: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SplitGroup {
    pub ids: Vec<String>,
    pub offset: u64,
    pub value: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SplitPlanRequest {
    pub input_utxo: String,
    pub ordinals_address: String,
    pub fee_rate: Option<u64>,
    pub total_value: u64,
    pub groups: Vec<SplitGroup>,
}

#[derive(Debug, Serialize)]
pub struct SplitPlanResponse {
    pub ok: bool,
    pub splittable: bool,
    pub network_fee: u64,
    pub total: u64,
    pub outputs: Vec<SplitGroup>,
    pub output_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SplitBuildPsbtRequest {
    pub input_utxo: String,
    pub ordinals_address: String,
    pub payment_address: String,
    pub receive_address: Option<String>,
    pub change_address: Option<String>,
    pub ordinals_public_key: Option<String>,
    pub payment_public_key: Option<String>,
    pub fee_rate: Option<u64>,
    pub total_value: u64,
    pub groups: Vec<SplitGroup>,
    pub payment_utxos: Vec<PaymentUtxo>,
}

#[derive(Debug, Serialize)]
pub struct SplitBuildPsbtResponse {
    pub ok: bool,
    pub psbt: String,
    pub unsigned_txid: String,
    pub sign_inputs: BTreeMap<String, Vec<u32>>,
    pub network_fee: u64,
    pub total: u64,
    pub outputs: usize,
    pub tx_outputs: usize,
    pub vsize: u64,
}

#[derive(Debug, Deserialize)]
pub struct SplitBroadcastRequest {
    pub signed_psbt: Option<String>,
    pub raw_tx: Option<String>,
    pub expected_txid: String,
}

#[derive(Debug, Serialize)]
pub struct SplitBroadcastResponse {
    pub ok: bool,
    pub txid: String,
    pub mempool_url: Option<String>,
}
