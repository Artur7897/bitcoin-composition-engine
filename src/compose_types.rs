use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::models::Utxo;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposePlanItem {
    pub id: String,
    pub postage: u64,
    pub utxo: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposePlanRequest {
    pub root_id: String,
    pub root_postage: u64,
    pub items: Vec<ComposePlanItem>,
    pub fee_rate: Option<u64>,
    pub payment_method: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposePlanResponse {
    pub ok: bool,
    pub packable: bool,
    pub network_fee: u64,
    pub service_fee: u64,
    pub total: u64,
    pub planned_offsets: Vec<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeBuildPsbtRequest {
    pub root_id: String,
    pub root_postage: u64,
    pub root_utxo: String,
    pub items: Vec<ComposePlanItem>,

    pub ordinals_address: String,
    pub payment_address: String,
    pub receive_address: Option<String>,
    pub change_address: Option<String>,
    pub ordinals_public_key: Option<String>,
    pub payment_public_key: Option<String>,

    pub payment_utxos: Vec<Utxo>,

    pub fee_rate: Option<u64>,
    pub payment_method: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeBuildPsbtResponse {
    pub ok: bool,
    pub psbt: String,
    pub unsigned_txid: String,
    pub sign_inputs: BTreeMap<String, Vec<u32>>,
    pub network_fee: u64,
    pub service_fee: u64,
    pub total: u64,
    pub planned_offsets: Vec<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastRequest {
    pub signed_psbt: Option<String>,
    pub raw_tx: Option<String>,
    pub expected_txid: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeBroadcastResponse {
    pub ok: bool,
    pub txid: String,
    pub mempool_url: Option<String>,
}
