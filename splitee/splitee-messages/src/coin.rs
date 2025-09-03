pub mod stateless {
    use splitee_crypto::{math::ECPoint, signature::Signature};

    use crate::{CoinToss, Party};

    pub struct CoinShare {
        sender: Party,
        toss_id: CoinToss,
        result_share: ECPoint,
        validity_sig: Signature,
    }

    impl CoinShare {
        pub const fn new_share(
            senderID: Party,
            toss_id: CoinToss,
            result_share: ECPoint,
            validity_sig: Signature,
        ) -> Self {
            Self {
                sender: (senderID),
                toss_id: (toss_id),
                result_share: (result_share),
                validity_sig: (validity_sig),
            }
        }

        pub fn get_sender(&self) -> Party {
            self.sender
        }

        pub fn get_toss_id(&self) -> CoinToss {
            self.toss_id
        }

        pub fn get_validity_sig(&self) -> Signature {
            unimplemented!()
        }

        pub fn get_result_share(&self) -> ECPoint {
            unimplemented!()
        }
    }
}

pub mod cachin {
    use splitee_crypto::{chaum_pedersen::ZKPProofPair, math::ECPoint};

    use crate::{CoinToss, Party};

    pub struct CoinShare {
        sender: Party,
        toss_id: CoinToss,
        result_share: ECPoint,
        zkp: ZKPProofPair,
    }

    impl CoinShare {
        pub const fn new_share(
            senderID: Party,
            toss_id: CoinToss,
            result_share: ECPoint,
            zkp: ZKPProofPair,
        ) -> Self {
            Self {
                sender: (senderID),
                toss_id: (toss_id),
                result_share: (result_share),
                zkp: (zkp),
            }
        }

        pub fn get_sender(&self) -> Party {
            self.sender
        }

        pub fn get_toss_id(&self) -> CoinToss {
            self.toss_id
        }

        pub fn get_result_share(&self) -> ECPoint {
            unimplemented!()
        }

        pub fn get_zkp(&self) -> ZKPProofPair {
            unimplemented!()
        }
    }
}

pub mod statefull {
    use splitee_crypto::signature::Signature;

    use crate::{CoinToss, Party};

    pub struct CoinShare {
        sender: Party,
        toss_id: CoinToss,
        validity_sig: Signature,
    }

    impl CoinShare {
        pub const fn new_share(
            senderID: Party,
            toss_id: CoinToss,
            validity_sig: Signature,
        ) -> Self {
            Self {
                sender: (senderID),
                toss_id: (toss_id),
                validity_sig: (validity_sig),
            }
        }

        pub fn get_sender(&self) -> Party {
            self.sender
        }

        pub fn get_toss_id(&self) -> CoinToss {
            self.toss_id
        }

        pub fn get_validity_sig(&self) -> Signature {
            unimplemented!()
        }
    }
}
