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

/// 单段：`(长度前缀, 内容)` 折进去 —— `const` 上下文可用。
///
/// ⚠ 常量里不能调用 `blake3` 那一套（`Hasher` 不是 const），所以源码哈希走 FNV。
pub const fn fnv1a_stage(mut hash: u64, text: &str) -> u64 {
    let bytes = text.as_bytes();
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
    hash
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

/// **一个字段怎么进键**：按自己的类型写字节，不丢精度、不靠格式化。
///
/// ⚠ 闭集：只列仓里参数真用到的那些类型。加一种就要在这里加一条 ——
/// 编译器会当场报（`PxParams` 生成的 `impl` 找不到实现）。
pub trait HashField {
    fn hash_field(&self, hasher: &mut blake3::Hasher);
}

macro_rules! hash_le {
    ($($ty:ty),+ $(,)?) => {
        $(impl HashField for $ty {
            fn hash_field(&self, hasher: &mut blake3::Hasher) {
                hasher.update(&self.to_le_bytes());
            }
        })+
    };
}

hash_le!(f32, f64, u8, u16, u32, u64, i8, i16, i32, i64);

impl HashField for bool {
    fn hash_field(&self, hasher: &mut blake3::Hasher) {
        hasher.update(&[*self as u8]);
    }
}

impl<T: HashField, const N: usize> HashField for [T; N] {
    fn hash_field(&self, hasher: &mut blake3::Hasher) {
        for item in self {
            item.hash_field(hasher);
        }
    }
}

impl HashField for str {
    fn hash_field(&self, hasher: &mut blake3::Hasher) {
        hasher.update(&(self.len() as u64).to_le_bytes());
        hasher.update(self.as_bytes());
    }
}

impl HashField for String {
    fn hash_field(&self, hasher: &mut blake3::Hasher) {
        self.as_str().hash_field(hasher);
    }
}

/// **超参数**：一个 struct 自己说明它贡献给键的是什么。
///
/// ⚠ 由 `#[derive(PxParams)]`（住 `px_derive`）按字段列表生成。手写的话，
/// "加了字段却忘了进 `key`"是个**静默** bug —— 改了参数却命中旧产物。
pub trait PxKeyed {
    fn key(&self, hasher: &mut blake3::Hasher);
}

/// re-export：生成出来的 `impl` 里写的是 `::blake3::Hasher`，
/// 用的人（schema crate）不必自己再依赖 `blake3`。
pub use blake3;
