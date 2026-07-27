use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose, Engine as _};
use bitcoin::{
    absolute::LockTime, address::Address, psbt::Psbt, transaction::Version, Amount, Network,
    OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
};
use std::{collections::BTreeMap, str::FromStr};

use crate::compose_types::{ComposeBuildPsbtRequest, ComposeBuildPsbtResponse};
use crate::fees::{estimate_network_fee, service_fee_sats, DUST_LIMIT, SERVICE_FEE_ADDRESS};
use crate::models::Utxo;

pub fn run_compose_build_psbt(req: ComposeBuildPsbtRequest) -> Result<ComposeBuildPsbtResponse> {
    validate_request(&req)?;

    let service_fee = service_fee_sats(req.payment_method.as_deref())?;

    let root_utxo = build_utxo(
        req.root_utxo.clone(),
        req.root_postage,
        req.ordinals_address.clone(),
    )?;

    let item_utxos: Vec<Utxo> = req
        .items
        .iter()
        .map(|item| {
            let outpoint = item
                .utxo
                .clone()
                .ok_or_else(|| anyhow!("missing item UTXO for {}", item.id))?;

            build_utxo(outpoint, item.postage, req.ordinals_address.clone())
        })
        .collect::<Result<Vec<_>>>()?;

    let mut all_utxos = Vec::<Utxo>::new();
    all_utxos.push(root_utxo.clone());
    all_utxos.extend(item_utxos.clone());
    all_utxos.extend(req.payment_utxos.clone());

    let mut inputs = Vec::<TxIn>::new();

    for utxo in &all_utxos {
        inputs.push(build_txin(utxo)?);
    }

    let ordinals_value = root_utxo
        .value
        .checked_add(item_utxos.iter().try_fold(0_u64, |total, utxo| {
            total
                .checked_add(utxo.value)
                .ok_or_else(|| anyhow!("ordinal value overflow"))
        })?)
        .ok_or_else(|| anyhow!("composition value overflow"))?;

    let payment_value = req.payment_utxos.iter().try_fold(0_u64, |total, utxo| {
        total
            .checked_add(utxo.value)
            .ok_or_else(|| anyhow!("payment value overflow"))
    })?;

    let (vsize, network_fee) = estimate_network_fee(inputs.len(), 3, req.fee_rate.unwrap_or(1))?;

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

    let outputs = vec![
        TxOut {
            value: Amount::from_sat(ordinals_value),
            script_pubkey: address_to_script(&receive_address)?,
        },
        TxOut {
            value: Amount::from_sat(service_fee),
            script_pubkey: address_to_script(SERVICE_FEE_ADDRESS)?,
        },
        TxOut {
            value: Amount::from_sat(change_value),
            script_pubkey: address_to_script(&change_address)?,
        },
    ];

    let tx = Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input: inputs,
        output: outputs,
    };

    let mut psbt = Psbt::from_unsigned_tx(tx)?;

    for (index, utxo) in all_utxos.iter().enumerate() {
        let is_ordinal = index < 1 + item_utxos.len();

        psbt.inputs[index].witness_utxo = Some(TxOut {
            value: Amount::from_sat(utxo.value),
            script_pubkey: address_to_script(&utxo.address)?,
        });

        let public_key = if is_ordinal {
            req.ordinals_public_key.as_ref()
        } else {
            req.payment_public_key.as_ref()
        };

        if utxo.address.starts_with("bc1p") {
            if let Some(public_key_hex) = public_key {
                psbt.inputs[index].tap_internal_key = Some(xonly_from_pubkey_hex(public_key_hex)?);
            }
        }
    }

    let psbt_base64 = general_purpose::STANDARD.encode(psbt.serialize());

    let mut sign_inputs = BTreeMap::<String, Vec<u32>>::new();

    let ordinal_input_count = 1 + item_utxos.len();

    let ordinal_indices: Vec<u32> = (0..ordinal_input_count)
        .map(|index| u32::try_from(index).map_err(|_| anyhow!("ordinal signing index overflow")))
        .collect::<Result<Vec<_>>>()?;

    sign_inputs
        .entry(req.ordinals_address.clone())
        .or_default()
        .extend(ordinal_indices);

    let payment_indices: Vec<u32> = (ordinal_input_count..all_utxos.len())
        .map(|index| u32::try_from(index).map_err(|_| anyhow!("payment signing index overflow")))
        .collect::<Result<Vec<_>>>()?;

    sign_inputs
        .entry(req.payment_address.clone())
        .or_default()
        .extend(payment_indices);

    let mut planned_offsets = Vec::<u64>::with_capacity(req.items.len());

    let mut next_offset = req.root_postage;

    for item in &req.items {
        planned_offsets.push(next_offset);

        next_offset = next_offset
            .checked_add(item.postage)
            .ok_or_else(|| anyhow!("compose offset overflow"))?;
    }

    let total = network_fee
        .checked_add(service_fee)
        .ok_or_else(|| anyhow!("total fee overflow"))?;

    let _ = vsize;

    Ok(ComposeBuildPsbtResponse {
        ok: true,
        psbt: psbt_base64,
        sign_inputs,
        network_fee,
        service_fee,
        total,
        planned_offsets,
    })
}

fn validate_request(req: &ComposeBuildPsbtRequest) -> Result<()> {
    if req.root_id.trim().is_empty() {
        bail!("missing root ID");
    }

    if req.root_utxo.trim().is_empty() {
        bail!("missing root UTXO");
    }

    if req.root_postage == 0 {
        bail!("root postage must be greater than zero");
    }

    if req.items.is_empty() {
        bail!("compose requires at least one item");
    }

    if req.ordinals_address.trim().is_empty() {
        bail!("missing ordinals address");
    }

    if req.payment_address.trim().is_empty() {
        bail!("missing payment address");
    }

    if req.payment_utxos.is_empty() {
        bail!("missing payment UTXOs");
    }

    for item in &req.items {
        if item.id.trim().is_empty() {
            bail!("compose item contains empty ID");
        }

        if item.postage == 0 {
            bail!("compose item {} has zero postage", item.id);
        }

        if item
            .utxo
            .as_deref()
            .map(str::trim)
            .map(|value| value.is_empty())
            .unwrap_or(true)
        {
            bail!("missing item UTXO for {}", item.id);
        }
    }

    for payment_utxo in &req.payment_utxos {
        if payment_utxo.address != req.payment_address {
            bail!("payment UTXO address does not match payment address");
        }

        if !payment_utxo.inscriptions.is_empty() {
            bail!("payment UTXO contains inscriptions");
        }
    }

    Ok(())
}

fn build_utxo(outpoint: String, value: u64, address: String) -> Result<Utxo> {
    if value == 0 {
        bail!("UTXO has zero value");
    }

    Ok(Utxo {
        outpoint,
        value,
        address,
        inscriptions: Vec::new(),
    })
}

fn build_txin(utxo: &Utxo) -> Result<TxIn> {
    Ok(TxIn {
        previous_output: OutPoint::from_str(&utxo.outpoint)?,
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
