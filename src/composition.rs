use crate::models::Utxo;

#[derive(Debug, Clone)]
pub struct Composition {
    pub root_offset: u64,
    pub item_offsets: Vec<u64>,
    pub total_value: u64,

    pub root_inscription_id: String,
    pub item_inscription_ids: Vec<String>,
}

pub fn detect_composition(utxo: &Utxo) -> Option<Composition> {
    if utxo.inscriptions.is_empty() {
        return None;
    }

    let mut pairs: Vec<(u64, String)> = utxo
        .inscriptions
        .iter()
        .map(|i| (i.satpoint.offset, i.id.clone()))
        .collect();

    pairs.sort_by_key(|(offset, _)| *offset);

    if pairs.is_empty() {
        return None;
    }

    if pairs[0].0 != 0 {
        return None;
    }

    let root = &pairs[0];

    let items = if pairs.len() > 1 {
        pairs[1..].to_vec()
    } else {
        Vec::new()
    };

    Some(Composition {
        root_offset: root.0,
        item_offsets: items.iter().map(|(o, _)| *o).collect(),
        total_value: utxo.value,

        root_inscription_id: root.1.clone(),
        item_inscription_ids: items.iter().map(|(_, id)| id.clone()).collect(),
    })
}