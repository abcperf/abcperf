pub mod statefull {
    use splitee_crypto::{
        hash::hash,
        math::{ECPoint, FieldElement, PrivateKey, PublicKey},
        signature::Signature,
    };
    use splitee_messages::{
        encryption::{statefull::DecryptionShare, CiphertextStatefull},
        Party,
    };

    pub struct ThresholdEncryption {
        validity_sig_priv_key: PrivateKey,
        validity_sig_pub_keys: Vec<PublicKey>,
        global_pub_key: PublicKey,
        valid_decryption_shares: Vec<DecryptionShare>,
        group_generator: ECPoint,
        own_party_id: Party,
    }

    impl ThresholdEncryption {
        pub fn enrcypt(&self, msg_to_encrypt: Box<[u8]>) -> CiphertextStatefull {
            let r = FieldElement::rand();

            let temp1 = ECPoint::scalar_multiply(&r, &self.global_pub_key.to_ecpoint());
            //let temp2 = hash(temp1.as_ref()).into();
            //let temp3 = todo!(); //msg_to_encrypt.to_bignum();
            let c = todo!(); //FieldElement::xor(&temp2, &temp3);

            let u = ECPoint::scalar_multiply(&r, &self.group_generator);

            let h: FieldElement = hash(
                [msg_to_encrypt.as_ref(), temp1.as_ref(), u.as_ref()]
                    .concat()
                    .as_slice(),
            )
            .into();

            CiphertextStatefull::new_ciphertext(c, u, h) // Todo auch später nochmal machen
        }

        pub fn create_decryption_share(&self, ciphertext: CiphertextStatefull) -> DecryptionShare {
            let dec_share_msg = hash(ciphertext.as_ref()).into();
            let sig = Signature::sign(&dec_share_msg, &self.validity_sig_priv_key);

            DecryptionShare::new_share(self.own_party_id, ciphertext, sig)
        }

        pub fn verify_decryption_share(&self, dec_share_to_verify: DecryptionShare) -> bool {
            let sender_id = dec_share_to_verify.get_sender().to_usize();
            let ciphertext = dec_share_to_verify.get_ciphertext();
            let message = hash(ciphertext.as_ref());
            let sig_to_verify = dec_share_to_verify.get_validity_sig();

            sig_to_verify.verify(&message, &self.validity_sig_pub_keys[sender_id])
        }
    }
}

pub mod stateless {
    use splitee_crypto::{
        chaum_pedersen::{create_zkp, ZKPProofPair},
        hash::hash,
        math::{ECPoint, FieldElement, PublicKey},
        util::LagrangeCoefficients,
    };
    use splitee_messages::{
        encryption::{stateless::DecryptionShare, CiphertextStateless},
        Party,
    };

    pub struct ThresholdEncryption {
        validity_sig_pub_keys: Vec<PublicKey>,
        global_pub_key: PublicKey,
        group_generator_1: ECPoint,
        group_generator_2: ECPoint,
        own_party_id: Party,
        valid_decryption_shares: Vec<DecryptionShare>,
        lagrange: LagrangeCoefficients,
    }

    impl ThresholdEncryption {
        pub fn encrypt(&self, msg_to_encrypt: Box<[u8]>) -> (CiphertextStateless, ZKPProofPair) {
            let r = FieldElement::rand();

            let temp1 = ECPoint::scalar_multiply(&r, &self.global_pub_key.to_ecpoint());
            //let temp2 = hash(temp1.as_ref()).into();
            //let temp3 = msg_to_encrypt.to_bignum();
            let c = todo!(); //BigNumber::xor(&temp2, &temp3);

            let u = ECPoint::scalar_multiply(&r, &self.group_generator_1);

            let (msg_zkp, u_strich) =
                create_zkp(&self.group_generator_1, &self.group_generator_2, &r);

            (CiphertextStateless::new_ciphertext(c, u, u_strich), msg_zkp)
        }

        pub fn verify_decryption_share(&self, dec_share_to_verify: DecryptionShare) -> bool {
            let sender_id = dec_share_to_verify.get_sender().to_usize();
            let ciphertext = dec_share_to_verify.get_ciphertext();
            let message = hash(
                [
                    ciphertext.0.as_ref(),
                    ciphertext.1.as_ref(),
                    dec_share_to_verify.get_glob_decryption_share().as_ref(),
                ]
                .concat()
                .as_slice(),
            ); // TODO machen
            let sig_to_verify = dec_share_to_verify.get_validity_sig();

            sig_to_verify.verify(&message, &self.validity_sig_pub_keys[sender_id])
        }

        pub fn combine_global_decryption(&self) -> Box<[u8]> {
            let mut result: ECPoint = ECPoint::get_point_at_infinity();
            let ciphertext = self.valid_decryption_shares[0].get_ciphertext();

            // TODO Extract Interpolation Set from valid CoinShares
            let sender_ids: Vec<FieldElement> = self
                .valid_decryption_shares
                .iter()
                .map(|s| s.get_sender().to_fieldelement())
                .collect();
            let lagrange_coefficients = self.lagrange.calculate_lagrange_coefficients(sender_ids);

            for i in 0..self.valid_decryption_shares.len() {
                let result_share_temp: ECPoint =
                    self.valid_decryption_shares[i].get_glob_decryption_share();
                let temp: ECPoint =
                    ECPoint::scalar_multiply(&lagrange_coefficients[i], &result_share_temp);
                result = ECPoint::point_addition(&result, &temp);
            }

            let hash: FieldElement = hash(result.as_ref()).into();
            //let cleartext = BigNumber::xor(&hash, &ciphertext.0.get_c());

            //Message::from_bignum(cleartext)
            todo!()
        }
    }
}

pub mod shoup_gennaro {
    use splitee_crypto::{
        chaum_pedersen::{create_zkp, verify_zkp, ZKPProofPair},
        hash::hash,
        math::{ECPoint, FieldElement, PrivateKeyShare, PublicKey},
        util::LagrangeCoefficients,
    };
    use splitee_messages::{
        encryption::{shoup_gennaro::DecryptionShare, CiphertextStateless},
        Party,
    };

    pub struct ThresholdEncryption {
        global_pub_key: PublicKey,
        global_secret_key_share: PrivateKeyShare,
        verification_keys: Vec<PublicKey>,
        group_generator_1: ECPoint,
        group_generator_2: ECPoint,
        own_party_id: Party,
        valid_decryption_shares: Vec<DecryptionShare>,
        lagrange: LagrangeCoefficients,
    }

    impl ThresholdEncryption {
        pub fn encrypt(&self, msg_to_encrypt: Box<u8>) -> (CiphertextStateless, ZKPProofPair) {
            let r = FieldElement::rand();

            let temp1 = ECPoint::scalar_multiply(&r, &self.global_pub_key.to_ecpoint());
            // let temp2 = hash(temp1.as_ref()).into();
            let c = todo!(); // FieldElement::xor(&temp2, &temp3);

            let u = ECPoint::scalar_multiply(&r, &self.group_generator_1);

            let (msg_zkp, u_strich) =
                create_zkp(&self.group_generator_1, &self.group_generator_2, &r);

            (CiphertextStateless::new_ciphertext(c, u, u_strich), msg_zkp)
        }

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
                ECPoint::scalar_multiply(&self.global_secret_key_share.to_fieldelement(), &u);
            let validity_zkp = create_zkp(
                &u,
                &self.group_generator_1,
                &self.global_secret_key_share.to_fieldelement(),
            );

            DecryptionShare::new_share(
                self.own_party_id,
                ciphertext,
                global_dec_share,
                validity_zkp.0,
            )
        }

        pub fn verify_decryption_share(&self, dec_share_to_verify: DecryptionShare) -> bool {
            let current_ciphertext = dec_share_to_verify.get_ciphertext();

            let u = current_ciphertext.0.get_u();
            let u_strich = current_ciphertext.0.get_u_strich();
            let encrypt_zkp = current_ciphertext.1;

            let zkp_encrypt_validity = verify_zkp(
                &self.group_generator_1,
                &self.group_generator_2,
                &u,
                &u_strich,
                &encrypt_zkp,
            );

            if (!zkp_encrypt_validity) {
                // TODO Invalides Share definieren und ausgeben
                print!("?");
                return false;
            }

            let global_dec_share = dec_share_to_verify.get_glob_decryption_share();
            let share_zkp = dec_share_to_verify.get_zkp();
            let sender_id = dec_share_to_verify.get_sender().to_usize();

            verify_zkp(
                &u,
                &self.group_generator_1,
                &global_dec_share,
                &self.verification_keys[sender_id].to_ecpoint(),
                &share_zkp,
            )
        }

        pub fn combine_global_decryption(&self) -> Box<[u8]> {
            let mut result: ECPoint = ECPoint::get_point_at_infinity();
            let ciphertext = self.valid_decryption_shares[0].get_ciphertext();

            // TODO Extract Interpolation Set from valid CoinShares
            let sender_ids: Vec<FieldElement> = self
                .valid_decryption_shares
                .iter()
                .map(|s| s.get_sender().to_fieldelement())
                .collect();
            let lagrange_coefficients = self.lagrange.calculate_lagrange_coefficients(sender_ids);

            for i in 0..self.valid_decryption_shares.len() {
                let result_share_temp: ECPoint =
                    self.valid_decryption_shares[i].get_glob_decryption_share();
                let temp: ECPoint =
                    ECPoint::scalar_multiply(&lagrange_coefficients[i], &result_share_temp);
                result = ECPoint::point_addition(&result, &temp);
            }

            //let hash = hash(result.as_ref()).into();
            //let cleartext = BigNumber::xor(&hash, &ciphertext.0.get_c());

            //Message::from_bignum(cleartext)
            todo!()
        }
    }
}
