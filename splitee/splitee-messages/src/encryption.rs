use splitee_crypto::math::{ECPoint, FieldElement};

pub struct CiphertextStatefull {
    c: FieldElement,
    u: ECPoint,
    h: FieldElement,
}

impl AsRef<[u8]> for CiphertextStatefull {
    fn as_ref(&self) -> &[u8] {
        unimplemented!()
    }
}

impl CiphertextStatefull {
    pub const fn new_ciphertext(c: FieldElement, u: ECPoint, h: FieldElement) -> Self {
        Self { c, u, h }
    }

    pub fn get_c(&self) -> FieldElement {
        todo!()
    }

    pub fn get_u(&self) -> ECPoint {
        unimplemented!()
    }

    pub fn get_h(&self) -> FieldElement {
        todo!()
    }
}
pub struct CiphertextStateless {
    c: FieldElement,
    u: ECPoint,
    u_strich: ECPoint,
}

impl AsRef<[u8]> for CiphertextStateless {
    fn as_ref(&self) -> &[u8] {
        unimplemented!()
    }
}

impl CiphertextStateless {
    pub const fn new_ciphertext(c: FieldElement, u: ECPoint, u_strich: ECPoint) -> Self {
        Self { c, u, u_strich }
    }

    pub fn get_c(&self) -> FieldElement {
        unimplemented!()
    }

    pub fn get_u(&self) -> ECPoint {
        unimplemented!()
    }

    pub fn get_u_strich(&self) -> ECPoint {
        unimplemented!()
    }
}

pub mod shoup_gennaro {
    use splitee_crypto::{chaum_pedersen::ZKPProofPair, math::ECPoint};

    use crate::{encryption::CiphertextStateless, Party};

    pub struct DecryptionShare {
        sender: Party,
        ciph: (CiphertextStateless, ZKPProofPair),
        glob_decryption_share: ECPoint,
        zkp: ZKPProofPair,
    }

    impl DecryptionShare {
        pub const fn new_share(
            sender: Party,
            ciphertext: (CiphertextStateless, ZKPProofPair),
            glob_share: ECPoint,
            zkp: ZKPProofPair,
        ) -> Self {
            Self {
                sender: (sender),
                ciph: (ciphertext),
                glob_decryption_share: (glob_share),
                zkp: (zkp),
            }
        }

        pub fn get_sender(&self) -> Party {
            self.sender
        }

        pub fn get_ciphertext(&self) -> (CiphertextStateless, ZKPProofPair) {
            unimplemented!()
        }

        pub fn get_glob_decryption_share(&self) -> ECPoint {
            unimplemented!()
        }

        pub fn get_zkp(&self) -> ZKPProofPair {
            unimplemented!()
        }
    }
}

pub mod stateless {
    use splitee_crypto::{chaum_pedersen::ZKPProofPair, math::ECPoint, signature::Signature};

    use crate::{encryption::CiphertextStateless, Party};

    pub struct DecryptionShare {
        sender: Party,
        ciph: (CiphertextStateless, ZKPProofPair),
        glob_decryption_share: ECPoint,
        validity_sig: Signature,
    }

    impl DecryptionShare {
        pub const fn new_share(
            sender: Party,
            ciphertext: (CiphertextStateless, ZKPProofPair),
            glob_share: ECPoint,
            validity_sig: Signature,
        ) -> Self {
            Self {
                sender,
                ciph: ciphertext,
                glob_decryption_share: glob_share,
                validity_sig,
            }
        }

        pub fn get_sender(&self) -> Party {
            self.sender
        }

        pub fn get_ciphertext(&self) -> (CiphertextStateless, ZKPProofPair) {
            unimplemented!()
        }

        pub fn get_glob_decryption_share(&self) -> ECPoint {
            unimplemented!()
        }

        pub fn get_validity_sig(&self) -> Signature {
            unimplemented!()
        }
    }
}

pub mod statefull {
    use splitee_crypto::signature::Signature;

    use crate::{encryption::CiphertextStatefull, Party};

    pub struct DecryptionShare {
        sender: Party,
        ciph: CiphertextStatefull,
        validity_sig: Signature,
    }

    impl DecryptionShare {
        pub const fn new_share(
            sender: Party,
            ciphertext: CiphertextStatefull,
            validity_sig: Signature,
        ) -> Self {
            Self {
                sender,
                ciph: ciphertext,
                validity_sig,
            }
        }

        pub fn get_sender(&self) -> Party {
            self.sender
        }

        pub fn get_ciphertext(&self) -> CiphertextStatefull {
            unimplemented!()
        }

        pub fn get_validity_sig(&self) -> Signature {
            unimplemented!()
        }
    }
}
