use hashbar::{Hashbar, Hasher};
use remote_attestation_shared::types::{PubKey, RawSignature};
use serde::{Deserialize, Serialize};
use sgx_types::types::Mac128bit;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct Signature(pub(crate) RawSignature);

impl Hashbar for Signature {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        hasher.update(self.0.as_ref());
    }
}

/// The struct that defines an attestation
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Attestation {
    pub(crate) pub_key: PubKey,
    #[serde(with = "serde_bytes")]
    pub(crate) intel_report: Box<[u8]>,
}

impl Attestation {
    pub(crate) fn new(pub_key: impl Into<PubKey>, intel_report: Box<[u8]>) -> Self {
        Self {
            pub_key: pub_key.into(),
            intel_report,
        }
    }
}

impl Hashbar for Attestation {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        hasher.update(&self.pub_key.gx);
        hasher.update(&self.pub_key.gy);
        hasher.update(&self.intel_report);
    }
}

/// The struct that defines an attestation
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Handshake {
    #[serde(with = "serde_bytes")]
    pub(crate) proposed_secret: Box<[u8]>,
    pub(crate) proposed_secret_mac: Mac128bit,
}
