use hashbar::Hashbar;
use remote_attestation_shared::types::{PubKey, RawSignature};
use serde::{Deserialize, Serialize};
use usig::{Count, Counter};

/// The struct that defines a signature
// This struct is necessary due to only being able to pass parameters of primitive types in the .edl file
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Signature {
    pub(crate) signature: RawSignature,
    pub(crate) counter: u64,
}

impl Signature {
    pub(crate) fn new(signature: impl Into<RawSignature>, counter: u64) -> Self {
        Self {
            signature: signature.into(),
            counter,
        }
    }
}

impl Hashbar for Signature {
    fn hash<H: hashbar::Hasher>(&self, hasher: &mut H) {
        hasher.update(self.signature.as_ref());
        hasher.update(&self.counter.to_le_bytes());
    }
}

impl Counter for Signature {
    /// Get the counter value of this USIG signature
    fn counter(&self) -> usig::Count {
        Count(self.counter)
    }
}

/// The struct that defines an attestation
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attestation {
    pub(crate) pub_key: PubKey,
    counter: u64,
    #[serde(with = "serde_bytes")]
    pub(crate) intel_report: Box<[u8]>,
}

impl Attestation {
    pub(crate) fn new(
        pub_key: impl Into<PubKey>,
        counter: u64,
        intel_report: impl Into<Box<[u8]>>,
    ) -> Self {
        Self {
            pub_key: pub_key.into(),
            counter,
            intel_report: intel_report.into(),
        }
    }
}

impl Hashbar for Attestation {
    fn hash<H: hashbar::Hasher>(&self, hasher: &mut H) {
        hasher.update(&self.pub_key.gx);
        hasher.update(&self.pub_key.gy);
        hasher.update(&self.counter.to_le_bytes());
        hasher.update(self.intel_report.as_ref());
    }
}
