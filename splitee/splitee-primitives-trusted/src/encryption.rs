pub mod statefull {
    use splitee_crypto::{
        hash::hash,
        math::{ECPoint, PrivateKey, PublicKey},
    };
    use splitee_messages::{encryption::statefull::DecryptionShare, Party};

    pub struct ThresholdEncryption {
        validity_sig_pub_keys: Vec<PublicKey>,
        global_dec_priv_key: PrivateKey,
        own_party_id: Party,
    }

    impl ThresholdEncryption {
        pub fn combine_global_decryption(
            &self,
            valid_decryption_shares: Vec<DecryptionShare>,
        ) -> Box<[u8]> {
            //TODO Checke Größe von Share-Vec und überprüfe ob wirklich von verschiedenen Sendern und ob auch alle anderen Parameter (TossID) passen
            let current_ciphertext = valid_decryption_shares[0].get_ciphertext();

            for share in valid_decryption_shares {
                let share_ciphertext = share.get_ciphertext();
                let share_sender = share.get_sender().to_usize();
                let share_validity_signature = share.get_validity_sig();
                let message = hash(share_ciphertext.as_ref());

                let share_validity = share_validity_signature
                    .verify(&message, &self.validity_sig_pub_keys[share_sender]);

                if (!share_validity) {
                    // TODO Invalide Signatur festlegen
                    return todo!();
                }
            }

            let c = current_ciphertext.get_c();
            let temp = ECPoint::scalar_multiply(
                &self.global_dec_priv_key.to_bignum(),
                &current_ciphertext.get_u(),
            );
            //let temp_2 = hash(temp.as_ref()).into();
            //let cleartext = FieldElement::xor(&c, &temp_2);

            // let h = hash([cleartext.as_ref(), temp.as_ref()].concat().as_slice()).into();
            // if !FieldElement::equals(&h, &current_ciphertext.get_h()) {
            //     //TODO Später in Ruhe anschauen
            //     return todo!();
            // }

            todo!()
        }
    }
}

pub mod stateless {
    use splitee_crypto::{
        chaum_pedersen::{verify_zkp, ZKPProofPair},
        hash::hash,
        math::{ECPoint, PrivateKey, PrivateKeyShare},
        signature::Signature,
    };
    use splitee_messages::{
        encryption::{stateless::DecryptionShare, CiphertextStateless},
        Party,
    };

    pub struct ThresholdEncryption {
        validity_sig_priv_key: PrivateKey,
        global_key_share: PrivateKeyShare,
        own_party_id: Party,
        group_generator_1: ECPoint,
        group_generator_2: ECPoint,
    }

    impl ThresholdEncryption {
        pub fn create_decryption_share(
            &self,
            ciphertext: (CiphertextStateless, ZKPProofPair),
        ) -> DecryptionShare {
            let u = ciphertext.0.get_u();
            let u_strich = ciphertext.0.get_u_strich();
            let zkp_to_verify = ciphertext.1;

            let zkp_validity = verify_zkp(
                &self.group_generator_1,
                &self.group_generator_2,
                &u,
                &u_strich,
                &zkp_to_verify,
            );

            if (!zkp_validity) {
                // TODO Invalides Share definieren und ausgeben
                print!("?");
                //return DecryptionShare_Stateless{};
            }

            let global_dec_share =
                ECPoint::scalar_multiply(&self.global_key_share.to_fieldelement(), &u);

            let message = hash(
                [
                    ciphertext.0.as_ref(),
                    ciphertext.1.as_ref(),
                    global_dec_share.as_ref(),
                ]
                .concat()
                .as_slice(),
            )
            .into(); //TODO as_ref für ZKP-Proof implementieren
            let validity_sig = Signature::sign(&message, &self.validity_sig_priv_key);

            DecryptionShare::new_share(
                self.own_party_id,
                ciphertext,
                global_dec_share,
                validity_sig,
            )
        }
    }
}
