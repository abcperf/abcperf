use blake3::traits::digest::Digest;
use blake3::Hasher;

pub struct HashValue {
    hash: [u8; 32],
}

pub fn hash(input: &[u8]) -> HashValue {
    let hasher = Hasher::new().chain_update(input).finalize();
    HashValue {
        hash: hasher.into(),
    }
}

impl AsRef<[u8]> for HashValue {
    fn as_ref(&self) -> &[u8] {
        &self.hash
    }
}
