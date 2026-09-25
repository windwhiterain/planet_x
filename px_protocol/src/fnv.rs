pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

pub const fn fnv1a_bytes(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        index += 1;
    }
    hash
}

pub const fn fnv1a(text: &str) -> u64 {
    fnv1a_bytes(text.as_bytes())
}
