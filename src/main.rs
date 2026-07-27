use anyhow::{anyhow, Result};
use std::{env, process};

mod compose_broadcast;
mod compose_plan;
mod compose_psbt;
mod compose_types;
mod extract;
mod extract_broadcast;
mod extract_psbt;
mod fees;
mod insert;
mod insert_broadcast;
mod insert_psbt;
mod models;
mod recompose_psbt;
pub mod spec;
mod split_broadcast;
mod split_plan;
mod split_psbt;
mod split_types;
pub mod verify;
pub mod verify_compose;

use crate::compose_types::{BroadcastRequest, ComposeBuildPsbtRequest, ComposePlanRequest};

use compose_broadcast::run_compose_broadcast;
use compose_plan::run_compose_plan;
use compose_psbt::run_compose_build_psbt;

fn parse_json<T: serde::de::DeserializeOwned>(payload_raw: &str) -> Result<T> {
    let payload: T = serde_json::from_str(payload_raw)?;
    Ok(payload)
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        eprintln!("missing command or payload");
        process::exit(1);
    }

    let command = &args[1];
    let payload_raw = &args[2];

    let result = match command.as_str() {
        "compose-plan" => {
            let req: Result<ComposePlanRequest> = parse_json(payload_raw);

            req.and_then(run_compose_plan)
                .and_then(|v| serde_json::to_string(&v).map_err(Into::into))
        }

        "compose-build-psbt" => {
            let req: Result<ComposeBuildPsbtRequest> = parse_json(payload_raw);

            req.and_then(run_compose_build_psbt)
                .and_then(|v| serde_json::to_string(&v).map_err(Into::into))
        }

        "compose-broadcast" => {
            let req: Result<BroadcastRequest> = parse_json(payload_raw);

            req.and_then(run_compose_broadcast)
                .and_then(|v| serde_json::to_string(&v).map_err(Into::into))
        }
        "split-plan" => {
            let req: Result<split_types::SplitPlanRequest> = parse_json(payload_raw);

            req.and_then(split_plan::run_split_plan)
                .and_then(|v| serde_json::to_string(&v).map_err(Into::into))
        }

        "split-build-psbt" => {
            let req: Result<split_types::SplitBuildPsbtRequest> = parse_json(payload_raw);

            req.and_then(split_psbt::run_split_build_psbt)
                .and_then(|v| serde_json::to_string(&v).map_err(Into::into))
        }

        "split-execute" => {
            let req: Result<split_types::SplitBuildPsbtRequest> = parse_json(payload_raw);

            req.and_then(split_psbt::run_split_build_psbt)
                .and_then(|v| serde_json::to_string(&v).map_err(Into::into))
        }

        "split-broadcast" => {
            let req: Result<split_types::SplitBroadcastRequest> = parse_json(payload_raw);

            req.and_then(split_broadcast::run_split_broadcast)
                .and_then(|v| serde_json::to_string(&v).map_err(Into::into))
        }

        "extract-plan" => {
            let req: Result<extract::ExtractPlanRequest> = parse_json(payload_raw);

            req.and_then(extract::run_extract_plan)
                .and_then(|v| serde_json::to_string(&v).map_err(Into::into))
        }

        "extract-build-psbt" => {
            let req: Result<extract_psbt::ExtractBuildPsbtRequest> = parse_json(payload_raw);

            req.and_then(extract_psbt::run_extract_build_psbt)
                .and_then(|v| serde_json::to_string(&v).map_err(Into::into))
        }

        "extract-broadcast" => {
            let req: Result<extract_broadcast::ExtractBroadcastRequest> = parse_json(payload_raw);

            req.and_then(extract_broadcast::run_extract_broadcast)
                .and_then(|v| serde_json::to_string(&v).map_err(Into::into))
        }

        "insert-plan" => {
            let req: Result<insert::InsertPlanRequest> = parse_json(payload_raw);

            req.and_then(insert::run_insert_plan)
                .and_then(|v| serde_json::to_string(&v).map_err(Into::into))
        }

        "insert-build-psbt" => {
            let req: Result<insert_psbt::InsertBuildPsbtRequest> = parse_json(payload_raw);

            req.and_then(insert_psbt::run_insert_build_psbt)
                .and_then(|v| serde_json::to_string(&v).map_err(Into::into))
        }

        "insert-broadcast" => {
            let req: Result<insert_broadcast::InsertBroadcastRequest> = parse_json(payload_raw);

            req.and_then(insert_broadcast::run_insert_broadcast)
                .and_then(|v| serde_json::to_string(&v).map_err(Into::into))
        }

        "verify-composition" => {
            let req: Result<verify::VerifyCompositionRequest> = parse_json(payload_raw);

            req.and_then(verify::verify_composition)
                .and_then(|value| serde_json::to_string(&value).map_err(Into::into))
        }

        "verify-compose" => {
            let req: Result<verify_compose::VerifyComposeRequest> = parse_json(payload_raw);

            req.and_then(verify_compose::verify_compose)
                .and_then(|value| serde_json::to_string(&value).map_err(Into::into))
        }
        _ => Err(anyhow!("unknown command: {}", command)),
    };

    match result {
        Ok(json) => {
            println!("{}", json);
        }

        Err(err) => {
            eprintln!("{}", err);
            process::exit(1);
        }
    }
}
