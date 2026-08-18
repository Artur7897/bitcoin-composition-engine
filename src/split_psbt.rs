use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose, Engine as _};
use bitcoin::{
    absolute, address::Address, psbt::Psbt, transaction, Amount, Network, OutPoint, ScriptBuf,
    Sequence, Transaction, TxIn, TxOut, Witness,
};
use std::{collections::BTreeMap, str::FromStr};

use crate::execution_guard::{validate_current_output, CurrentOutputState};
use crate::fees::{estimate_network_fee, service_fee_sats, DUST_LIMIT, SERVICE_FEE_ADDRESS};
use crate::split_plan::run_split_plan;
use crate::split_types::{
    PaymentUtxo, SplitBuildPsbtRequest, SplitBuildPsbtResponse, SplitGroup, SplitPlanRequest,
};

pub fn run_split_build_psbt(req: SplitBuildPsbtRequest) -> Result<SplitBuildPsbtResponse> {
    validate_request(&req)?;

    /*
     * Execution boundary:
     * Split is rebuilt only from current Bitcoin Core + ord state.
     */
    let current_input = validate_current_output(&req.input_utxo)?;

    validate_split_source(
        &current_input,
        req.total_value,
        &req.groups,
        &req.ordinals_address,
    )?;

    let mut payment_utxos = Vec::<PaymentUtxo>::with_capacity(req.payment_utxos.len());

    for requested in &req.payment_utxos {
        let current = validate_current_output(&requested.outpoint)?;

        validate_payment_source(&current, requested, &req.payment_address)?;

        payment_utxos.push(current_output_to_payment(&current, &req.payment_address)?);
    }

    let fee_rate = req.fee_rate.unwrap_or(1).max(1);

    let service_fee = service_fee_sats(req.payment_method.as_deref())?;

    let plan = run_split_plan(SplitPlanRequest {
        input_utxo: req.input_utxo.clone(),
        ordinals_address: req.ordinals_address.clone(),
        fee_rate: Some(fee_rate),
        total_value: req.total_value,
        groups: req.groups.clone(),
        payment_method: req.payment_method.clone(),
    })?;

    let payment_value = payment_utxos.iter().try_fold(0_u64, |total, utxo| {
        total
            .checked_add(utxo.value)
            .ok_or_else(|| anyhow!("payment value overflow"))
    })?;

    let input_count = 1_usize
        .checked_add(payment_utxos.len())
        .ok_or_else(|| anyhow!("split input count overflow"))?;

    /*
     * Gruppenoutputs + Service Fee + Change.
     */
    let output_count = plan
        .outputs
        .len()
        .checked_add(2)
        .ok_or_else(|| anyhow!("split output count overflow"))?;

    let (vsize, network_fee) = estimate_network_fee(input_count, output_count, fee_rate)?;

    let required_payment = network_fee
        .checked_add(service_fee)
        .and_then(|value| value.checked_add(DUST_LIMIT))
        .ok_or_else(|| anyhow!("required payment overflow"))?;

    if payment_value < required_payment {
        bail!(
            "payment UTXOs too small: required at least {}, supplied {}",
            required_payment,
            payment_value
        );
    }

    let change_value = payment_value
        .checked_sub(network_fee)
        .and_then(|value| value.checked_sub(service_fee))
        .ok_or_else(|| anyhow!("invalid payment change"))?;

    if change_value < DUST_LIMIT {
        bail!("payment change would be dust");
    }

    let receive_address = req
        .receive_address
        .clone()
        .unwrap_or_else(|| req.ordinals_address.clone());

    let change_address = req
        .change_address
        .clone()
        .unwrap_or_else(|| req.payment_address.clone());

    let mut outputs = Vec::<TxOut>::new();

    /*
     * Die Split-Gruppen müssen zuerst und in Offset-Reihenfolge
     * ausgegeben werden.
     */
    for group in &plan.outputs {
        outputs.push(TxOut {
            value: Amount::from_sat(group.value),
            script_pubkey: address_to_script(&receive_address)?,
        });
    }

    outputs.push(TxOut {
        value: Amount::from_sat(service_fee),
        script_pubkey: address_to_script(SERVICE_FEE_ADDRESS)?,
    });

    outputs.push(TxOut {
        value: Amount::from_sat(change_value),
        script_pubkey: address_to_script(&change_address)?,
    });

    /*
     * unsigned_txid is the Build -> Sign -> Broadcast commitment.
     * All real inputs therefore have to be SegWit.
     */
    if !is_segwit_address(&req.ordinals_address) {
        bail!("split broadcast commitment requires SegWit ordinal input");
    }

    for payment_utxo in &payment_utxos {
        if !is_segwit_address(&payment_utxo.address) {
            bail!(
                "split broadcast commitment requires SegWit payment input {}",
                payment_utxo.outpoint
            );
        }
    }

    let mut inputs = Vec::<TxIn>::new();

    /*
     * Composition-Input bleibt zwingend Input 0.
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

    let tx_outputs = tx.output.len();
    let unsigned_txid = tx.txid().to_string();

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

    let payment_indices: Vec<u32> = (1..input_count)
        .map(|index| u32::try_from(index).map_err(|_| anyhow!("payment signing index overflow")))
        .collect::<Result<Vec<_>>>()?;

    sign_inputs
        .entry(req.payment_address.clone())
        .or_default()
        .extend(payment_indices);

    let total = network_fee
        .checked_add(service_fee)
        .ok_or_else(|| anyhow!("total fee overflow"))?;

    Ok(SplitBuildPsbtResponse {
        ok: true,
        psbt: psbt_base64,
        unsigned_txid,
        sign_inputs,
        network_fee,
        service_fee,
        total,
        outputs: plan.outputs.len(),
        tx_outputs,
        vsize,
    })
}

fn validate_split_source(
    state: &CurrentOutputState,
    claimed_total: u64,
    groups: &[SplitGroup],
    expected_address: &str,
) -> Result<()> {
    if state.value != claimed_total {
        bail!(
            "split input {} value mismatch: request claimed {}, Bitcoin has {}",
            state.outpoint,
            claimed_total,
            state.value
        );
    }

    let address = state
        .address
        .as_deref()
        .ok_or_else(|| anyhow!("split input {} has no current address", state.outpoint))?;

    if address != expected_address {
        bail!(
            "split input {} address mismatch: expected {}, got {}",
            state.outpoint,
            expected_address,
            address
        );
    }

    for group in groups {
        let end = group
            .offset
            .checked_add(group.value)
            .ok_or_else(|| anyhow!("split group range overflow"))?;

        if end > state.value {
            bail!(
                "split group at offset {} exceeds current UTXO value {}",
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
                        "split inscription {} is not present on current output {}",
                        id,
                        state.outpoint
                    )
                })?;

            if current_offset < group.offset || current_offset >= end {
                bail!(
                    "split inscription {} moved: current offset {} is outside requested range {}..{}",
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
    requested: &PaymentUtxo,
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
        bail!("payment UTXO request address does not match payment address");
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

fn current_output_to_payment(
    state: &CurrentOutputState,
    expected_address: &str,
) -> Result<PaymentUtxo> {
    let address = state
        .address
        .as_deref()
        .ok_or_else(|| anyhow!("payment UTXO {} has no current address", state.outpoint))?;

    if address != expected_address {
        bail!(
            "payment UTXO {} address mismatch: expected {}, got {}",
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

    Ok(PaymentUtxo {
        outpoint: state.outpoint.clone(),
        value: state.value,
        address: address.to_string(),
    })
}

fn validate_request(req: &SplitBuildPsbtRequest) -> Result<()> {
    if req.input_utxo.trim().is_empty() {
        bail!("missing input UTXO");
    }

    if req.ordinals_address.trim().is_empty() {
        bail!("missing ordinals address");
    }

    if req.payment_address.trim().is_empty() {
        bail!("missing payment address");
    }

    if req.total_value == 0 {
        bail!("input UTXO has zero value");
    }

    if req.groups.len() < 2 {
        bail!("not composed");
    }

    if req.payment_utxos.is_empty() {
        bail!("missing payment UTXOs");
    }

    for payment_utxo in &req.payment_utxos {
        if payment_utxo.address != req.payment_address {
            bail!("payment UTXO address does not match payment address");
        }

        if payment_utxo.value == 0 {
            bail!("payment UTXO has zero value");
        }
    }

    Ok(())
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
    fn rejects_split_total_value_mismatch() {
        let current = state(1000, vec![(0, vec!["A"], 500), (500, vec!["B"], 500)]);

        let groups = vec![
            SplitGroup {
                ids: vec!["A".to_string()],
                offset: 0,
                value: 500,
            },
            SplitGroup {
                ids: vec!["B".to_string()],
                offset: 500,
                value: 500,
            },
        ];

        let error = validate_split_source(&current, 999, &groups, ADDRESS)
            .expect_err("stale split total must fail");

        assert!(error
            .to_string()
            .contains("request claimed 999, Bitcoin has 1000"));
    }

    #[test]
    fn rejects_inscription_outside_requested_split_range() {
        let current = state(1000, vec![(0, vec!["A"], 700), (700, vec!["B"], 300)]);

        let groups = vec![
            SplitGroup {
                ids: vec!["A".to_string(), "B".to_string()],
                offset: 0,
                value: 500,
            },
            SplitGroup {
                ids: vec!["C".to_string()],
                offset: 500,
                value: 500,
            },
        ];

        let error = validate_split_source(&current, 1000, &groups, ADDRESS)
            .expect_err("moved inscription must fail");

        assert!(error.to_string().contains("outside requested range"));
    }

    #[test]
    fn accepts_shared_satpoint_inside_split_range() {
        let current = state(1000, vec![(0, vec!["A", "B"], 500), (500, vec!["C"], 500)]);

        let groups = vec![
            SplitGroup {
                ids: vec!["A".to_string(), "B".to_string()],
                offset: 0,
                value: 500,
            },
            SplitGroup {
                ids: vec!["C".to_string()],
                offset: 500,
                value: 500,
            },
        ];

        validate_split_source(&current, 1000, &groups, ADDRESS)
            .expect("shared satpoint must remain valid");
    }
}

fn is_segwit_address(address: &str) -> bool {
    address.to_ascii_lowercase().starts_with("bc1")
}
