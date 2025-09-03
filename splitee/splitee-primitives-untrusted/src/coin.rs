pub mod statefull {
    use splitee_crypto::{
        hash::hash,
        math::{PrivateKey, PublicKey},
        signature::Signature,
    };
    use splitee_messages::{coin::statefull::CoinShare, CoinToss, Party};

    pub struct Coin {
        validity_sig_priv_key: PrivateKey,
        validity_sig_pub_keys: Vec<PublicKey>,
        valid_coin_shares: Vec<CoinShare>, //TODO Wahrscheinlich eher rausziehen, dass Anwendung sich drum kümmert
        own_party_id: Party,
    }

    impl Coin {
        pub fn init(&self) {
            todo!()
        }

        pub fn create_coin_share(&self, toss_id: CoinToss) -> CoinShare {
            // Create a validity signature over the hash of the CoinTossID.
            let message = hash(toss_id.as_ref());
            let sig = Signature::sign(&message, &self.validity_sig_priv_key);
            CoinShare::new_share(self.own_party_id, toss_id, sig)
        }

        pub fn verify_coin_share(&self, coin_share_to_verify: CoinShare) -> bool {
            // Verify the validty signature over the hash of the CoinShares CoinTossID using the senders PublicKey. //TODO Kompakter machen, geht sicher irgendwie
            let sender_id = coin_share_to_verify.get_sender().to_usize();
            let toss_id = coin_share_to_verify.get_toss_id();
            let message = hash(toss_id.as_ref());
            let sig_to_verify = coin_share_to_verify.get_validity_sig();

            sig_to_verify.verify(&message, &self.validity_sig_pub_keys[sender_id])
        }
    }
}

pub mod stateless {
    use std::collections::HashSet;

    use splitee_crypto::{
        hash::hash,
        math::{ECPoint, FieldElement, PublicKey},
        signature::Signature,
        util::LagrangeCoefficients,
    };
    use splitee_messages::{coin::stateless::CoinShare, Party};

    pub struct Coin {
        validity_sig_pub_keys: Vec<PublicKey>,
        valid_coin_shares: Vec<CoinShare>,
        own_party_id: Party,
        lagrange: LagrangeCoefficients,
    }

    impl Coin {
        pub fn init(&self) {
            todo!()
        }

        pub fn verify_coin_share(&self, coin_share_to_verify: CoinShare) -> bool {
            // Verify the validty signature over the hash of the CoinShares CoinTossID and Result Share using the senders PublicKey. //TODO Kompakter machen, geht sicher irgendwie
            let sender_id = coin_share_to_verify.get_sender().to_usize();
            let toss_id = coin_share_to_verify.get_toss_id();
            let sig_to_verify = coin_share_to_verify.get_validity_sig();
            let result_share = coin_share_to_verify.get_result_share();
            let message = hash(
                [toss_id.as_ref(), result_share.as_ref()]
                    .concat()
                    .as_slice(),
            );

            sig_to_verify.verify(&message, &self.validity_sig_pub_keys[sender_id])
        }

        pub fn combine_randomness(&self) -> Party {
            let mut result: ECPoint = ECPoint::get_point_at_infinity();

            //TODO Überprüfen dass keine Sender zweimal und das Coin Shares zu aktuellem Toss gehören

            //Extract SenderIDs from CoinShares and calculate approptiate Lagrange-Coefficients
            let sender_ids: Vec<FieldElement> = self
                .valid_coin_shares
                .iter()
                .map(|s| s.get_sender().to_fieldelement())
                .collect();

            let lagrange_coefficients = self.lagrange.calculate_lagrange_coefficients(sender_ids);

            //Reconstruct global unique ECPoint through polynomial interpolation
            for i in 0..self.valid_coin_shares.len() {
                let result_share_temp: ECPoint = self.valid_coin_shares[i].get_result_share();
                let temp: ECPoint =
                    ECPoint::scalar_multiply(&lagrange_coefficients[i], &result_share_temp);
                result = ECPoint::point_addition(&result, &temp);
            }

            hash(result.as_ref()).into()
            //HashFunctions::hash_to_party_id_from_ecpoint(&result)
        }
    }
}

pub mod cachin {
    use splitee_crypto::{
        chaum_pedersen::*,
        hash::hash,
        math::{ECPoint, FieldElement, PrivateKeyShare, PublicKey},
        util::LagrangeCoefficients,
    };
    use splitee_messages::{coin::cachin::CoinShare, CoinToss, Party};

    pub struct Coin {
        priv_key: PrivateKeyShare,
        pub_keys: Vec<PublicKey>,
        group_generator: ECPoint,
        valid_coin_shares: Vec<CoinShare>,
        own_party_id: Party,
        lagrange: LagrangeCoefficients,
    }

    impl Coin {
        pub fn init(&self) {
            //let gennaro_key_gen = GennaroKeyGen{};
            //let (priv_key,pub_key,verif_keys) = gennaro_key_gen.execute_dkg();
        }

        pub fn create_coin_share(&self, toss_id: CoinToss) -> CoinShare {
            // Create the result share and the accompanying ZKP
            let coin_id_hash = ECPoint::hash_to_curve(toss_id.as_ref()).into();
            let result_share =
                ECPoint::scalar_multiply(&self.priv_key.to_fieldelement(), &coin_id_hash);
            let (coin_share_zkp, _dont_care) = create_zkp(
                &self.group_generator,
                &coin_id_hash,
                &self.priv_key.to_fieldelement(),
            );

            CoinShare::new_share(self.own_party_id, toss_id, result_share, coin_share_zkp)
        }

        pub fn verify_coin_share(&self, coin_share_to_verify: CoinShare) -> bool {
            //Verify the validity of the ZKP, to determine the validity of the CoinShare
            let sender_id = coin_share_to_verify.get_sender().to_usize();
            let coin_id_hash =
                ECPoint::hash_to_curve(coin_share_to_verify.get_toss_id().as_ref()).into();
            let coin_share_zkp = coin_share_to_verify.get_zkp();
            let coin_result_share = coin_share_to_verify.get_result_share();

            verify_zkp(
                &self.group_generator,
                &coin_id_hash,
                &self.pub_keys[sender_id].to_ecpoint(),
                &coin_result_share,
                &coin_share_zkp,
            )
        }

        pub fn combine_randomness(&self) -> Party {
            let mut result: ECPoint = ECPoint::get_point_at_infinity();

            let sender_ids: Vec<FieldElement> = self
                .valid_coin_shares
                .iter()
                .map(|s| s.get_sender().to_fieldelement())
                .collect();
            let lagrange_coefficients = self.lagrange.calculate_lagrange_coefficients(sender_ids);

            for i in 0..self.valid_coin_shares.len() {
                let result_share_temp: ECPoint = self.valid_coin_shares[i].get_result_share();
                let temp = ECPoint::scalar_multiply(&lagrange_coefficients[i], &result_share_temp);
                result = ECPoint::point_addition(&result, &temp);
            }

            hash(result.as_ref()).into()
        }
    }
}
