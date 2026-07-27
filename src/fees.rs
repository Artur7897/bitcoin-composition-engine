use anyhow::{anyhow, bail, Result};

pub const SERVICE_FEE_SATS: u64 = 1500;

pub const SERVICE_FEE_ADDRESS: &str = "bc1qznl7wxgtemt5eprmr6g3yj7nn7xh5gtzuvezuz";

pub const MIN_NETWORK_FEE: u64 = 250;
pub const DUST_LIMIT: u64 = 546;

pub fn service_fee_sats(payment_method: Option<&str>) -> Result<u64> {
    match payment_method {
        None | Some("bitcoin") => Ok(SERVICE_FEE_SATS),

        Some(method) => {
            bail!("unsupported payment method: {}", method)
        }
    }
}

pub fn estimate_vsize(input_count: usize, output_count: usize) -> Result<u64> {
    let inputs = u64::try_from(input_count).map_err(|_| anyhow!("too many inputs"))?;

    let outputs = u64::try_from(output_count).map_err(|_| anyhow!("too many outputs"))?;

    10_u64
        .checked_add(
            inputs
                .checked_mul(68)
                .ok_or_else(|| anyhow!("input vsize overflow"))?,
        )
        .and_then(|value| {
            outputs
                .checked_mul(43)
                .and_then(|output_size| value.checked_add(output_size))
        })
        .ok_or_else(|| anyhow!("vsize overflow"))
}

pub fn estimate_network_fee(
    input_count: usize,
    output_count: usize,
    fee_rate: u64,
) -> Result<(u64, u64)> {
    let vsize = estimate_vsize(input_count, output_count)?;

    let fee = vsize
        .checked_mul(fee_rate.max(1))
        .ok_or_else(|| anyhow!("network fee overflow"))?
        .max(MIN_NETWORK_FEE);

    Ok((vsize, fee))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitcoin_service_fee_is_1500_sats() {
        assert_eq!(service_fee_sats(None).unwrap(), 1500);
        assert_eq!(service_fee_sats(Some("bitcoin")).unwrap(), 1500);
    }

    #[test]
    fn credits_are_rejected_until_implemented() {
        let error =
            service_fee_sats(Some("credits")).expect_err("credits must not be accepted yet");

        assert_eq!(error.to_string(), "unsupported payment method: credits");
    }

    #[test]
    fn network_fee_uses_actual_transaction_size() {
        let (vsize, fee) = estimate_network_fee(2, 3, 1).unwrap();

        assert_eq!(vsize, 275);
        assert_eq!(fee, 275);
    }

    #[test]
    fn network_fee_respects_minimum() {
        let (vsize, fee) = estimate_network_fee(1, 1, 1).unwrap();

        assert_eq!(vsize, 121);
        assert_eq!(fee, MIN_NETWORK_FEE);
    }
}
