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
use crate::fees::{estimate_network_fee, service_fee_sats, DUST_LIMIT, SERVICE_FEE_ADDRESS};
use crate::models::Utxo;
use crate::recompose_psbt::{build_recompose_psbt, estimate_recompose_fee, RecomposeBuildRequest};

#[derive(Debug, Serialize)]
pub struct ExtractRecomposePsbtResponse {
    pub psbt: String,
    pub unsigned_txid: String,
    pub sign_inputs: BTreeMap<String, Vec<u32>>,
    pub network_fee: u64,
    pub vsize: u64,
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
    pub fee_rate: Option<u64>,
    pub payment_method: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExtractBuildPsbtResponse {
    pub ok: bool,
    pub psbt: String,

    /// Txid der unsignierten SegWit-Parent-Transaktion.
    pub unsigned_txid: String,
    pub recompose_psbt: Option<ExtractRecomposePsbtResponse>,
    pub sign_inputs: BTreeMap<String, Vec<u32>>,

    pub network_fee: u64,
    pub recompose_network_fee: u64,
    pub service_fee: u64,
    pub total: u64,

    pub vsize: u64,
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

    let fee_rate = req.fee_rate.unwrap_or(1).max(1);

    let plan = run_extract_plan(ExtractPlanRequest {
        input_utxo: req.input_utxo.clone(),
        ordinals_address: req.ordinals_address.clone(),
        total_value: req.total_value,
        groups: req.groups.clone(),
        extract_groups: req.extract_groups.clone(),
    })?;

    let service_fee = service_fee_sats(req.payment_method.as_deref())?;

    let payment_value = payment_utxos.iter().try_fold(0_u64, |total, utxo| {
        total
            .checked_add(utxo.value)
            .ok_or_else(|| anyhow!("payment value overflow"))
    })?;

    let parent_input_count = 1_usize
        .checked_add(payment_utxos.len())
        .ok_or_else(|| anyhow!("parent input count overflow"))?;

    let parent_output_count = plan
        .outputs
        .len()
        .checked_add(if service_fee > 0 { 1 } else { 0 })
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| anyhow!("parent output count overflow"))?;

    let (parent_vsize, network_fee) =
        estimate_network_fee(parent_input_count, parent_output_count, fee_rate)?;

    /*
     * Wenn Recompose erforderlich ist, verwendet die Child-Transaktion:
     *
     * - alle Remainder-Outputs des Parents
     * - den Payment-Change-Output des Parents
     *
     * Outputs:
     * - recomposed Ordinal-UTXO
     * - neuer Payment-Change
     */
    let recompose_network_fee = match plan.recompose.as_ref() {
        Some(recompose) => {
            let (_, fee) = estimate_recompose_fee(recompose.input_output_indices.len(), fee_rate)?;

            fee
        }

        None => 0,
    };

    let required_payment = network_fee
        .checked_add(recompose_network_fee)
        .and_then(|value| value.checked_add(service_fee))
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
     * Der Parent bezahlt nur seine eigene Network Fee und die Service Fee.
     * Die Child Fee bleibt zunächst im Parent-Change und wird erst von
     * Recompose ausgegeben.
     */
    let payment_change_value = payment_value
        .checked_sub(network_fee)
        .and_then(|value| value.checked_sub(service_fee))
        .ok_or_else(|| anyhow!("invalid parent payment change"))?;

    if plan.recompose.is_some() && payment_change_value < recompose_network_fee + DUST_LIMIT {
        bail!("parent change too small for recompose");
    }

    if plan.recompose.is_none() && payment_change_value < DUST_LIMIT {
        bail!("payment change would be dust");
    }

    let mut outputs = Vec::<TxOut>::new();

    /*
     * Diese Outputs müssen zuerst und exakt in der vom Core geplanten
     * Sat-Reihenfolge erscheinen.
     */
    for output in &plan.outputs {
        outputs.push(TxOut {
            value: Amount::from_sat(output.value),
            script_pubkey: address_to_script(&output.address)?,
        });
    }

    if service_fee > 0 {
        outputs.push(TxOut {
            value: Amount::from_sat(service_fee),
            script_pubkey: address_to_script(SERVICE_FEE_ADDRESS)?,
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
     * Der Composition-Input muss zwingend Input 0 bleiben.
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
             * Der Child-Txid darf aus dem unsignierten Parent nur dann
             * abgeleitet werden, wenn alle Parent-Inputs SegWit verwenden.
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
                 * Der Parent-Change gehört der Change-Adresse.
                 * Deshalb muss diese Adresse den Payment-Input
                 * der Child-Transaktion signieren.
                 */
                payment_signing_address: change_address.clone(),
                change_address: change_address.clone(),

                ordinals_public_key: req.ordinals_public_key.clone(),
                payment_public_key: req.payment_public_key.clone(),

                fee_rate,
            })?;

            if result.network_fee != recompose_network_fee {
                bail!(
                    "recompose fee mismatch: reserved {}, built {}",
                    recompose_network_fee,
                    result.network_fee
                );
            }

            Some(ExtractRecomposePsbtResponse {
                psbt: result.psbt,
                unsigned_txid: result.unsigned_txid,
                sign_inputs: result.sign_inputs,
                network_fee: result.network_fee,
                vsize: result.vsize,
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
        network_fee,
        recompose_network_fee,
        service_fee,
        total: network_fee
            .checked_add(recompose_network_fee)
            .and_then(|value| value.checked_add(service_fee))
            .ok_or_else(|| anyhow!("total fee overflow"))?,
        vsize: parent_vsize,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_guard::CurrentSatpoint;

    const ADDRESS: &str = "bc1qznl7wxgtemt5eprmr6g3yj7nn7xh5gtzuvezuz";

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
