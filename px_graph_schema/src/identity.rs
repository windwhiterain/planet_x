//! 身份哈希：FNV-1a 的 64 位。**只做变更检测，不做安全**（与 §19.1 同一套口径）。

/// FNV-1a 的 64 位常数（载荷指纹与源码哈希共用一套）。
pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 载荷指纹走字节版；源码哈希走字符串版。
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

/// 多段源码的 FNV-1a：算子的 `SOURCE_HASH` 要覆盖它的**共享依赖**（§28.2）。
///
/// 每段带 8 字节长度前缀 ⇒ `["ab","c"]` 与 `["a","bc"]` 不会撞；顺序由调用点写死
/// （同一份列表换个顺序 = 另一个哈希，也算是"源码变了"）。
pub const fn fnv1a_sources(sources: &[&str]) -> u64 {
    let mut hash = FNV_OFFSET;
    let mut index = 0;
    while index < sources.len() {
        let bytes = sources[index].as_bytes();
        let mut length = bytes.len() as u64;
        let mut written = 0;
        while written < 8 {
            hash ^= (length & 0xff) as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
            length >>= 8;
            written += 1;
        }
        let mut at = 0;
        while at < bytes.len() {
            hash ^= bytes[at] as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
            at += 1;
        }
        index += 1;
    }
    hash
}
