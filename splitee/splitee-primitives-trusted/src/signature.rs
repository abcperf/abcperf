pub mod statefull {
    use splitee_crypto::{
        hash::hash,
        math::{PrivateKey, PublicKey},
        signature::Signature,
    };
    use splitee_messages::{signature::statefull::SignatureShare, Party};

    pub struct ThresholdSignature {
        validity_sig_pub_keys: Vec<PublicKey>,
        global_sig_priv_key: PrivateKey,
        own_party_id: Party,
    }

    impl ThresholdSignature {
        pub fn combine_global_signature(
            &self,
            valid_signature_shares: Vec<SignatureShare>,
        ) -> Signature {
            //TODO Checke Größe von Share-Vec und überprüfe ob wirklich von verschiedenen Sendern und ob auch alle anderen Parameter (TossID) passen
            let current_message = valid_signature_shares[0].get_message();
            let current_message = hash(current_message);
            for share in valid_signature_shares {
                let share_message = share.get_message();
                let share_sender = share.get_sender().to_usize();
                let share_validity_signature = share.get_validity_sig();
                let share_message = hash(share_message);

                let share_validity = share_validity_signature
                    .verify(&share_message, &self.validity_sig_pub_keys[share_sender]);

                if !share_validity {
                    // TODO Invalide Signatur festlegen
                    return todo!();
                }
            }

            Signature::sign(&current_message, &self.global_sig_priv_key)
        }
    }
}

pub mod stateless {
    use splitee_crypto::{
        hash::hash,
        math::{ECPoint, PrivateKey, PrivateKeyShare},
        signature::Signature,
    };
    use splitee_messages::{signature::stateless::SignatureShare, Party};

    pub struct ThresholdSignature {
        validity_sig_priv_key: PrivateKey,
        global_key_share: PrivateKeyShare,
        own_party_id: Party,
    }

    impl ThresholdSignature {
        pub fn create_signature_share(&self, msg_to_sign: Box<[u8]>) -> SignatureShare {
            let msg_point = ECPoint::hash_to_curve(&msg_to_sign).into();
            let glob_sig_share =
                ECPoint::scalar_multiply(&self.global_key_share.to_fieldelement(), &msg_point);

            let message = hash(
                [msg_to_sign.as_ref(), glob_sig_share.as_ref()]
                    .concat()
                    .as_slice(),
            )
            .into();
            let sig = Signature::sign(&message, &self.validity_sig_priv_key);

            SignatureShare::new_share(self.own_party_id, msg_to_sign, glob_sig_share, sig)
        }
    }
}
