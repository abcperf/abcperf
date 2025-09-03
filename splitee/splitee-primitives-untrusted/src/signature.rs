pub mod statefull {
    use splitee_crypto::{
        hash::hash,
        math::{PrivateKey, PublicKey},
        signature::Signature,
    };
    use splitee_messages::{signature::statefull::SignatureShare, Party};

    pub struct ThresholdSignature {
        validity_sig_priv_key: PrivateKey,
        validity_sig_pub_keys: Vec<PublicKey>,
        global_public_key: PublicKey,
        valid_signature_shares: Vec<SignatureShare>,
        own_party_id: Party,
    }

    impl ThresholdSignature {
        pub fn create_signature_share(&self, msg_to_sign: Box<[u8]>) -> SignatureShare {
            // Create a validity signature on the message to sign
            let hash = hash(&msg_to_sign);
            let sig = Signature::sign(&hash, &self.validity_sig_priv_key);
            let return_share = SignatureShare::new_share(self.own_party_id, msg_to_sign, sig);

            return_share
        }

        pub fn verify_signature_share(&self, sig_share_to_verifiy: SignatureShare) -> bool {
            // Verify the validity signature on the message to sign using the senders PublicKey
            let sender_id = sig_share_to_verifiy.get_sender().to_usize();
            let message = sig_share_to_verifiy.get_message();
            let sig_to_verify = sig_share_to_verifiy.get_validity_sig();

            let message = hash(&message);

            sig_to_verify.verify(&message, &self.validity_sig_pub_keys[sender_id])
        }

        pub fn verify_global_signature(&self, sig_to_verify: Signature, msg: &[u8]) -> bool {
            let msg = hash(&msg);

            sig_to_verify.verify(&msg, &self.global_public_key)
        }
    }
}

pub mod stateless {
    use splitee_crypto::{
        hash::hash,
        math::{ECPoint, FieldElement, PublicKey},
        pairing_signature::PairingSignature,
        util::LagrangeCoefficients,
    };
    use splitee_messages::{signature::stateless::SignatureShare, Party};

    pub struct ThresholdSignature {
        global_public_key: PublicKey,
        validity_sig_pub_keys: Vec<PublicKey>,
        valid_signature_shares: Vec<SignatureShare>,
        own_party_id: Party,
        lagrange: LagrangeCoefficients,
    }

    impl ThresholdSignature {
        pub fn verify_signature_share(&self, sig_share_to_verify: SignatureShare) -> bool {
            // Verify the validity signature on the message to sign and the global signature share using the senders PublicKey
            let sender_id = sig_share_to_verify.get_sender().to_usize();
            let msg = sig_share_to_verify.get_message();
            let glob_sig_share = sig_share_to_verify.get_global_sigshare();
            let sig_to_verify = sig_share_to_verify.get_validity_sig();
            let message = hash([msg.as_ref(), glob_sig_share.as_ref()].concat().as_slice());

            sig_to_verify.verify(&message, &self.validity_sig_pub_keys[sender_id])
        }

        pub fn combine_global_signature(&self) -> PairingSignature {
            let mut result: ECPoint = ECPoint::get_point_at_infinity();

            //Extract SenderIDs from CoinShares and calculate approptiate Lagrange-Coefficients
            let sender_ids: Vec<FieldElement> = self
                .valid_signature_shares
                .iter()
                .map(|s| s.get_sender().to_fieldelement())
                .collect();
            let lagrange_coefficients = self.lagrange.calculate_lagrange_coefficients(sender_ids);

            //Reconstruct global unique ECPoint through polynomial interpolation
            for i in 0..self.valid_signature_shares.len() {
                let result_share_temp: ECPoint =
                    self.valid_signature_shares[i].get_global_sigshare();
                let temp: ECPoint =
                    ECPoint::scalar_multiply(&lagrange_coefficients[i], &result_share_temp);
                result = ECPoint::point_addition(&result, &temp);
            }

            PairingSignature::from(result)
        }

        pub fn verify_global_signature(
            &self,
            sig_to_verify: PairingSignature,
            msg: Box<[u8]>,
        ) -> bool {
            PairingSignature::verify(&msg, &self.global_public_key, &sig_to_verify)
        }
    }
}

pub mod bls {
    use splitee_crypto::{
        math::{ECPoint, FieldElement, PrivateKeyShare, PublicKey},
        pairing_signature::PairingSignature,
        util::LagrangeCoefficients,
    };
    use splitee_messages::{signature::bls::SignatureShare, Party};

    pub struct ThresholdSignature {
        global_public_key: PublicKey,
        priv_key_share: PrivateKeyShare,
        verification_keys: Vec<PublicKey>,
        valid_signature_shares: Vec<SignatureShare>,
        group_generator: ECPoint,
        own_party_id: Party,
        lagrange: LagrangeCoefficients,
    }

    impl ThresholdSignature {
        pub fn create_signature_share(&self, msg_to_sign: Box<[u8]>) -> SignatureShare {
            //Create the global signature share on the message to sign
            let msg_hash = ECPoint::hash_to_curve(msg_to_sign.as_ref()).into();
            let glob_sig_share =
                ECPoint::scalar_multiply(&self.priv_key_share.to_fieldelement(), &msg_hash);

            SignatureShare::new_share(self.own_party_id, msg_to_sign, glob_sig_share)
        }

        pub fn verify_signature_share(&self, sig_share_to_verify: SignatureShare) -> bool {
            let sender_id = sig_share_to_verify.get_sender().to_usize();
            let msg = sig_share_to_verify.get_message();
            let glob_sig_share = sig_share_to_verify.get_global_sigshare();

            let msg_hash = ECPoint::hash_to_curve(msg.as_ref()).into();

            PairingSignature::verify_irgendwas(
                &msg_hash,
                &glob_sig_share,
                &self.group_generator,
                &self.verification_keys[sender_id].to_ecpoint(),
            )
        }

        pub fn combine_global_signature(&self) -> PairingSignature {
            let mut result: ECPoint = ECPoint::get_point_at_infinity();

            // TODO Extract Interpolation Set from valid CoinShares
            let sender_ids: Vec<FieldElement> = self
                .valid_signature_shares
                .iter()
                .map(|s| s.get_sender().to_fieldelement())
                .collect();
            let lagrange_coefficients = self.lagrange.calculate_lagrange_coefficients(sender_ids);

            for i in 0..self.valid_signature_shares.len() {
                let result_share_temp: ECPoint =
                    self.valid_signature_shares[i].get_global_sigshare();
                let temp: ECPoint =
                    ECPoint::scalar_multiply(&lagrange_coefficients[i], &result_share_temp);
                result = ECPoint::point_addition(&result, &temp);
            }

            PairingSignature::from(result)
        }

        pub fn verify_global_signature(
            &self,
            sig_to_verify: PairingSignature,
            msg: Box<[u8]>,
        ) -> bool {
            PairingSignature::verify(&msg, &self.global_public_key, &sig_to_verify)
        }
    }
}
