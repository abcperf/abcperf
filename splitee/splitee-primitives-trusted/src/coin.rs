pub mod statefull {
    use std::collections::HashSet;

    use splitee_crypto::{
        hash::hash,
        math::{FieldElement, PublicKey},
        prf::PRF,
    };
    use splitee_messages::{coin::statefull::CoinShare, Party};

    pub struct Coin {
        validity_sig_pub_keys: Vec<PublicKey>,
        prf_seed: FieldElement,
        prf: PRF,
        own_party_id: Party,
        max_fault_parties: u16,
    }

    impl Coin {
        pub fn combine_randomness(&self, valid_coin_shares: Vec<CoinShare>) -> Party {
            //TODO Checke Größe von Share-Vec und überprüfe ob wirklich von verschiedenen Sendern und ob auch alle anderen Parameter (TossID) passen

            let current_toss_id = valid_coin_shares[0].get_toss_id();

            let sender_ids: HashSet<Party> =
                valid_coin_shares.iter().map(|s| s.get_sender()).collect();

            // Überprüfe Validity Signatures aller übergebenen Coin Shares
            for share in valid_coin_shares {
                let share_toss_id = share.get_toss_id();
                let message = hash(share_toss_id.as_ref());
                let share_sender = share.get_sender().to_usize();
                let share_validity_signature = share.get_validity_sig();

                let share_validity = share_validity_signature
                    .verify(&message, &self.validity_sig_pub_keys[share_sender]);

                if !share_validity {
                    //return Party::MAX; TOSO
                }
            }

            //
            let result = self.prf.evaluate(current_toss_id.to_bignum());
            hash(result.as_ref()).into()
        }
    }
}

pub mod stateless {
    use splitee_crypto::{
        hash::hash,
        math::{ECPoint, PrivateKey, PrivateKeyShare},
        signature::Signature,
    };
    use splitee_messages::{coin::stateless::CoinShare, CoinToss, Party};

    pub struct Coin {
        validity_sig_priv_key: PrivateKey,
        result_key_share: PrivateKeyShare,
        own_party_id: Party,
    }

    impl Coin {
        pub fn create_coin_share(&self, toss_id: CoinToss) -> CoinShare {
            // Berechne Result Share
            let toss_point = ECPoint::hash_to_curve(toss_id.as_ref());
            let result_share =
                ECPoint::scalar_multiply(&self.result_key_share.to_fieldelement(), &toss_point);

            //Erzeuge Validity Signature
            let message = hash(
                [toss_id.as_ref(), result_share.as_ref()]
                    .concat()
                    .as_slice(),
            );
            let sig = Signature::sign(&message, &self.validity_sig_priv_key);

            //Returne Coin Share
            CoinShare::new_share(self.own_party_id, toss_id, result_share, sig)
        }
    }
}
