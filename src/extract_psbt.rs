use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose, Engine as _};
use bitcoin::{
    absolute, address::Address, psbt::Psbt, transaction, Amount, Network, OutPoint, ScriptBuf,
    Sequence, Transaction, TxIn, TxOut, Witness,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, str::FromStr};

use crate::execution_guard::{validate_current_output, CurrentOutputState};
use crate::extract::{
    run_extract_plan, ExtractGroup, ExtractPlanRequest, ExtractedOutputRef, RecomposeIntent,
};
use crate::fees::DUST_LIMIT;
use crate::models::Utxo;
use crate::recompose_psbt::{build_recompose_psbt, RecomposeBuildRequest};

#[derive(Debug, Serialize)]
pub struct ExtractRecomposePsbtResponse {
    pub psbt: String,
    pub unsigned_txid: String,
    pub sign_inputs: BTreeMap<String, Vec<u32>>,
    pub miner_fee_sats: u64,
    pub change_output_index: u32,
    pub change_value: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtractBuildPsbtRequest {
    pub input_utxo: String,
    pub ordinals_address: String,
    pub payment_address: String,
    pub change_address: Option<String>,

    pub ordinals_public_key: Option<String>,
    pub payment_public_key: Option<String>,

    pub total_value: u64,
    pub groups: Vec<ExtractGroup>,
    pub extract_groups: Vec<Vec<String>>,

    pub payment_utxos: Vec<Utxo>,
    pub primary_miner_fee_sats: u64,
    pub recompose_miner_fee_sats: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ExtractBuildPsbtResponse {
    pub ok: bool,
    pub psbt: String,

    /// Txid of the unsigned SegWit parent transaction.
    pub unsigned_txid: String,
    pub recompose_psbt: Option<ExtractRecomposePsbtResponse>,
    pub sign_inputs: BTreeMap<String, Vec<u32>>,

    pub miner_fee_sats: u64,
    pub recompose_miner_fee_sats: u64,
    pub total_miner_fee_sats: u64,

    pub ordinal_outputs: usize,
    pub tx_outputs: usize,

    pub payment_change_output_index: u32,
    pub payment_change_value: u64,

    pub extracted_outputs: Vec<ExtractedOutputRef>,
    pub recompose: Option<RecomposeIntent>,
}

pub fn run_extract_build_psbt(req: ExtractBuildPsbtRequest) -> Result<ExtractBuildPsbtResponse> {
    if req.payment_utxos.is_empty() {
        bail!("missing payment UTXOs");
    }

    /*
     * Execution boundary:
     * Extract is executable only against current Bitcoin Core + ord state.
     */
    let current_input = validate_current_output(&req.input_utxo)?;

    validate_extract_source(
        &current_input,
        req.total_value,
        &req.groups,
        &req.ordinals_address,
    )?;

    let mut payment_utxos = Vec::<Utxo>::with_capacity(req.payment_utxos.len());

    for requested in &req.payment_utxos {
        let current = validate_current_output(&requested.outpoint)?;

        validate_payment_source(&current, requested, &req.payment_address)?;

        payment_utxos.push(current_output_to_utxo(&current, &req.payment_address)?);
    }

    let plan = run_extract_plan(ExtractPlanRequest {
        input_utxo: req.input_utxo.clone(),
        ordinals_address: req.ordinals_address.clone(),
        total_value: req.total_value,
        groups: req.groups.clone(),
        extract_groups: req.extract_groups.clone(),
    })?;

    let payment_value = payment_utxos.iter().try_fold(0_u64, |total, utxo| {
        total
            .checked_add(utxo.value)
            .ok_or_else(|| anyhow!("payment value overflow"))
    })?;

    let miner_fee_sats = req.primary_miner_fee_sats;

    /*
     * When recompose is required, the child transaction uses:
     *
     * - all remainder outputs of the parent
     * - the payment change output of the parent
     *
     * Outputs:
     * - recomposed Ordinal-UTXO
     * - new payment change
     */
    let recompose_miner_fee_sats =
        resolve_recompose_miner_fee(plan.recompose.is_some(), req.recompose_miner_fee_sats)?;

    let required_payment = miner_fee_sats
        .checked_add(recompose_miner_fee_sats)
        .and_then(|value| value.checked_add(DUST_LIMIT))
        .ok_or_else(|| anyhow!("required payment overflow"))?;

    if payment_value < required_payment {
        bail!(
            "payment UTXOs too small: required at least {}, supplied {}",
            required_payment,
            payment_value
        );
    }

    /*
     * The parent pays only its own network fee.
     * The child fee initially remains in the parent change and is only
     * emitted by recompose.
     */
    let payment_change_value = payment_value
        .checked_sub(miner_fee_sats)
        .ok_or_else(|| anyhow!("invalid parent payment change"))?;

    if plan.recompose.is_some() && payment_change_value < recompose_miner_fee_sats + DUST_LIMIT {
        bail!("parent change too small for recompose");
    }

    if plan.recompose.is_none() && payment_change_value < DUST_LIMIT {
        bail!("payment change would be dust");
    }

    let mut outputs = Vec::<TxOut>::new();

    /*
     * These outputs must appear first and in the exact
     * sat order planned by BCE.
     */
    for output in &plan.outputs {
        outputs.push(TxOut {
            value: Amount::from_sat(output.value),
            script_pubkey: address_to_script(&output.address)?,
        });
    }

    let change_address = req
        .change_address
        .clone()
        .unwrap_or_else(|| req.payment_address.clone());

    let payment_change_output_index =
        u32::try_from(outputs.len()).map_err(|_| anyhow!("too many transaction outputs"))?;

    outputs.push(TxOut {
        value: Amount::from_sat(payment_change_value),
        script_pubkey: address_to_script(&change_address)?,
    });

    let mut inputs = Vec::<TxIn>::new();

    /*
     * The composition input must remain input 0.
     */
    inputs.push(build_txin(&req.input_utxo)?);

    for payment_utxo in &payment_utxos {
        inputs.push(build_txin(&payment_utxo.outpoint)?);
    }

    let tx = Transaction {
        version: transaction::Version(2),
        lock_time: absolute::LockTime::ZERO,
        input: inputs,
        output: outputs,
    };

    let unsigned_txid = tx.txid().to_string();
    let tx_outputs = tx.output.len();

    let mut psbt = Psbt::from_unsigned_tx(tx)?;

    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: Amount::from_sat(current_input.value),
        script_pubkey: ScriptBuf::from_hex(&current_input.script_pubkey)
            .map_err(|_| anyhow!("invalid current input scriptPubKey"))?,
    });

    if req.ordinals_address.starts_with("bc1p") {
        if let Some(public_key_hex) = req.ordinals_public_key.as_ref() {
            psbt.inputs[0].tap_internal_key = Some(xonly_from_pubkey_hex(public_key_hex)?);
        }
    }

    for (payment_index, payment_utxo) in payment_utxos.iter().enumerate() {
        let psbt_index = payment_index
            .checked_add(1)
            .ok_or_else(|| anyhow!("payment input index overflow"))?;

        psbt.inputs[psbt_index].witness_utxo = Some(TxOut {
            value: Amount::from_sat(payment_utxo.value),
            script_pubkey: address_to_script(&payment_utxo.address)?,
        });

        if payment_utxo.address.starts_with("bc1p") {
            if let Some(public_key_hex) = req.payment_public_key.as_ref() {
                psbt.inputs[psbt_index].tap_internal_key =
                    Some(xonly_from_pubkey_hex(public_key_hex)?);
            }
        }
    }

    let psbt_base64 = general_purpose::STANDARD.encode(psbt.serialize());

    let mut sign_inputs = BTreeMap::<String, Vec<u32>>::new();

    sign_inputs
        .entry(req.ordinals_address.clone())
        .or_default()
        .push(0);

    let payment_signing_indices: Vec<u32> = (1..=payment_utxos.len())
        .map(|index| u32::try_from(index).map_err(|_| anyhow!("payment signing index overflow")))
        .collect::<Result<Vec<_>>>()?;

    let recompose_psbt = match plan.recompose.as_ref() {
        Some(recompose) => {
            /*
             * The child txid may be derived from the unsigned parent only
             * when all parent inputs use SegWit.
             */
            if !is_segwit_address(&req.ordinals_address) {
                bail!("recompose requires a SegWit ordinals address");
            }

            for payment_utxo in &payment_utxos {
                if !is_segwit_address(&payment_utxo.address) {
                    bail!("recompose requires SegWit payment inputs");
                }
            }

            let result = build_recompose_psbt(RecomposeBuildRequest {
                parent_txid: unsigned_txid.clone(),
                parent_outputs: plan.outputs.clone(),
                recompose: recompose.clone(),

                payment_change_output_index,
                payment_change_value,

                ordinals_address: req.ordinals_address.clone(),

                /*
                 * The parent change belongs to the change address.
                 * Therefore this address must control the payment input
                 * used to sign the child transaction.
                 */
                payment_signing_address: change_address.clone(),
                change_address: change_address.clone(),

                ordinals_public_key: req.ordinals_public_key.clone(),
                payment_public_key: req.payment_public_key.clone(),

                miner_fee_sats: recompose_miner_fee_sats,
            })?;

            Some(ExtractRecomposePsbtResponse {
                psbt: result.psbt,
                unsigned_txid: result.unsigned_txid,
                sign_inputs: result.sign_inputs,
                miner_fee_sats: result.miner_fee_sats,
                change_output_index: result.change_output_index,
                change_value: result.change_value,
            })
        }

        None => None,
    };

    sign_inputs
        .entry(req.payment_address.clone())
        .or_default()
        .extend(payment_signing_indices);

    Ok(ExtractBuildPsbtResponse {
        ok: true,
        psbt: psbt_base64,
        unsigned_txid,
        sign_inputs,
        miner_fee_sats,
        recompose_miner_fee_sats,
        total_miner_fee_sats: miner_fee_sats
            .checked_add(recompose_miner_fee_sats)
            .ok_or_else(|| anyhow!("total miner fee overflow"))?,
        ordinal_outputs: plan.outputs.len(),
        tx_outputs,
        payment_change_output_index,
        payment_change_value,
        extracted_outputs: plan.extracted_outputs,
        recompose: plan.recompose,
        recompose_psbt,
    })
}

fn validate_extract_source(
    state: &CurrentOutputState,
    claimed_total: u64,
    groups: &[ExtractGroup],
    expected_address: &str,
) -> Result<()> {
    if state.value != claimed_total {
        bail!(
            "extract input {} value mismatch: request claimed {}, Bitcoin has {}",
            state.outpoint,
            claimed_total,
            state.value
        );
    }

    let address = state
        .address
        .as_deref()
        .ok_or_else(|| anyhow!("extract input {} has no current address", state.outpoint))?;

    if address != expected_address {
        bail!(
            "extract input {} address mismatch: expected {}, got {}",
            state.outpoint,
            expected_address,
            address
        );
    }

    for group in groups {
        let end = group
            .offset
            .checked_add(group.postage)
            .ok_or_else(|| anyhow!("extract group range overflow"))?;

        if end > state.value {
            bail!(
                "extract group at offset {} exceeds current UTXO value {}",
                group.offset,
                state.value
            );
        }

        for id in &group.ids {
            let current_offset = state
                .satpoints
                .iter()
                .find(|satpoint| satpoint.ids.iter().any(|current| current == id))
                .map(|satpoint| satpoint.offset)
                .ok_or_else(|| {
                    anyhow!(
                        "extract inscription {} is not present on current output {}",
                        id,
                        state.outpoint
                    )
                })?;

            if current_offset < group.offset || current_offset >= end {
                bail!(
                    "extract inscription {} moved: current offset {} is outside requested range {}..{}",
                    id,
                    current_offset,
                    group.offset,
                    end
                );
            }
        }
    }

    Ok(())
}

fn validate_payment_source(
    state: &CurrentOutputState,
    requested: &Utxo,
    expected_address: &str,
) -> Result<()> {
    if !state.satpoints.is_empty() {
        bail!("payment UTXO {} contains inscriptions", state.outpoint);
    }

    if state.value != requested.value {
        bail!(
            "payment UTXO {} value mismatch: request claimed {}, Bitcoin has {}",
            state.outpoint,
            requested.value,
            state.value
        );
    }

    if requested.address != expected_address {
        bail!(
            "payment UTXO {} request address does not match payment address",
            state.outpoint
        );
    }

    let address = state
        .address
        .as_deref()
        .ok_or_else(|| anyhow!("payment UTXO {} has no current address", state.outpoint))?;

    if address != expected_address {
        bail!(
            "payment UTXO {} current address mismatch: expected {}, got {}",
            state.outpoint,
            expected_address,
            address
        );
    }

    Ok(())
}

fn current_output_to_utxo(state: &CurrentOutputState, expected_address: &str) -> Result<Utxo> {
    let address = state
        .address
        .as_deref()
        .ok_or_else(|| anyhow!("current output {} has no address", state.outpoint))?;

    if address != expected_address {
        bail!(
            "current output {} address mismatch: expected {}, got {}",
            state.outpoint,
            expected_address,
            address
        );
    }

    /*
     * The address representation must resolve to exactly the scriptPubKey
     * reported by Bitcoin Core. This prevents address reconstruction from
     * becoming a second source of truth.
     */
    let address_script = address_to_script(address)?;
    let current_script = decode_hex(&state.script_pubkey)?;

    if address_script.as_bytes() != current_script.as_slice() {
        bail!(
            "current output {} scriptPubKey does not match current address",
            state.outpoint
        );
    }

    Ok(Utxo {
        outpoint: state.outpoint.clone(),
        value: state.value,
        address: address.to_string(),
        inscriptions: Vec::new(),
    })
}

fn build_txin(outpoint: &str) -> Result<TxIn> {
    Ok(TxIn {
        previous_output: OutPoint::from_str(outpoint)?,
        script_sig: ScriptBuf::new(),
        sequence: Sequence::MAX,
        witness: Witness::new(),
    })
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
        length => bail!("invalid public key length: {}", length),
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

fn is_segwit_address(address: &str) -> bool {
    address.to_ascii_lowercase().starts_with("bc1")
}

fn resolve_recompose_miner_fee(recompose_required: bool, supplied_fee: Option<u64>) -> Result<u64> {
    match (recompose_required, supplied_fee) {
        (true, Some(fee)) => Ok(fee),
        (true, None) => bail!("extract recompose requires a recompose miner fee"),
        (false, Some(_)) => {
            bail!("extract without recompose must not include a recompose miner fee")
        }
        (false, None) => Ok(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_guard::CurrentSatpoint;

    const ADDRESS: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";

    #[test]
    fn recompose_requires_its_own_miner_fee() {
        let error =
            resolve_recompose_miner_fee(true, None).expect_err("recompose fee must be required");

        assert_eq!(
            error.to_string(),
            "extract recompose requires a recompose miner fee"
        );
    }

    #[test]
    fn extract_without_recompose_rejects_recompose_fee() {
        let error = resolve_recompose_miner_fee(false, Some(1))
            .expect_err("unused recompose fee must be rejected");

        assert_eq!(
            error.to_string(),
            "extract without recompose must not include a recompose miner fee"
        );
    }

    fn state(value: u64, satpoints: Vec<(u64, Vec<&str>, u64)>) -> CurrentOutputState {
        CurrentOutputState {
            outpoint: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:0"
                .to_string(),
            value,
            script_pubkey: String::new(),
            address: Some(ADDRESS.to_string()),
            satpoints: satpoints
                .into_iter()
                .map(|(offset, ids, postage)| CurrentSatpoint {
                    ids: ids.into_iter().map(str::to_string).collect(),
                    offset,
                    postage,
                })
                .collect(),
        }
    }

    #[test]
    fn rejects_extract_total_value_mismatch() {
        let current = state(1000, vec![(0, vec!["A"], 500), (500, vec!["B"], 500)]);

        let groups = vec![
            ExtractGroup {
                ids: vec!["A".to_string()],
                offset: 0,
                postage: 500,
            },
            ExtractGroup {
                ids: vec!["B".to_string()],
                offset: 500,
                postage: 500,
            },
        ];

        let error = validate_extract_source(&current, 999, &groups, ADDRESS)
            .expect_err("stale extract total must fail");

        assert!(error
            .to_string()
            .contains("request claimed 999, Bitcoin has 1000"));
    }

    #[test]
    fn rejects_inscription_outside_extract_range() {
        let current = state(1000, vec![(0, vec!["A"], 700), (700, vec!["B"], 300)]);

        let groups = vec![
            ExtractGroup {
                ids: vec!["A".to_string(), "B".to_string()],
                offset: 0,
                postage: 500,
            },
            ExtractGroup {
                ids: vec!["C".to_string()],
                offset: 500,
                postage: 500,
            },
        ];

        let error = validate_extract_source(&current, 1000, &groups, ADDRESS)
            .expect_err("moved inscription must fail");

        assert!(error.to_string().contains("outside requested range"));
    }

    #[test]
    fn accepts_shared_satpoint_inside_extract_range() {
        let current = state(1000, vec![(0, vec!["A", "B"], 500), (500, vec!["C"], 500)]);

        let groups = vec![
            ExtractGroup {
                ids: vec!["A".to_string(), "B".to_string()],
                offset: 0,
                postage: 500,
            },
            ExtractGroup {
                ids: vec!["C".to_string()],
                offset: 500,
                postage: 500,
            },
        ];

        validate_extract_source(&current, 1000, &groups, ADDRESS)
            .expect("shared satpoint must remain valid");
    }
}
