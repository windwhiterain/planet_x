pub use px_protocol::fnv::{FNV_OFFSET, FNV_PRIME, fnv1a, fnv1a_bytes};

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

impl<T> HashField for crate::contract::Cooked<T> {
    fn hash_field(&self, hasher: &mut blake3::Hasher) {
        hasher.update(b"px_cook/cooked-field/v1");
        hasher.update(&self.key);
    }
}

pub trait PxKeyed {
    fn key(&self, hasher: &mut blake3::Hasher);
}

pub use blake3;
