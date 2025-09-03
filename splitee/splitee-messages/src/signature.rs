pub mod bls {
    use splitee_crypto::math::ECPoint;

    use crate::Party;

    pub struct SignatureShare {
        sender: Party,
        msg: Box<[u8]>,
        global_sig_share: ECPoint,
    }

    impl SignatureShare {
        pub const fn new_share(senderID: Party, msg: Box<[u8]>, glob_sig_share: ECPoint) -> Self {
            Self {
                sender: (senderID),
                msg: (msg),
                global_sig_share: (glob_sig_share),
            }
        }

        pub fn get_sender(&self) -> Party {
            self.sender
        }

        pub fn get_message(&self) -> Box<[u8]> {
            unimplemented!()
        }

        pub fn get_global_sigshare(&self) -> ECPoint {
            unimplemented!()
        }
    }
}

pub mod stateless {
    use splitee_crypto::{math::ECPoint, signature::Signature};

    use crate::Party;

    pub struct SignatureShare {
        sender: Party,
        msg: Box<[u8]>,
        global_sig_share: ECPoint,
        validity_sig: Signature,
    }

    impl SignatureShare {
        pub const fn new_share(
            senderID: Party,
            msg: Box<[u8]>,
            glob_sig_share: ECPoint,
            validity_sig: Signature,
        ) -> Self {
            Self {
                sender: (senderID),
                msg: (msg),
                global_sig_share: (glob_sig_share),
                validity_sig: (validity_sig),
            }
        }

        pub fn get_sender(&self) -> Party {
            self.sender
        }

        pub fn get_message(&self) -> Box<[u8]> {
            unimplemented!()
        }

        pub fn get_global_sigshare(&self) -> ECPoint {
            unimplemented!()
        }

        pub fn get_validity_sig(&self) -> Signature {
            unimplemented!()
        }
    }
}

pub mod statefull {
    use splitee_crypto::signature::Signature;

    use crate::Party;

    pub struct SignatureShare {
        sender: Party,
        msg: Box<[u8]>,
        validity_sig: Signature,
    }

    impl SignatureShare {
        pub const fn new_share(senderID: Party, msg: Box<[u8]>, validity_sig: Signature) -> Self {
            Self {
                sender: (senderID),
                msg: (msg),
                validity_sig: (validity_sig),
            }
        }

        pub fn get_sender(&self) -> Party {
            self.sender
        }

        pub fn get_message(&self) -> &[u8] {
            unimplemented!()
        }

        pub fn get_validity_sig(&self) -> Signature {
            unimplemented!()
        }
    }
}
