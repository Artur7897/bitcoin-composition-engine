// offsets.rs

pub fn calculate_offsets(root_postage: u64, item_postages: &[u64]) -> Vec<u64> {
    let mut offsets = Vec::new();
    let mut current_offset = 0u64;

    offsets.push(current_offset);

    current_offset += root_postage;

    for postage in item_postages {
        offsets.push(current_offset);
        current_offset += *postage;
    }

    offsets
}