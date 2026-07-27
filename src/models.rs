use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SatPoint {
    pub txid: String,
    pub vout: u32,
    pub offset: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Inscription {
    pub id: String,
    pub satpoint: SatPoint,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Utxo {
    pub outpoint: String,
    pub value: u64,
    pub address: String,
    pub inscriptions: Vec<Inscription>,
}
