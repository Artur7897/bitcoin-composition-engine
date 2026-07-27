use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose, Engine as _};
use bitcoin::{
    absolute, address::Address, psbt::Psbt, transaction, Amount, Network, OutPoint, ScriptBuf,
    Sequence, Transaction, TxIn, TxOut, Witness,
};
use std::{collections::BTreeMap, str::FromStr};

use crate::fees::{estimate_network_fee, service_fee_sats, DUST_LIMIT, SERVICE_FEE_ADDRESS};
use crate::split_plan::run_split_plan;
use crate::split_types::{SplitBuildPsbtRequest, SplitBuildPsbtResponse, SplitPlanRequest};

pub fn run_split_build_psbt(req: SplitBuildPsbtRequest) -> Result<SplitBuildPsbtResponse> {
    validate_request(&req)?;

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

    let payment_value = req.payment_utxos.iter().try_fold(0_u64, |total, utxo| {
        total
            .checked_add(utxo.value)
            .ok_or_else(|| anyhow!("payment value overflow"))
    })?;

    let input_count = 1_usize
        .checked_add(req.payment_utxos.len())
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

    let mut inputs = Vec::<TxIn>::new();

    /*
     * Composition-Input bleibt zwingend Input 0.
     */
    inputs.push(build_txin(&req.input_utxo)?);

    for payment_utxo in &req.payment_utxos {
        inputs.push(build_txin(&payment_utxo.outpoint)?);
    }

    let tx = Transaction {
        version: transaction::Version(2),
        lock_time: absolute::LockTime::ZERO,
        input: inputs,
        output: outputs,
    };

    let tx_outputs = tx.output.len();

    let mut psbt = Psbt::from_unsigned_tx(tx)?;

    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: Amount::from_sat(req.total_value),
        script_pubkey: address_to_script(&req.ordinals_address)?,
    });

    if req.ordinals_address.starts_with("bc1p") {
        if let Some(public_key_hex) = req.ordinals_public_key.as_ref() {
            psbt.inputs[0].tap_internal_key = Some(xonly_from_pubkey_hex(public_key_hex)?);
        }
    }

    for (payment_index, payment_utxo) in req.payment_utxos.iter().enumerate() {
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
        sign_inputs,
        network_fee,
        service_fee,
        total,
        outputs: plan.outputs.len(),
        tx_outputs,
        vsize,
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
