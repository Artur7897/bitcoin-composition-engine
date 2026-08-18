use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose, Engine as _};
use bitcoin::{
    absolute::LockTime, address::Address, psbt::Psbt, transaction::Version, Amount, Network,
    OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
};
use std::{
    collections::{BTreeMap, HashSet},
    str::FromStr,
};

use crate::compose_types::{ComposeBuildPsbtRequest, ComposeBuildPsbtResponse};
use crate::execution_guard::{validate_current_output, CurrentOutputState};
use crate::fees::{estimate_network_fee, service_fee_sats, DUST_LIMIT, SERVICE_FEE_ADDRESS};
use crate::models::Utxo;

pub fn run_compose_build_psbt(req: ComposeBuildPsbtRequest) -> Result<ComposeBuildPsbtResponse> {
    validate_request(&req)?;

    let service_fee = service_fee_sats(req.payment_method.as_deref())?;

    /*
     * Execution boundary:
     *
     * Request/plan values are never authoritative here.
     * Every real transaction input is re-read from Bitcoin Core + ord
     * immediately before PSBT construction.
     */
    let root_state = validate_current_output(&req.root_utxo)?;

    validate_ordinal_source(
        &root_state,
        &req.root_id,
        req.root_postage,
        &req.ordinals_address,
    )?;

    let root_utxo = current_output_to_utxo(&root_state, &req.ordinals_address)?;

    let mut item_utxos = Vec::<Utxo>::with_capacity(req.items.len());

    for item in &req.items {
        let outpoint = item
            .utxo
            .as_deref()
            .ok_or_else(|| anyhow!("missing item UTXO for {}", item.id))?;

        let state = validate_current_output(outpoint)?;

        validate_ordinal_source(&state, &item.id, item.postage, &req.ordinals_address)?;

        item_utxos.push(current_output_to_utxo(&state, &req.ordinals_address)?);
    }

    let mut payment_utxos = Vec::<Utxo>::with_capacity(req.payment_utxos.len());

    for requested in &req.payment_utxos {
        let state = validate_current_output(&requested.outpoint)?;

        validate_payment_source(&state, requested, &req.payment_address)?;

        payment_utxos.push(current_output_to_utxo(&state, &req.payment_address)?);
    }

    let mut all_utxos = Vec::<Utxo>::new();
    all_utxos.push(root_utxo.clone());
    all_utxos.extend(item_utxos.clone());
    all_utxos.extend(payment_utxos.clone());

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

    let payment_value = payment_utxos.iter().try_fold(0_u64, |total, utxo| {
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

    let mut planned_offsets = Vec::<u64>::with_capacity(item_utxos.len());

    let mut next_offset = root_utxo.value;

    for item_utxo in &item_utxos {
        planned_offsets.push(next_offset);

        next_offset = next_offset
            .checked_add(item_utxo.value)
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

    /*
     * Every inscription ID participating in the composition
     * must be unique.
     */
    let mut seen_ids = HashSet::<String>::new();

    if !seen_ids.insert(req.root_id.clone()) {
        bail!("duplicate compose inscription ID: {}", req.root_id);
    }

    /*
     * Every transaction input must reference a unique outpoint.
     *
     * This also prevents an ordinal UTXO from accidentally being
     * reused as a payment UTXO.
     */
    let mut seen_outpoints = HashSet::<String>::new();

    if !seen_outpoints.insert(req.root_utxo.clone()) {
        bail!("duplicate compose input UTXO: {}", req.root_utxo);
    }

    for item in &req.items {
        if item.id.trim().is_empty() {
            bail!("compose item contains empty ID");
        }

        if item.postage == 0 {
            bail!("compose item {} has zero postage", item.id);
        }

        if !seen_ids.insert(item.id.clone()) {
            bail!("duplicate compose inscription ID: {}", item.id);
        }

        let item_utxo = item
            .utxo
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("missing item UTXO for {}", item.id))?;

        if !seen_outpoints.insert(item_utxo.to_string()) {
            bail!("duplicate compose input UTXO: {}", item_utxo);
        }
    }

    for payment_utxo in &req.payment_utxos {
        if payment_utxo.outpoint.trim().is_empty() {
            bail!("payment UTXO contains empty outpoint");
        }

        if payment_utxo.value == 0 {
            bail!("payment UTXO has zero value");
        }

        if payment_utxo.address != req.payment_address {
            bail!("payment UTXO address does not match payment address");
        }

        if !payment_utxo.inscriptions.is_empty() {
            bail!("payment UTXO contains inscriptions");
        }

        if !seen_outpoints.insert(payment_utxo.outpoint.clone()) {
            bail!("duplicate compose input UTXO: {}", payment_utxo.outpoint);
        }
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

    if state.value == 0 {
        bail!("current output {} has zero value", state.outpoint);
    }

    Ok(Utxo {
        outpoint: state.outpoint.clone(),
        value: state.value,
        address: address.to_string(),
        inscriptions: Vec::new(),
    })
}

fn current_output_contains_id(state: &CurrentOutputState, id: &str) -> bool {
    state
        .satpoints
        .iter()
        .any(|satpoint| satpoint.ids.iter().any(|current| current == id))
}

fn validate_ordinal_source(
    state: &CurrentOutputState,
    expected_id: &str,
    claimed_value: u64,
    expected_address: &str,
) -> Result<()> {
    if !current_output_contains_id(state, expected_id) {
        bail!(
            "inscription {} is not present on current output {}",
            expected_id,
            state.outpoint
        );
    }

    if state.value != claimed_value {
        bail!(
            "current output {} value mismatch: request claimed {}, Bitcoin has {}",
            state.outpoint,
            claimed_value,
            state.value
        );
    }

    let address = state
        .address
        .as_deref()
        .ok_or_else(|| anyhow!("current output {} has no address", state.outpoint))?;

    if address != expected_address {
        bail!(
            "current ordinal output {} address mismatch: expected {}, got {}",
            state.outpoint,
            expected_address,
            address
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_guard::CurrentSatpoint;

    const ADDRESS: &str = "bc1qznl7wxgtemt5eprmr6g3yj7nn7xh5gtzuvezuz";

    fn state(value: u64, ids: Vec<&str>) -> CurrentOutputState {
        CurrentOutputState {
            outpoint: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:0"
                .to_string(),
            value,
            script_pubkey: String::new(),
            address: Some(ADDRESS.to_string()),
            satpoints: vec![CurrentSatpoint {
                ids: ids.into_iter().map(str::to_string).collect(),
                offset: 0,
                postage: value,
            }],
        }
    }

    #[test]
    fn rejects_claimed_value_mismatch() {
        let current = state(1000, vec!["ID-A"]);

        let error = validate_ordinal_source(&current, "ID-A", 546, ADDRESS)
            .expect_err("mismatched claimed value must fail");

        assert!(error
            .to_string()
            .contains("request claimed 546, Bitcoin has 1000"));
    }

    #[test]
    fn rejects_missing_expected_inscription() {
        let current = state(1000, vec!["ID-B"]);

        let error = validate_ordinal_source(&current, "ID-A", 1000, ADDRESS)
            .expect_err("missing expected inscription must fail");

        assert!(error
            .to_string()
            .contains("is not present on current output"));
    }

    #[test]
    fn accepts_expected_id_on_shared_satpoint() {
        let current = state(1000, vec!["ID-A", "ID-B"]);

        validate_ordinal_source(&current, "ID-A", 1000, ADDRESS)
            .expect("expected ID on shared satpoint must remain valid");
    }
}
