use crate::models::Utxo;
//use crate::spec::CompositionSpec;

pub struct PackItem {
    pub index: u32,
    pub outpoint: String,
    pub inscription_id: String,
}

pub struct PackPlan {
    pub root_outpoint: String,
    pub root_inscription_id: String,
    pub items: Vec<PackItem>,
    pub target_offsets: Vec<u64>,
    pub total_required_value: u64,
}

pub fn plan_pack(
    root: &Utxo,
    contents: &[Utxo],
    _spec: &serde_json::Value
) -> Result<PackPlan, String> {

    // root validation
    if root.inscriptions.len() != 1 {
        return Err("root must contain exactly 1 inscription".into());
    }

    let root_ins = &root.inscriptions[0];

    if root_ins.satpoint.offset != 0 {
        return Err("root inscription must be at offset 0".into());
    }

    // collect items
    let mut items = Vec::new();

    for (i, utxo) in contents.iter().enumerate() {
        if utxo.inscriptions.len() != 1 {
            return Err(format!("content {} must contain exactly 1 inscription", i + 1));
        }

        let ins = &utxo.inscriptions[0];

        if ins.satpoint.offset != 0 {
            return Err(format!("content {} inscription must be at offset 0", i + 1));
        }

        items.push(PackItem {
            index: i as u32,
            outpoint: utxo.outpoint.clone(),
            inscription_id: ins.id.clone(),
        });
    }

    // 🔥 STANDARD: offsets = sequential after root
    let mut target_offsets = Vec::new();

    let mut current_offset = 1u64;

    for _ in &items {
        target_offsets.push(current_offset);
        current_offset += 1;
    }

    // total sats = root + items (minimal model)
    let total_required_value = 1 + items.len() as u64;

    Ok(PackPlan {
        root_outpoint: root.outpoint.clone(),
        root_inscription_id: root_ins.id.clone(),
        items,
        target_offsets,
        total_required_value,
    })
}