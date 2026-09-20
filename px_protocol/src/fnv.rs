//! FNV-1a 的 64 位：线格式那一侧的**指纹**用的（清单帧的 `fingerprint`、载荷指纹）。
//!
//! ⚠ **只做变更检测，不做安全**（与 §19.1 同一套口径）。常量放在这里，是因为
//! "载荷的指纹"与"源码的指纹"必须共用同一套常数 —— 两处各写一份就是两处口径。
//! 键那一侧（`px_graph_schema::identity`）从这里 re-export。

/// FNV-1a 的 64 位常数。
pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 载荷指纹走**字节**版；源码哈希走**字符串**版。
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
