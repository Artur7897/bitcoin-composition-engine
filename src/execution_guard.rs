use anyhow::{anyhow, bail, Result};
use serde_json::json;

use crate::bitcoin_rpc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitcoinOutputState {
    pub outpoint: String,
    pub value: u64,
    pub script_pubkey: String,
    pub address: Option<String>,
}

pub fn read_current_bitcoin_output(outpoint: &str) -> Result<BitcoinOutputState> {
    let (txid, vout) = parse_outpoint(outpoint)?;

    let result = bitcoin_rpc::call("gettxout", json!([txid, vout, true]))?;

    if result.is_null() {
        bail!("Bitcoin output {} is spent or does not exist", outpoint);
    }

    let btc_value = result
        .get("value")
        .ok_or_else(|| anyhow!("gettxout returned no value for {}", outpoint))?;

    let value = btc_json_to_sats(btc_value)?;

    let script = result
        .get("scriptPubKey")
        .ok_or_else(|| anyhow!("gettxout returned no scriptPubKey for {}", outpoint))?;

    let script_pubkey = script
        .get("hex")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("gettxout returned no scriptPubKey hex for {}", outpoint))?
        .to_string();

    let address = script
        .get("address")
        .and_then(|value| value.as_str())
        .map(str::to_string);

    Ok(BitcoinOutputState {
        outpoint: outpoint.to_string(),
        value,
        script_pubkey,
        address,
    })
}

fn parse_outpoint(outpoint: &str) -> Result<(&str, u32)> {
    let (txid, vout) = outpoint
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("invalid outpoint: {}", outpoint))?;

    if txid.len() != 64 || !txid.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid transaction id in outpoint: {}", outpoint);
    }

    let vout = vout
        .parse::<u32>()
        .map_err(|_| anyhow!("invalid vout in outpoint: {}", outpoint))?;

    Ok((txid, vout))
}

fn btc_json_to_sats(value: &serde_json::Value) -> Result<u64> {
    let raw = value.to_string();

    if raw.starts_with('-') {
        bail!("invalid negative Bitcoin output value");
    }

    let (mantissa, exponent) = match raw.find(['e', 'E']) {
        Some(pos) => {
            let exponent = raw[pos + 1..]
                .parse::<i32>()
                .map_err(|_| anyhow!("invalid Bitcoin output value: {}", raw))?;

            (&raw[..pos], exponent)
        }
        None => (raw.as_str(), 0),
    };

    let (whole, fractional) = match mantissa.split_once('.') {
        Some((whole, fractional)) => (whole, fractional),
        None => (mantissa, ""),
    };

    if whole.is_empty()
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !fractional.chars().all(|c| c.is_ascii_digit())
    {
        bail!("invalid Bitcoin output value: {}", raw);
    }

    let digits_raw = format!("{}{}", whole, fractional);

    let digits = digits_raw
        .parse::<u128>()
        .map_err(|_| anyhow!("Bitcoin output value overflow: {}", raw))?;

    if digits == 0 {
        return Ok(0);
    }

    let scale = 8_i32
        .checked_add(exponent)
        .and_then(|value| value.checked_sub(fractional.len() as i32))
        .ok_or_else(|| anyhow!("Bitcoin output value scale overflow: {}", raw))?;

    let sats = if scale >= 0 {
        let scale = u32::try_from(scale)
            .map_err(|_| anyhow!("Bitcoin output value scale overflow: {}", raw))?;

        let multiplier = 10_u128
            .checked_pow(scale)
            .ok_or_else(|| anyhow!("Bitcoin output value overflow: {}", raw))?;

        digits
            .checked_mul(multiplier)
            .ok_or_else(|| anyhow!("Bitcoin output value overflow: {}", raw))?
    } else {
        let divisor_scale = scale
            .checked_neg()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| anyhow!("Bitcoin output value scale overflow: {}", raw))?;

        let divisor = 10_u128
            .checked_pow(divisor_scale)
            .ok_or_else(|| anyhow!("Bitcoin output value has sub-satoshi precision: {}", raw))?;

        if digits % divisor != 0 {
            bail!("Bitcoin output value has sub-satoshi precision: {}", raw);
        }

        digits / divisor
    };

    u64::try_from(sats).map_err(|_| anyhow!("Bitcoin output value overflow: {}", raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_outpoint() {
        let txid = "a".repeat(64);
        let outpoint = format!("{}:7", txid);

        let (parsed_txid, vout) = parse_outpoint(&outpoint).unwrap();

        assert_eq!(parsed_txid, txid);
        assert_eq!(vout, 7);
    }

    #[test]
    fn rejects_invalid_outpoint() {
        assert!(parse_outpoint("not-an-outpoint").is_err());
        assert!(parse_outpoint("abc:0").is_err());
    }

    #[test]
    fn converts_btc_json_to_sats_exactly() {
        let value_1092: serde_json::Value = serde_json::from_str("0.00001092").unwrap();
        let one_btc: serde_json::Value = serde_json::from_str("1").unwrap();
        let one_btc_decimal: serde_json::Value = serde_json::from_str("1.00000000").unwrap();
        let one_sat: serde_json::Value = serde_json::from_str("0.00000001").unwrap();

        assert_eq!(btc_json_to_sats(&value_1092).unwrap(), 1092);
        assert_eq!(btc_json_to_sats(&one_btc).unwrap(), 100_000_000);
        assert_eq!(btc_json_to_sats(&one_btc_decimal).unwrap(), 100_000_000);
        assert_eq!(btc_json_to_sats(&one_sat).unwrap(), 1);
    }

    #[test]
    fn rejects_sub_satoshi_and_negative_values() {
        let sub_sat: serde_json::Value = serde_json::from_str("0.000000001").unwrap();
        let negative: serde_json::Value = serde_json::from_str("-0.00000001").unwrap();

        assert!(btc_json_to_sats(&sub_sat).is_err());
        assert!(btc_json_to_sats(&negative).is_err());
    }

    #[test]
    fn shared_satpoint_is_one_physical_unit() {
        let inscriptions = vec![
            OrdInscriptionState {
                id: "ID-A".to_string(),
                offset: 0,
            },
            OrdInscriptionState {
                id: "ID-B".to_string(),
                offset: 0,
            },
        ];

        let satpoints = build_current_satpoints(&inscriptions, 1000).unwrap();

        assert_eq!(satpoints.len(), 1);
        assert_eq!(satpoints[0].offset, 0);
        assert_eq!(satpoints[0].postage, 1000);
        assert_eq!(
            satpoints[0].ids,
            vec!["ID-A".to_string(), "ID-B".to_string()]
        );

        let total: u64 = satpoints.iter().map(|satpoint| satpoint.postage).sum();
        assert_eq!(total, 1000);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrdInscriptionState {
    pub id: String,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrdOutputState {
    pub outpoint: String,
    pub value: u64,
    pub script_pubkey: String,
    pub inscriptions: Vec<OrdInscriptionState>,
}

pub fn read_current_ord_output(outpoint: &str) -> Result<OrdOutputState> {
    parse_outpoint(outpoint)?;

    let base_url =
        std::env::var("ORD_SERVER_URL").map_err(|_| anyhow!("ORD_SERVER_URL missing"))?;

    let client = reqwest::blocking::Client::new();

    let output: serde_json::Value = client
        .get(format!(
            "{}/output/{}",
            base_url.trim_end_matches('/'),
            outpoint
        ))
        .header("Accept", "application/json")
        .send()?
        .error_for_status()?
        .json()?;

    if output.get("indexed").and_then(|v| v.as_bool()) != Some(true) {
        bail!("ord output {} is not indexed", outpoint);
    }

    if output.get("spent").and_then(|v| v.as_bool()) != Some(false) {
        bail!("ord output {} is spent", outpoint);
    }

    let returned_outpoint = output
        .get("outpoint")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("ord returned no outpoint for {}", outpoint))?;

    if returned_outpoint != outpoint {
        bail!(
            "ord outpoint mismatch: requested {}, returned {}",
            outpoint,
            returned_outpoint
        );
    }

    let value = output
        .get("value")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("ord returned no valid value for {}", outpoint))?;

    let script_pubkey = output
        .get("script_pubkey")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("ord returned no script_pubkey for {}", outpoint))?
        .to_string();

    let ids = output
        .get("inscriptions")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("ord returned no inscription list for {}", outpoint))?;

    let mut inscriptions = Vec::with_capacity(ids.len());

    for id in ids {
        let id = id
            .as_str()
            .ok_or_else(|| anyhow!("ord returned invalid inscription ID for {}", outpoint))?;

        let inscription: serde_json::Value = client
            .get(format!(
                "{}/inscription/{}",
                base_url.trim_end_matches('/'),
                id
            ))
            .header("Accept", "application/json")
            .send()?
            .error_for_status()?
            .json()?;

        let returned_id = inscription
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("ord returned no ID for inscription {}", id))?;

        if returned_id != id {
            bail!(
                "ord inscription ID mismatch: requested {}, returned {}",
                id,
                returned_id
            );
        }

        let satpoint = inscription
            .get("satpoint")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("ord returned no satpoint for inscription {}", id))?;

        let expected_prefix = format!("{}:", outpoint);

        let offset_raw = satpoint.strip_prefix(&expected_prefix).ok_or_else(|| {
            anyhow!(
                "ord inscription {} belongs to {}, expected {}",
                id,
                satpoint,
                outpoint
            )
        })?;

        let offset = offset_raw
            .parse::<u64>()
            .map_err(|_| anyhow!("invalid ord satpoint for inscription {}: {}", id, satpoint))?;

        if offset >= value {
            bail!(
                "ord inscription {} offset {} exceeds output value {}",
                id,
                offset,
                value
            );
        }

        inscriptions.push(OrdInscriptionState {
            id: id.to_string(),
            offset,
        });
    }

    inscriptions.sort_by_key(|item| item.offset);

    Ok(OrdOutputState {
        outpoint: outpoint.to_string(),
        value,
        script_pubkey,
        inscriptions,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CurrentSatpoint {
    pub ids: Vec<String>,
    pub offset: u64,
    pub postage: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CurrentOutputState {
    pub outpoint: String,
    pub value: u64,
    pub script_pubkey: String,
    pub address: Option<String>,
    pub satpoints: Vec<CurrentSatpoint>,
}

#[derive(Debug, serde::Deserialize)]
pub struct InspectCurrentStateRequest {
    pub outpoint: String,
}

pub fn inspect_current_state(req: InspectCurrentStateRequest) -> Result<CurrentOutputState> {
    validate_current_output(&req.outpoint)
}

fn build_current_satpoints(
    inscriptions: &[OrdInscriptionState],
    total_value: u64,
) -> Result<Vec<CurrentSatpoint>> {
    if inscriptions.is_empty() {
        return Ok(Vec::new());
    }

    let mut grouped = Vec::<CurrentSatpoint>::new();

    for inscription in inscriptions {
        if inscription.offset >= total_value {
            bail!(
                "inscription {} offset {} exceeds output value {}",
                inscription.id,
                inscription.offset,
                total_value
            );
        }

        match grouped.last_mut() {
            Some(current) if current.offset == inscription.offset => {
                current.ids.push(inscription.id.clone());
            }

            _ => {
                grouped.push(CurrentSatpoint {
                    ids: vec![inscription.id.clone()],
                    offset: inscription.offset,
                    postage: 0,
                });
            }
        }
    }

    for index in 0..grouped.len() {
        let end = grouped
            .get(index + 1)
            .map(|next| next.offset)
            .unwrap_or(total_value);

        let postage = end
            .checked_sub(grouped[index].offset)
            .ok_or_else(|| anyhow!("invalid satpoint geometry"))?;

        if postage == 0 {
            bail!(
                "satpoint at offset {} has zero postage",
                grouped[index].offset
            );
        }

        grouped[index].postage = postage;
    }

    Ok(grouped)
}

pub fn validate_current_output(outpoint: &str) -> Result<CurrentOutputState> {
    let bitcoin = read_current_bitcoin_output(outpoint)?;
    let ord = read_current_ord_output(outpoint)?;

    if bitcoin.outpoint != ord.outpoint {
        bail!(
            "state mismatch: Bitcoin Core outpoint {} != ord outpoint {}",
            bitcoin.outpoint,
            ord.outpoint
        );
    }

    if bitcoin.value != ord.value {
        bail!(
            "state mismatch for {}: Bitcoin Core value {} != ord value {}",
            outpoint,
            bitcoin.value,
            ord.value
        );
    }

    if !bitcoin
        .script_pubkey
        .eq_ignore_ascii_case(&ord.script_pubkey)
    {
        bail!("state mismatch for {}: scriptPubKey differs", outpoint);
    }

    let satpoints = build_current_satpoints(&ord.inscriptions, bitcoin.value)?;

    Ok(CurrentOutputState {
        outpoint: bitcoin.outpoint,
        value: bitcoin.value,
        script_pubkey: bitcoin.script_pubkey,
        address: bitcoin.address,
        satpoints,
    })
}
