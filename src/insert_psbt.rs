use anyhow::{anyhow, bail, Result};
use base64::{engine::general_purpose, Engine as _};
use bitcoin::{
    absolute, address::Address, psbt::Psbt, transaction, Amount, Network, OutPoint, ScriptBuf,
    Sequence, Transaction, TxIn, TxOut, Witness,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, str::FromStr};

use crate::execution_guard::{validate_current_output, CurrentOutputState};
use crate::insert::{
    run_insert_plan, ExistingInsertGroup, InsertPlanMode, InsertPlanRequest, NewInsertGroup,
    PlannedInsertGroup,
};
use crate::models::Utxo;

use crate::fees::{estimate_network_fee, service_fee_sats, DUST_LIMIT, SERVICE_FEE_ADDRESS};

#[derive(Debug, Clone, Deserialize)]
pub struct InsertBuildPsbtRequest {
    pub input_utxo: String,
    pub ordinals_address: String,
    pub payment_address: String,
    pub change_address: Option<String>,

    pub ordinals_public_key: Option<String>,
    pub payment_public_key: Option<String>,

    pub total_value: u64,
    pub existing_groups: Vec<ExistingInsertGroup>,
    pub insert_groups: Vec<NewInsertGroup>,
    pub ordered_groups: Vec<Vec<String>>,

    pub payment_utxos: Vec<Utxo>,
    pub fee_rate: Option<u64>,
    pub payment_method: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InsertPsbtTransaction {
    pub psbt: String,
    pub unsigned_txid: String,
    pub sign_inputs: BTreeMap<String, Vec<u32>>,

    pub network_fee: u64,
    pub vsize: u64,

    pub ordinal_inputs: usize,
    pub ordinal_outputs: usize,
    pub tx_outputs: usize,

    pub payment_change_output_index: u32,
    pub payment_change_value: u64,
}

struct InsertExecutionState {
    current_input: CurrentOutputState,
    inserted_inputs: BTreeMap<String, CurrentOutputState>,
    payment_inputs: Vec<CurrentOutputState>,
}

#[derive(Debug, Serialize)]
pub struct InsertBuildPsbtResponse {
    pub ok: bool,
    pub mode: InsertPlanMode,

    /// For DirectAppend, primary is already the final insert transaction.
    /// For SplitAndInsert, primary becomes the split parent transaction.
    pub primary: InsertPsbtTransaction,

    /// Present only for SplitAndInsert.
    pub child: Option<InsertPsbtTransaction>,

    pub service_fee: u64,
    pub network_fee: u64,
    pub total: u64,

    pub final_total_value: u64,
    pub final_groups: Vec<PlannedInsertGroup>,
}

pub fn run_insert_build_psbt(req: InsertBuildPsbtRequest) -> Result<InsertBuildPsbtResponse> {
    if req.payment_utxos.is_empty() {
        bail!("missing payment UTXOs");
    }

    /*
     * Execution boundary:
     * every external input is re-read from Bitcoin Core + ord immediately
     * before the Insert PSBTs are constructed.
     */
    let execution = build_execution_state(&req)?;

    let fee_rate = req.fee_rate.unwrap_or(1).max(1);

    let plan = run_insert_plan(InsertPlanRequest {
        input_utxo: req.input_utxo.clone(),
        ordinals_address: req.ordinals_address.clone(),
        total_value: req.total_value,
        existing_groups: req.existing_groups.clone(),
        insert_groups: req.insert_groups.clone(),
        ordered_groups: req.ordered_groups.clone(),
    })?;

    let service_fee = service_fee_sats(req.payment_method.as_deref())?;

    match plan.mode {
        InsertPlanMode::DirectAppend => {
            let primary = build_direct_append_psbt(&req, &plan, &execution, fee_rate, service_fee)?;

            let network_fee = primary.network_fee;

            Ok(InsertBuildPsbtResponse {
                ok: true,
                mode: plan.mode,
                primary,
                child: None,
                service_fee,
                network_fee,
                total: network_fee
                    .checked_add(service_fee)
                    .ok_or_else(|| anyhow!("total fee overflow"))?,
                final_total_value: plan.final_total_value,
                final_groups: plan.final_groups,
            })
        }

        InsertPlanMode::SplitAndInsert => {
            let (primary, child) =
                build_split_and_insert_psbts(&req, &plan, &execution, fee_rate, service_fee)?;

            let network_fee = primary
                .network_fee
                .checked_add(child.network_fee)
                .ok_or_else(|| anyhow!("combined network fee overflow"))?;

            Ok(InsertBuildPsbtResponse {
                ok: true,
                mode: plan.mode,
                primary,
                child: Some(child),
                service_fee,
                network_fee,
                total: network_fee
                    .checked_add(service_fee)
                    .ok_or_else(|| anyhow!("total fee overflow"))?,
                final_total_value: plan.final_total_value,
                final_groups: plan.final_groups,
            })
        }
    }
}

fn build_direct_append_psbt(
    req: &InsertBuildPsbtRequest,
    plan: &crate::insert::InsertPlanResponse,
    execution: &InsertExecutionState,
    fee_rate: u64,
    service_fee: u64,
) -> Result<InsertPsbtTransaction> {
    let ordinal_input_count = plan.child_inputs.len();

    if ordinal_input_count < 2 {
        bail!("direct append requires existing and inserted inputs");
    }

    let input_count = ordinal_input_count
        .checked_add(execution.payment_inputs.len())
        .ok_or_else(|| anyhow!("input count overflow"))?;

    let output_count = 1_usize
        .checked_add(if service_fee > 0 { 1 } else { 0 })
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| anyhow!("output count overflow"))?;

    let (vsize, network_fee) = estimate_network_fee(input_count, output_count, fee_rate)?;

    let payment_value = checked_current_payment_total(&execution.payment_inputs)?;

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

    let payment_change_value = payment_value
        .checked_sub(network_fee)
        .and_then(|value| value.checked_sub(service_fee))
        .ok_or_else(|| anyhow!("invalid payment change"))?;

    if payment_change_value < DUST_LIMIT {
        bail!("payment change would be dust");
    }

    let change_address = req
        .change_address
        .clone()
        .unwrap_or_else(|| req.payment_address.clone());

    let mut inputs = Vec::<TxIn>::new();

    /*
     * Inputs must follow the exact final order calculated by BCE:
     *
     * existing composition first, followed by append groups.
     */
    for child_input in &plan.child_inputs {
        let input_utxo = child_input
            .input_utxo
            .as_deref()
            .ok_or_else(|| anyhow!("direct append input has no UTXO"))?;

        inputs.push(build_txin(input_utxo)?);
    }

    for payment_state in &execution.payment_inputs {
        inputs.push(build_txin(&payment_state.outpoint)?);
    }

    let mut outputs = Vec::<TxOut>::new();

    outputs.push(TxOut {
        value: Amount::from_sat(plan.final_total_value),
        script_pubkey: address_to_script(&req.ordinals_address)?,
    });

    if service_fee > 0 {
        outputs.push(TxOut {
            value: Amount::from_sat(service_fee),
            script_pubkey: address_to_script(SERVICE_FEE_ADDRESS)?,
        });
    }

    let payment_change_output_index =
        u32::try_from(outputs.len()).map_err(|_| anyhow!("too many outputs"))?;

    outputs.push(TxOut {
        value: Amount::from_sat(payment_change_value),
        script_pubkey: address_to_script(&change_address)?,
    });

    let tx = Transaction {
        version: transaction::Version(2),
        lock_time: absolute::LockTime::ZERO,
        input: inputs,
        output: outputs,
    };

    let unsigned_txid = tx.txid().to_string();
    let tx_outputs = tx.output.len();

    let mut psbt = Psbt::from_unsigned_tx(tx)?;

    for (input_index, child_input) in plan.child_inputs.iter().enumerate() {
        let input_utxo = child_input
            .input_utxo
            .as_deref()
            .ok_or_else(|| anyhow!("direct append input has no UTXO"))?;

        let current = child_current_state(req, execution, input_utxo)?;

        psbt.inputs[input_index].witness_utxo = Some(TxOut {
            value: Amount::from_sat(current.value),
            script_pubkey: current_script(current)?,
        });

        if req.ordinals_address.starts_with("bc1p") {
            if let Some(public_key_hex) = req.ordinals_public_key.as_ref() {
                psbt.inputs[input_index].tap_internal_key =
                    Some(xonly_from_pubkey_hex(public_key_hex)?);
            }
        }
    }

    for (payment_index, payment_state) in execution.payment_inputs.iter().enumerate() {
        let psbt_index = ordinal_input_count
            .checked_add(payment_index)
            .ok_or_else(|| anyhow!("payment input index overflow"))?;

        psbt.inputs[psbt_index].witness_utxo = Some(TxOut {
            value: Amount::from_sat(payment_state.value),
            script_pubkey: current_script(payment_state)?,
        });

        if payment_state
            .address
            .as_deref()
            .is_some_and(|address| address.starts_with("bc1p"))
        {
            if let Some(public_key_hex) = req.payment_public_key.as_ref() {
                psbt.inputs[psbt_index].tap_internal_key =
                    Some(xonly_from_pubkey_hex(public_key_hex)?);
            }
        }
    }

    let psbt_base64 = general_purpose::STANDARD.encode(psbt.serialize());

    let mut sign_inputs = BTreeMap::<String, Vec<u32>>::new();

    let ordinal_signing_indices: Vec<u32> = (0..ordinal_input_count)
        .map(|index| u32::try_from(index).map_err(|_| anyhow!("ordinal signing index overflow")))
        .collect::<Result<Vec<_>>>()?;

    sign_inputs
        .entry(req.ordinals_address.clone())
        .or_default()
        .extend(ordinal_signing_indices);

    let payment_signing_indices: Vec<u32> = (ordinal_input_count..input_count)
        .map(|index| u32::try_from(index).map_err(|_| anyhow!("payment signing index overflow")))
        .collect::<Result<Vec<_>>>()?;

    sign_inputs
        .entry(req.payment_address.clone())
        .or_default()
        .extend(payment_signing_indices);

    Ok(InsertPsbtTransaction {
        psbt: psbt_base64,
        unsigned_txid,
        sign_inputs,
        network_fee,
        vsize,
        ordinal_inputs: ordinal_input_count,
        ordinal_outputs: 1,
        tx_outputs,
        payment_change_output_index,
        payment_change_value,
    })
}

fn build_split_and_insert_psbts(
    req: &InsertBuildPsbtRequest,
    plan: &crate::insert::InsertPlanResponse,
    execution: &InsertExecutionState,
    fee_rate: u64,
    service_fee: u64,
) -> Result<(InsertPsbtTransaction, InsertPsbtTransaction)> {
    if plan.existing_runs.len() < 2 {
        bail!("split_and_insert requires at least two existing runs");
    }

    /*
     * The child uses the unsigned parent txid.
     * Therefore all parent inputs must be SegWit.
     */
    if !is_segwit_address(&req.ordinals_address) {
        bail!("split_and_insert requires a SegWit ordinals address");
    }

    for payment_state in &execution.payment_inputs {
        let address = payment_state
            .address
            .as_deref()
            .ok_or_else(|| anyhow!("payment input has no current address"))?;

        if !is_segwit_address(address) {
            bail!("split_and_insert requires SegWit payment inputs");
        }
    }

    let payment_value = checked_current_payment_total(&execution.payment_inputs)?;

    let parent_input_count = 1_usize
        .checked_add(execution.payment_inputs.len())
        .ok_or_else(|| anyhow!("parent input count overflow"))?;

    let parent_output_count = plan
        .existing_runs
        .len()
        .checked_add(if service_fee > 0 { 1 } else { 0 })
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| anyhow!("parent output count overflow"))?;

    let (parent_vsize, parent_network_fee) =
        estimate_network_fee(parent_input_count, parent_output_count, fee_rate)?;

    let child_ordinal_input_count = plan.child_inputs.len();

    let child_input_count = child_ordinal_input_count
        .checked_add(1)
        .ok_or_else(|| anyhow!("child input count overflow"))?;

    let child_output_count = 2_usize;

    let (child_vsize, child_network_fee) =
        estimate_network_fee(child_input_count, child_output_count, fee_rate)?;

    let required_payment = parent_network_fee
        .checked_add(child_network_fee)
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

    let parent_change_value = payment_value
        .checked_sub(parent_network_fee)
        .and_then(|value| value.checked_sub(service_fee))
        .ok_or_else(|| anyhow!("invalid parent change"))?;

    let child_change_value = parent_change_value
        .checked_sub(child_network_fee)
        .ok_or_else(|| anyhow!("parent change cannot pay child fee"))?;

    if child_change_value < DUST_LIMIT {
        bail!("child payment change would be dust");
    }

    let change_address = req
        .change_address
        .clone()
        .unwrap_or_else(|| req.payment_address.clone());

    // ================= PARENT =================

    let mut parent_inputs = Vec::<TxIn>::new();

    /*
     * The existing composition remains parent input 0.
     */
    parent_inputs.push(build_txin(&req.input_utxo)?);

    for payment_state in &execution.payment_inputs {
        parent_inputs.push(build_txin(&payment_state.outpoint)?);
    }

    let mut parent_outputs = Vec::<TxOut>::new();

    /*
     * Existing runs must be emitted in their original
     * on-chain order.
     */
    for run in &plan.existing_runs {
        parent_outputs.push(TxOut {
            value: Amount::from_sat(run.value),
            script_pubkey: address_to_script(&req.ordinals_address)?,
        });
    }

    if service_fee > 0 {
        parent_outputs.push(TxOut {
            value: Amount::from_sat(service_fee),
            script_pubkey: address_to_script(SERVICE_FEE_ADDRESS)?,
        });
    }

    let parent_change_output_index =
        u32::try_from(parent_outputs.len()).map_err(|_| anyhow!("too many parent outputs"))?;

    parent_outputs.push(TxOut {
        value: Amount::from_sat(parent_change_value),
        script_pubkey: address_to_script(&change_address)?,
    });

    let parent_tx = Transaction {
        version: transaction::Version(2),
        lock_time: absolute::LockTime::ZERO,
        input: parent_inputs,
        output: parent_outputs,
    };

    let parent_txid = parent_tx.txid();
    let parent_tx_outputs = parent_tx.output.len();

    let mut parent_psbt = Psbt::from_unsigned_tx(parent_tx)?;

    parent_psbt.inputs[0].witness_utxo = Some(TxOut {
        value: Amount::from_sat(execution.current_input.value),
        script_pubkey: current_script(&execution.current_input)?,
    });

    if req.ordinals_address.starts_with("bc1p") {
        if let Some(public_key_hex) = req.ordinals_public_key.as_ref() {
            parent_psbt.inputs[0].tap_internal_key = Some(xonly_from_pubkey_hex(public_key_hex)?);
        }
    }

    for (payment_index, payment_state) in execution.payment_inputs.iter().enumerate() {
        let psbt_index = payment_index
            .checked_add(1)
            .ok_or_else(|| anyhow!("parent payment index overflow"))?;

        parent_psbt.inputs[psbt_index].witness_utxo = Some(TxOut {
            value: Amount::from_sat(payment_state.value),
            script_pubkey: current_script(payment_state)?,
        });

        if payment_state
            .address
            .as_deref()
            .is_some_and(|address| address.starts_with("bc1p"))
        {
            if let Some(public_key_hex) = req.payment_public_key.as_ref() {
                parent_psbt.inputs[psbt_index].tap_internal_key =
                    Some(xonly_from_pubkey_hex(public_key_hex)?);
            }
        }
    }

    let mut parent_sign_inputs = BTreeMap::<String, Vec<u32>>::new();

    parent_sign_inputs
        .entry(req.ordinals_address.clone())
        .or_default()
        .push(0);

    let parent_payment_indices: Vec<u32> = (1..parent_input_count)
        .map(|index| u32::try_from(index).map_err(|_| anyhow!("parent signing index overflow")))
        .collect::<Result<Vec<_>>>()?;

    parent_sign_inputs
        .entry(req.payment_address.clone())
        .or_default()
        .extend(parent_payment_indices);

    let primary = InsertPsbtTransaction {
        psbt: general_purpose::STANDARD.encode(parent_psbt.serialize()),
        unsigned_txid: parent_txid.to_string(),
        sign_inputs: parent_sign_inputs,
        network_fee: parent_network_fee,
        vsize: parent_vsize,
        ordinal_inputs: 1,
        ordinal_outputs: plan.existing_runs.len(),
        tx_outputs: parent_tx_outputs,
        payment_change_output_index: parent_change_output_index,
        payment_change_value: parent_change_value,
    };

    // ================= CHILD =================

    let mut child_inputs = Vec::<TxIn>::new();

    /*
     * Exact order planned by BCE:
     * parent runs and new insert UTXOs are interleaved.
     */
    for child_input in &plan.child_inputs {
        match child_input.kind {
            crate::insert::InsertChildInputKind::ExistingRun => {
                let parent_output_index = child_input
                    .parent_output_index
                    .ok_or_else(|| anyhow!("existing run has no parent output"))?;

                child_inputs.push(TxIn {
                    previous_output: OutPoint {
                        txid: parent_txid,
                        vout: parent_output_index,
                    },
                    script_sig: ScriptBuf::new(),
                    sequence: Sequence::MAX,
                    witness: Witness::new(),
                });
            }

            crate::insert::InsertChildInputKind::Inserted => {
                let input_utxo = child_input
                    .input_utxo
                    .as_deref()
                    .ok_or_else(|| anyhow!("inserted child input has no UTXO"))?;

                child_inputs.push(build_txin(input_utxo)?);
            }
        }
    }

    /*
     * Parent payment change comes last.
     */
    child_inputs.push(TxIn {
        previous_output: OutPoint {
            txid: parent_txid,
            vout: parent_change_output_index,
        },
        script_sig: ScriptBuf::new(),
        sequence: Sequence::MAX,
        witness: Witness::new(),
    });

    let child_outputs = vec![
        TxOut {
            value: Amount::from_sat(plan.final_total_value),
            script_pubkey: address_to_script(&req.ordinals_address)?,
        },
        TxOut {
            value: Amount::from_sat(child_change_value),
            script_pubkey: address_to_script(&change_address)?,
        },
    ];

    let child_tx = Transaction {
        version: transaction::Version(2),
        lock_time: absolute::LockTime::ZERO,
        input: child_inputs,
        output: child_outputs,
    };

    let child_txid = child_tx.txid();
    let child_tx_outputs = child_tx.output.len();

    let mut child_psbt = Psbt::from_unsigned_tx(child_tx)?;

    for (input_index, child_input) in plan.child_inputs.iter().enumerate() {
        let witness_utxo = match child_input.kind {
            crate::insert::InsertChildInputKind::ExistingRun => {
                let parent_output_index = child_input
                    .parent_output_index
                    .ok_or_else(|| anyhow!("existing run has no parent output"))?;

                let parent_output =
                    primary_output_for_child(plan, parent_output_index, &req.ordinals_address)?;

                parent_output
            }

            crate::insert::InsertChildInputKind::Inserted => {
                let input_utxo = child_input
                    .input_utxo
                    .as_deref()
                    .ok_or_else(|| anyhow!("inserted child input has no UTXO"))?;

                let current = execution.inserted_inputs.get(input_utxo).ok_or_else(|| {
                    anyhow!("missing validated insert input state for {}", input_utxo)
                })?;

                TxOut {
                    value: Amount::from_sat(current.value),
                    script_pubkey: current_script(current)?,
                }
            }
        };

        child_psbt.inputs[input_index].witness_utxo = Some(witness_utxo);

        if req.ordinals_address.starts_with("bc1p") {
            if let Some(public_key_hex) = req.ordinals_public_key.as_ref() {
                child_psbt.inputs[input_index].tap_internal_key =
                    Some(xonly_from_pubkey_hex(public_key_hex)?);
            }
        }
    }

    let child_payment_input_index = child_ordinal_input_count;

    child_psbt.inputs[child_payment_input_index].witness_utxo = Some(TxOut {
        value: Amount::from_sat(parent_change_value),
        script_pubkey: address_to_script(&change_address)?,
    });

    if change_address.starts_with("bc1p") {
        if let Some(public_key_hex) = req.payment_public_key.as_ref() {
            child_psbt.inputs[child_payment_input_index].tap_internal_key =
                Some(xonly_from_pubkey_hex(public_key_hex)?);
        }
    }

    let mut child_sign_inputs = BTreeMap::<String, Vec<u32>>::new();

    let child_ordinal_indices: Vec<u32> = (0..child_ordinal_input_count)
        .map(|index| u32::try_from(index).map_err(|_| anyhow!("child signing index overflow")))
        .collect::<Result<Vec<_>>>()?;

    child_sign_inputs
        .entry(req.ordinals_address.clone())
        .or_default()
        .extend(child_ordinal_indices);

    let child_payment_index = u32::try_from(child_payment_input_index)
        .map_err(|_| anyhow!("child payment signing index overflow"))?;

    child_sign_inputs
        .entry(change_address)
        .or_default()
        .push(child_payment_index);

    let child = InsertPsbtTransaction {
        psbt: general_purpose::STANDARD.encode(child_psbt.serialize()),
        unsigned_txid: child_txid.to_string(),
        sign_inputs: child_sign_inputs,
        network_fee: child_network_fee,
        vsize: child_vsize,
        ordinal_inputs: child_ordinal_input_count,
        ordinal_outputs: 1,
        tx_outputs: child_tx_outputs,
        payment_change_output_index: 1,
        payment_change_value: child_change_value,
    };

    Ok((primary, child))
}

fn build_execution_state(req: &InsertBuildPsbtRequest) -> Result<InsertExecutionState> {
    let current_input = validate_current_output(&req.input_utxo)?;

    validate_existing_insert_source(
        &current_input,
        req.total_value,
        &req.existing_groups,
        &req.ordinals_address,
    )?;

    let mut inserted_inputs = BTreeMap::<String, CurrentOutputState>::new();

    for group in &req.insert_groups {
        let state = validate_current_output(&group.input_utxo)?;

        validate_new_insert_source(&state, group, &req.ordinals_address)?;

        if inserted_inputs
            .insert(group.input_utxo.clone(), state)
            .is_some()
        {
            bail!("duplicate insert input UTXO: {}", group.input_utxo);
        }
    }

    let mut payment_inputs = Vec::<CurrentOutputState>::with_capacity(req.payment_utxos.len());

    for requested in &req.payment_utxos {
        let state = validate_current_output(&requested.outpoint)?;

        validate_payment_source(&state, requested, &req.payment_address)?;

        payment_inputs.push(state);
    }

    Ok(InsertExecutionState {
        current_input,
        inserted_inputs,
        payment_inputs,
    })
}

fn validate_existing_insert_source(
    state: &CurrentOutputState,
    claimed_total: u64,
    groups: &[ExistingInsertGroup],
    expected_address: &str,
) -> Result<()> {
    if state.value != claimed_total {
        bail!(
            "insert input {} value mismatch: request claimed {}, Bitcoin has {}",
            state.outpoint,
            claimed_total,
            state.value
        );
    }

    let address = state
        .address
        .as_deref()
        .ok_or_else(|| anyhow!("insert input {} has no current address", state.outpoint))?;

    if address != expected_address {
        bail!(
            "insert input {} address mismatch: expected {}, got {}",
            state.outpoint,
            expected_address,
            address
        );
    }

    for group in groups {
        let end = group
            .offset
            .checked_add(group.postage)
            .ok_or_else(|| anyhow!("existing insert group range overflow"))?;

        if end > state.value {
            bail!(
                "existing insert group at offset {} exceeds current UTXO value {}",
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
                        "existing inscription {} is not present on current output {}",
                        id,
                        state.outpoint
                    )
                })?;

            if current_offset < group.offset || current_offset >= end {
                bail!(
                    "existing inscription {} moved: current offset {} is outside requested range {}..{}",
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

fn validate_new_insert_source(
    state: &CurrentOutputState,
    group: &NewInsertGroup,
    expected_address: &str,
) -> Result<()> {
    if state.value != group.postage {
        bail!(
            "insert UTXO {} value mismatch: request claimed {}, Bitcoin has {}",
            state.outpoint,
            group.postage,
            state.value
        );
    }

    let address = state
        .address
        .as_deref()
        .ok_or_else(|| anyhow!("insert UTXO {} has no current address", state.outpoint))?;

    if address != expected_address {
        bail!(
            "insert UTXO {} address mismatch: expected {}, got {}",
            state.outpoint,
            expected_address,
            address
        );
    }

    for id in &group.ids {
        if !state
            .satpoints
            .iter()
            .any(|satpoint| satpoint.ids.iter().any(|current| current == id))
        {
            bail!(
                "insert inscription {} is not present on current output {}",
                id,
                state.outpoint
            );
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

fn current_script(state: &CurrentOutputState) -> Result<ScriptBuf> {
    ScriptBuf::from_hex(&state.script_pubkey)
        .map_err(|_| anyhow!("invalid current scriptPubKey for {}", state.outpoint))
}

fn checked_current_payment_total(payment_inputs: &[CurrentOutputState]) -> Result<u64> {
    payment_inputs.iter().try_fold(0_u64, |total, state| {
        total
            .checked_add(state.value)
            .ok_or_else(|| anyhow!("payment value overflow"))
    })
}

fn child_current_state<'a>(
    req: &InsertBuildPsbtRequest,
    execution: &'a InsertExecutionState,
    outpoint: &str,
) -> Result<&'a CurrentOutputState> {
    if outpoint == req.input_utxo {
        return Ok(&execution.current_input);
    }

    execution.inserted_inputs.get(outpoint).ok_or_else(|| {
        anyhow!(
            "missing validated current state for insert input {}",
            outpoint
        )
    })
}

fn primary_output_for_child(
    plan: &crate::insert::InsertPlanResponse,
    parent_output_index: u32,
    ordinals_address: &str,
) -> Result<TxOut> {
    let index =
        usize::try_from(parent_output_index).map_err(|_| anyhow!("invalid parent output index"))?;

    let run = plan
        .existing_runs
        .get(index)
        .ok_or_else(|| anyhow!("parent existing-run output index out of range"))?;

    Ok(TxOut {
        value: Amount::from_sat(run.value),
        script_pubkey: address_to_script(ordinals_address)?,
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

fn is_segwit_address(address: &str) -> bool {
    address.to_ascii_lowercase().starts_with("bc1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_guard::CurrentSatpoint;
    use base64::engine::general_purpose;
    use bitcoin::psbt::Psbt;

    const ADDRESS: &str = "bc1qznl7wxgtemt5eprmr6g3yj7nn7xh5gtzuvezuz";

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn request(append: bool) -> InsertBuildPsbtRequest {
        InsertBuildPsbtRequest {
            input_utxo: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:0"
                .to_string(),
            ordinals_address: ADDRESS.to_string(),
            payment_address: ADDRESS.to_string(),
            change_address: Some(ADDRESS.to_string()),
            ordinals_public_key: None,
            payment_public_key: None,
            total_value: 2646,
            existing_groups: vec![
                ExistingInsertGroup {
                    ids: ids(&["A"]),
                    offset: 0,
                    postage: 546,
                },
                ExistingInsertGroup {
                    ids: ids(&["B"]),
                    offset: 546,
                    postage: 700,
                },
                ExistingInsertGroup {
                    ids: ids(&["D"]),
                    offset: 1246,
                    postage: 600,
                },
                ExistingInsertGroup {
                    ids: ids(&["E"]),
                    offset: 1846,
                    postage: 800,
                },
            ],
            insert_groups: vec![NewInsertGroup {
                ids: ids(&["C"]),
                input_utxo: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc:0"
                    .to_string(),
                postage: 650,
            }],
            ordered_groups: if append {
                vec![
                    ids(&["A"]),
                    ids(&["B"]),
                    ids(&["D"]),
                    ids(&["E"]),
                    ids(&["C"]),
                ]
            } else {
                vec![
                    ids(&["A"]),
                    ids(&["B"]),
                    ids(&["C"]),
                    ids(&["D"]),
                    ids(&["E"]),
                ]
            },
            payment_utxos: vec![Utxo {
                outpoint: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:1"
                    .to_string(),
                value: 10000,
                address: ADDRESS.to_string(),
                inscriptions: Vec::new(),
            }],
            fee_rate: Some(1),
            payment_method: Some("bitcoin".to_string()),
        }
    }

    fn script_hex(address: &str) -> String {
        address_to_script(address)
            .expect("valid test address")
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn execution_state(req: &InsertBuildPsbtRequest) -> InsertExecutionState {
        let ordinal_script = if req.ordinals_address.starts_with("bc1") {
            script_hex(&req.ordinals_address)
        } else {
            String::new()
        };

        let current_input = CurrentOutputState {
            outpoint: req.input_utxo.clone(),
            value: req.total_value,
            script_pubkey: ordinal_script.clone(),
            address: Some(req.ordinals_address.clone()),
            satpoints: req
                .existing_groups
                .iter()
                .map(|group| CurrentSatpoint {
                    ids: group.ids.clone(),
                    offset: group.offset,
                    postage: group.postage,
                })
                .collect(),
        };

        let inserted_inputs = req
            .insert_groups
            .iter()
            .map(|group| {
                (
                    group.input_utxo.clone(),
                    CurrentOutputState {
                        outpoint: group.input_utxo.clone(),
                        value: group.postage,
                        script_pubkey: ordinal_script.clone(),
                        address: Some(req.ordinals_address.clone()),
                        satpoints: vec![CurrentSatpoint {
                            ids: group.ids.clone(),
                            offset: 0,
                            postage: group.postage,
                        }],
                    },
                )
            })
            .collect();

        let payment_inputs = req
            .payment_utxos
            .iter()
            .map(|utxo| CurrentOutputState {
                outpoint: utxo.outpoint.clone(),
                value: utxo.value,
                script_pubkey: script_hex(&utxo.address),
                address: Some(utxo.address.clone()),
                satpoints: Vec::new(),
            })
            .collect();

        InsertExecutionState {
            current_input,
            inserted_inputs,
            payment_inputs,
        }
    }

    fn plan(req: &InsertBuildPsbtRequest) -> crate::insert::InsertPlanResponse {
        run_insert_plan(InsertPlanRequest {
            input_utxo: req.input_utxo.clone(),
            ordinals_address: req.ordinals_address.clone(),
            total_value: req.total_value,
            existing_groups: req.existing_groups.clone(),
            insert_groups: req.insert_groups.clone(),
            ordered_groups: req.ordered_groups.clone(),
        })
        .expect("insert plan must build")
    }

    fn decode_psbt(value: &str) -> Psbt {
        let bytes = general_purpose::STANDARD
            .decode(value)
            .expect("valid base64");

        Psbt::deserialize(&bytes).expect("valid PSBT")
    }

    #[test]
    fn builds_direct_append_transaction() {
        let req = request(true);
        let plan = plan(&req);
        let execution = execution_state(&req);
        let service_fee = service_fee_sats(req.payment_method.as_deref()).unwrap();

        assert_eq!(plan.mode, InsertPlanMode::DirectAppend);

        let primary = build_direct_append_psbt(&req, &plan, &execution, 1, service_fee)
            .expect("direct append must build");

        assert_eq!(primary.network_fee, 343);
        assert_eq!(primary.vsize, 343);
        assert_eq!(primary.ordinal_inputs, 2);
        assert_eq!(primary.ordinal_outputs, 1);
        assert_eq!(primary.tx_outputs, 3);
        assert_eq!(primary.payment_change_value, 8157);

        let psbt = decode_psbt(&primary.psbt);
        let tx = &psbt.unsigned_tx;

        assert_eq!(tx.input.len(), 3);
        assert_eq!(tx.output.len(), 3);

        assert_eq!(tx.output[0].value.to_sat(), 3296);
        assert_eq!(tx.output[1].value.to_sat(), 1500);
        assert_eq!(tx.output[2].value.to_sat(), 8157);
    }

    #[test]
    fn builds_split_parent_and_insert_child() {
        let req = request(false);
        let plan = plan(&req);
        let execution = execution_state(&req);
        let service_fee = service_fee_sats(req.payment_method.as_deref()).unwrap();

        assert_eq!(plan.mode, InsertPlanMode::SplitAndInsert);

        let (primary, child) =
            build_split_and_insert_psbts(&req, &plan, &execution, 1, service_fee)
                .expect("split insert must build");

        assert_eq!(primary.network_fee, 318);
        assert_eq!(primary.ordinal_outputs, 2);
        assert_eq!(primary.payment_change_output_index, 3);
        assert_eq!(primary.payment_change_value, 8182);

        let parent = decode_psbt(&primary.psbt);

        assert_eq!(parent.unsigned_tx.output[0].value.to_sat(), 1246);
        assert_eq!(parent.unsigned_tx.output[1].value.to_sat(), 1400);
        assert_eq!(parent.unsigned_tx.output[2].value.to_sat(), 1500);
        assert_eq!(parent.unsigned_tx.output[3].value.to_sat(), 8182);

        assert_eq!(child.network_fee, 368);
        assert_eq!(child.ordinal_inputs, 3);
        assert_eq!(child.payment_change_value, 7814);

        let child_psbt = decode_psbt(&child.psbt);
        let tx = &child_psbt.unsigned_tx;

        assert_eq!(tx.input.len(), 4);
        assert_eq!(tx.output.len(), 2);

        assert_eq!(tx.input[0].previous_output.vout, 0);
        assert_eq!(tx.input[1].previous_output.vout, 0);
        assert_eq!(tx.input[2].previous_output.vout, 1);
        assert_eq!(tx.input[3].previous_output.vout, 3);

        assert_eq!(tx.output[0].value.to_sat(), 3296);
        assert_eq!(tx.output[1].value.to_sat(), 7814);

        assert_eq!(primary.network_fee + child.network_fee, 686);
        assert_eq!(service_fee, 1500);
        assert_eq!(primary.network_fee + child.network_fee + service_fee, 2186);
    }

    #[test]
    fn insufficient_payment_is_rejected() {
        let mut req = request(false);
        req.payment_utxos[0].value = 2000;

        let plan = plan(&req);
        let execution = execution_state(&req);
        let service_fee = service_fee_sats(req.payment_method.as_deref()).unwrap();

        let error = build_split_and_insert_psbts(&req, &plan, &execution, 1, service_fee)
            .expect_err("small payment must fail");

        assert!(error.to_string().contains("payment UTXOs too small"));
    }

    #[test]
    fn non_segwit_parent_is_rejected_for_split_insert() {
        let mut req = request(false);
        req.ordinals_address = "legacy-address".to_string();

        let plan = plan(&req);
        let execution = execution_state(&req);
        let service_fee = service_fee_sats(req.payment_method.as_deref()).unwrap();

        let error = build_split_and_insert_psbts(&req, &plan, &execution, 1, service_fee)
            .expect_err("legacy parent must fail");

        assert!(error
            .to_string()
            .contains("requires a SegWit ordinals address"));
    }
}

#[cfg(test)]
mod execution_guard_tests {
    use super::*;
    use crate::execution_guard::CurrentSatpoint;

    const ADDRESS: &str = "bc1qznl7wxgtemt5eprmr6g3yj7nn7xh5gtzuvezuz";

    fn state(
        outpoint: &str,
        value: u64,
        satpoints: Vec<(u64, Vec<&str>, u64)>,
    ) -> CurrentOutputState {
        CurrentOutputState {
            outpoint: outpoint.to_string(),
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
    fn rejects_existing_insert_total_value_mismatch() {
        let current = state(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:0",
            1000,
            vec![(0, vec!["A"], 500), (500, vec!["B"], 500)],
        );

        let groups = vec![
            ExistingInsertGroup {
                ids: vec!["A".to_string()],
                offset: 0,
                postage: 500,
            },
            ExistingInsertGroup {
                ids: vec!["B".to_string()],
                offset: 500,
                postage: 500,
            },
        ];

        let error = validate_existing_insert_source(&current, 999, &groups, ADDRESS)
            .expect_err("stale existing total must fail");

        assert!(error
            .to_string()
            .contains("request claimed 999, Bitcoin has 1000"));
    }

    #[test]
    fn rejects_new_insert_value_mismatch() {
        let current = state(
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc:0",
            1000,
            vec![(0, vec!["C"], 1000)],
        );

        let group = NewInsertGroup {
            ids: vec!["C".to_string()],
            input_utxo: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc:0"
                .to_string(),
            postage: 546,
        };

        let error = validate_new_insert_source(&current, &group, ADDRESS)
            .expect_err("stale insert value must fail");

        assert!(error
            .to_string()
            .contains("request claimed 546, Bitcoin has 1000"));
    }

    #[test]
    fn rejects_missing_new_insert_id() {
        let current = state(
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc:0",
            1000,
            vec![(0, vec!["OTHER"], 1000)],
        );

        let group = NewInsertGroup {
            ids: vec!["C".to_string()],
            input_utxo: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc:0"
                .to_string(),
            postage: 1000,
        };

        let error = validate_new_insert_source(&current, &group, ADDRESS)
            .expect_err("missing insert inscription must fail");

        assert!(error
            .to_string()
            .contains("is not present on current output"));
    }

    #[test]
    fn accepts_shared_satpoint_for_new_insert() {
        let current = state(
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc:0",
            1000,
            vec![(0, vec!["C", "HIDDEN"], 1000)],
        );

        let group = NewInsertGroup {
            ids: vec!["C".to_string()],
            input_utxo: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc:0"
                .to_string(),
            postage: 1000,
        };

        validate_new_insert_source(&current, &group, ADDRESS)
            .expect("shared satpoint must remain valid");
    }

    #[test]
    fn rejects_payment_with_inscription() {
        let current = state(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:1",
            10000,
            vec![(0, vec!["ORD"], 10000)],
        );

        let requested = Utxo {
            outpoint: current.outpoint.clone(),
            value: 10000,
            address: ADDRESS.to_string(),
            inscriptions: Vec::new(),
        };

        let error = validate_payment_source(&current, &requested, ADDRESS)
            .expect_err("inscription-bearing payment must fail");

        assert!(error.to_string().contains("contains inscriptions"));
    }
}
