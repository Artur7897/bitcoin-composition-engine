use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose, Engine as _};
use bitcoin::{
    absolute, address::Address, psbt::Psbt, transaction, Amount, Network, OutPoint, ScriptBuf,
    Sequence, Transaction, TxIn, TxOut, Txid, Witness,
};
use std::{collections::BTreeMap, str::FromStr};

use crate::extract::{ExtractOutput, ExtractOutputKind, RecomposeIntent};
use crate::fees::{estimate_network_fee, DUST_LIMIT};

pub struct RecomposeBuildRequest {
    pub parent_txid: String,

    /// Contains only the ordinal outputs of the extract parent transaction.
    pub parent_outputs: Vec<ExtractOutput>,

    pub recompose: RecomposeIntent,

    pub payment_change_output_index: u32,
    pub payment_change_value: u64,

    pub ordinals_address: String,
    pub payment_signing_address: String,
    pub change_address: String,

    pub ordinals_public_key: Option<String>,
    pub payment_public_key: Option<String>,

    pub fee_rate: u64,
}

#[derive(Debug)]
pub struct RecomposeBuildResult {
    pub psbt: String,
    pub unsigned_txid: String,
    pub sign_inputs: BTreeMap<String, Vec<u32>>,

    pub network_fee: u64,
    pub vsize: u64,

    pub change_output_index: u32,
    pub change_value: u64,
}

pub fn estimate_recompose_fee(remainder_input_count: usize, fee_rate: u64) -> Result<(u64, u64)> {
    if remainder_input_count < 2 {
        bail!("recompose requires at least two remainder inputs");
    }

    let input_count = remainder_input_count
        .checked_add(1)
        .ok_or_else(|| anyhow!("recompose input count overflow"))?;

    estimate_network_fee(input_count, 2, fee_rate)
}

pub fn build_recompose_psbt(req: RecomposeBuildRequest) -> Result<RecomposeBuildResult> {
    validate_recompose_geometry(&req.recompose)?;

    if req.recompose.input_output_indices.len() < 2 {
        bail!("recompose requires at least two remainder outputs");
    }

    for pair in req.recompose.input_output_indices.windows(2) {
        if pair[0] >= pair[1] {
            bail!("recompose input indices must be strictly ascending");
        }
    }

    let parent_txid = Txid::from_str(&req.parent_txid)?;

    let (vsize, network_fee) =
        estimate_recompose_fee(req.recompose.input_output_indices.len(), req.fee_rate)?;

    let change_value = req
        .payment_change_value
        .checked_sub(network_fee)
        .ok_or_else(|| anyhow!("payment change cannot pay recompose fee"))?;

    if change_value < DUST_LIMIT {
        bail!("recompose payment change would be dust");
    }

    let mut inputs = Vec::<TxIn>::new();
    let mut remainder_values = Vec::<u64>::new();

    /*
     * Remainder inputs must appear first and in parent-vout order.
     * Only then do their sats remain in the correct order.
     */
    for output_index in &req.recompose.input_output_indices {
        let parent_output = req
            .parent_outputs
            .get(*output_index as usize)
            .ok_or_else(|| anyhow!("missing parent remainder output {}", output_index))?;

        if parent_output.kind != ExtractOutputKind::Remainder {
            bail!("parent output {} is not a remainder", output_index);
        }

        inputs.push(build_txin(OutPoint {
            txid: parent_txid,
            vout: *output_index,
        }));

        remainder_values.push(parent_output.value);
    }

    let remainder_total = remainder_values.iter().try_fold(0_u64, |total, value| {
        total
            .checked_add(*value)
            .ok_or_else(|| anyhow!("remainder value overflow"))
    })?;

    if remainder_total != req.recompose.total_value {
        bail!(
            "parent remainder value mismatch: outputs {}, plan {}",
            remainder_total,
            req.recompose.total_value
        );
    }

    /*
     * Payment change comes after all ordinal inputs.
     */
    inputs.push(build_txin(OutPoint {
        txid: parent_txid,
        vout: req.payment_change_output_index,
    }));

    let outputs = vec![
        TxOut {
            value: Amount::from_sat(req.recompose.total_value),
            script_pubkey: address_to_script(&req.ordinals_address)?,
        },
        TxOut {
            value: Amount::from_sat(change_value),
            script_pubkey: address_to_script(&req.change_address)?,
        },
    ];

    let tx = Transaction {
        version: transaction::Version(2),
        lock_time: absolute::LockTime::ZERO,
        input: inputs,
        output: outputs,
    };

    let unsigned_txid = tx.txid().to_string();
    let mut psbt = Psbt::from_unsigned_tx(tx)?;

    let remainder_input_count = req.recompose.input_output_indices.len();

    for (input_index, value) in remainder_values.iter().enumerate() {
        psbt.inputs[input_index].witness_utxo = Some(TxOut {
            value: Amount::from_sat(*value),
            script_pubkey: address_to_script(&req.ordinals_address)?,
        });

        if req.ordinals_address.starts_with("bc1p") {
            if let Some(public_key_hex) = req.ordinals_public_key.as_ref() {
                psbt.inputs[input_index].tap_internal_key =
                    Some(xonly_from_pubkey_hex(public_key_hex)?);
            }
        }
    }

    let payment_input_index = remainder_input_count;

    psbt.inputs[payment_input_index].witness_utxo = Some(TxOut {
        value: Amount::from_sat(req.payment_change_value),
        script_pubkey: address_to_script(&req.change_address)?,
    });

    if req.change_address.starts_with("bc1p") {
        if let Some(public_key_hex) = req.payment_public_key.as_ref() {
            psbt.inputs[payment_input_index].tap_internal_key =
                Some(xonly_from_pubkey_hex(public_key_hex)?);
        }
    }

    let psbt_base64 = general_purpose::STANDARD.encode(psbt.serialize());

    let mut sign_inputs = BTreeMap::<String, Vec<u32>>::new();

    let ordinal_indices: Vec<u32> = (0..remainder_input_count)
        .map(|index| u32::try_from(index).map_err(|_| anyhow!("ordinal signing index overflow")))
        .collect::<Result<Vec<_>>>()?;

    sign_inputs
        .entry(req.ordinals_address.clone())
        .or_default()
        .extend(ordinal_indices);

    let payment_index = u32::try_from(payment_input_index)
        .map_err(|_| anyhow!("payment signing index overflow"))?;

    sign_inputs
        .entry(req.payment_signing_address)
        .or_default()
        .push(payment_index);

    Ok(RecomposeBuildResult {
        psbt: psbt_base64,
        unsigned_txid,
        sign_inputs,
        network_fee,
        vsize,
        change_output_index: 1,
        change_value,
    })
}

fn validate_recompose_geometry(recompose: &RecomposeIntent) -> Result<()> {
    let first = recompose
        .groups
        .first()
        .ok_or_else(|| anyhow!("recompose contains no groups"))?;

    if first.offset != 0 {
        bail!("recompose first offset must be 0");
    }

    for group in &recompose.groups {
        if group.ids.is_empty() {
            bail!("recompose group has no IDs");
        }

        if group.postage == 0 {
            bail!("recompose group has zero postage");
        }
    }

    for pair in recompose.groups.windows(2) {
        let expected_offset = pair[0]
            .offset
            .checked_add(pair[0].postage)
            .ok_or_else(|| anyhow!("recompose offset overflow"))?;

        if pair[1].offset != expected_offset {
            bail!(
                "invalid recompose geometry: expected {}, got {}",
                expected_offset,
                pair[1].offset
            );
        }
    }

    let calculated_total = recompose.groups.iter().try_fold(0_u64, |total, group| {
        total
            .checked_add(group.postage)
            .ok_or_else(|| anyhow!("recompose total overflow"))
    })?;

    if calculated_total != recompose.total_value {
        bail!(
            "recompose geometry value mismatch: calculated {}, supplied {}",
            calculated_total,
            recompose.total_value
        );
    }

    Ok(())
}

fn build_txin(outpoint: OutPoint) -> TxIn {
    TxIn {
        previous_output: outpoint,
        script_sig: ScriptBuf::new(),
        sequence: Sequence::MAX,
        witness: Witness::new(),
    }
}

fn address_to_script(address: &str) -> Result<ScriptBuf> {
    let parsed = Address::from_str(address)?.require_network(Network::Bitcoin)?;

    Ok(parsed.script_pubkey())
}

fn xonly_from_pubkey_hex(public_key_hex: &str) -> Result<bitcoin::secp256k1::XOnlyPublicKey> {
    let bytes = decode_hex(public_key_hex)?;

    let xonly_bytes: Vec<u8> = match bytes.len() {
        32 => bytes,
        33 => bytes[1..33].to_vec(),
        length => {
            bail!("invalid public key length: {}", length)
        }
    };

    bitcoin::secp256k1::XOnlyPublicKey::from_slice(&xonly_bytes)
        .map_err(|_| anyhow!("invalid xonly public key"))
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    let clean = value.trim().strip_prefix("0x").unwrap_or(value.trim());

    if !clean.len().is_multiple_of(2) {
        bail!("invalid hex length");
    }

    let mut output = Vec::with_capacity(clean.len() / 2);

    for index in (0..clean.len()).step_by(2) {
        let byte =
            u8::from_str_radix(&clean[index..index + 2], 16).map_err(|_| anyhow!("invalid hex"))?;

        output.push(byte);
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::ExtractGroup;
    use base64::engine::general_purpose;
    use bitcoin::psbt::Psbt;

    const ADDRESS: &str = "bc1qznl7wxgtemt5eprmr6g3yj7nn7xh5gtzuvezuz";

    fn parent_outputs() -> Vec<ExtractOutput> {
        vec![
            ExtractOutput {
                kind: ExtractOutputKind::Remainder,
                ids: vec!["A".to_string()],
                source_offset: 0,
                value: 546,
                address: ADDRESS.to_string(),
            },
            ExtractOutput {
                kind: ExtractOutputKind::Extracted,
                ids: vec!["B".to_string()],
                source_offset: 546,
                value: 700,
                address: ADDRESS.to_string(),
            },
            ExtractOutput {
                kind: ExtractOutputKind::Remainder,
                ids: vec!["C".to_string()],
                source_offset: 1246,
                value: 600,
                address: ADDRESS.to_string(),
            },
            ExtractOutput {
                kind: ExtractOutputKind::Extracted,
                ids: vec!["D".to_string()],
                source_offset: 1846,
                value: 800,
                address: ADDRESS.to_string(),
            },
        ]
    }

    fn recompose_intent() -> RecomposeIntent {
        RecomposeIntent {
            input_output_indices: vec![0, 2],
            groups: vec![
                ExtractGroup {
                    ids: vec!["A".to_string()],
                    offset: 0,
                    postage: 546,
                },
                ExtractGroup {
                    ids: vec!["C".to_string()],
                    offset: 546,
                    postage: 600,
                },
            ],
            total_value: 1146,
        }
    }

    fn request() -> RecomposeBuildRequest {
        RecomposeBuildRequest {
            parent_txid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            parent_outputs: parent_outputs(),
            recompose: recompose_intent(),
            payment_change_output_index: 5,
            payment_change_value: 8096,
            ordinals_address: ADDRESS.to_string(),
            payment_signing_address: ADDRESS.to_string(),
            change_address: ADDRESS.to_string(),
            ordinals_public_key: None,
            payment_public_key: None,
            fee_rate: 1,
        }
    }

    #[test]
    fn builds_expected_child_transaction() {
        let result = build_recompose_psbt(request()).expect("recompose PSBT must build");

        assert_eq!(result.network_fee, 300);
        assert_eq!(result.vsize, 300);
        assert_eq!(result.change_output_index, 1);
        assert_eq!(result.change_value, 7796);

        assert_eq!(result.sign_inputs.get(ADDRESS), Some(&vec![0, 1, 2]));

        let bytes = general_purpose::STANDARD
            .decode(&result.psbt)
            .expect("valid base64");

        let psbt = Psbt::deserialize(&bytes).expect("valid PSBT");

        let tx = &psbt.unsigned_tx;

        assert_eq!(tx.input.len(), 3);
        assert_eq!(tx.output.len(), 2);

        assert_eq!(tx.input[0].previous_output.vout, 0);
        assert_eq!(tx.input[1].previous_output.vout, 2);
        assert_eq!(tx.input[2].previous_output.vout, 5);

        assert_eq!(tx.output[0].value.to_sat(), 1146);
        assert_eq!(tx.output[1].value.to_sat(), 7796);
    }

    #[test]
    fn rejects_unsorted_parent_outputs() {
        let mut request = request();

        request.recompose.input_output_indices = vec![2, 0];

        let error = build_recompose_psbt(request).expect_err("unsorted inputs must fail");

        assert!(error.to_string().contains("strictly ascending"));
    }

    #[test]
    fn rejects_extracted_output_as_remainder_input() {
        let mut request = request();

        request.recompose.input_output_indices = vec![0, 1];

        let error = build_recompose_psbt(request).expect_err("extracted input must fail");

        assert!(error.to_string().contains("is not a remainder"));
    }

    #[test]
    fn rejects_remainder_value_mismatch() {
        let mut request = request();

        request.recompose.total_value = 1200;

        let error = build_recompose_psbt(request).expect_err("value mismatch must fail");

        assert!(error.to_string().contains("value mismatch"));
    }
}
