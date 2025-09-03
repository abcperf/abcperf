use std::{collections::HashMap, hash::Hash};

use splitee_crypto::math::{FieldElement, Polynomial, PrivateKeyShare, PublicKey};
use splitee_messages::{
    dkg::{Complaint, ComplaintAnswer, FeldmannShare, SecretShare},
    Party,
};

pub struct GennaroKeyGen {
    f: Polynomial,
    f_strich: Polynomial,
    p: FieldElement,
    q: FieldElement,
    generator_g: FieldElement,
    generator_h: FieldElement,
    recon_thresh: u16,
    n: u16,
    own_party_id: Party,
    own_secret_shares: Vec<SecretShare>,
}

impl GennaroKeyGen {
    pub fn create_secret_shares(&mut self) -> Vec<SecretShare> {
        //Generate Random Polynomials
        self.f = Polynomial::rand(0);
        self.f_strich = Polynomial::rand(0);

        //Generate commitments C_ik to the sharing polynomials
        let mut C: Vec<FieldElement> = Vec::new();

        for k in 0..self.recon_thresh {
            let a_k = self.f.get_coefficient(k.into());
            let b_k = self.f_strich.get_coefficient(k.into());
            let g_exp_a_k = FieldElement::exp(&self.generator_g, &a_k);
            let h_exp_b_k = FieldElement::exp(&self.generator_h, &b_k);
            let C_k = FieldElement::mul_explicit_mod(&g_exp_a_k, &h_exp_b_k, &self.p);

            C.push(C_k);
        }

        //Generate Secret Shares by evaluating polynomials at index j for party j
        let mut secret_shares: Vec<SecretShare> = Vec::new();

        for j in 1..self.n {
            let f_j = self.f.evaluate(j);
            let f_strich_j = self.f_strich.evaluate(j);

            let sec_share_j = SecretShare::new(f_j, f_strich_j, C.clone(), self.own_party_id);
            secret_shares.push(sec_share_j);
        }

        secret_shares
    }

    pub fn verify_secret_share(&self, share_to_verify: SecretShare) -> bool {
        // Unpack Secret Share
        let s_j = share_to_verify.get_s_j();
        let s_strich_j = share_to_verify.get_s_strich_j();
        let C = share_to_verify.get_C();
        let j = share_to_verify.get_sender();

        // Calculate left equation
        let g_exp_s_j = FieldElement::exp(&self.generator_g, &s_j);
        let h_exp_s_strich_j = FieldElement::exp(&self.generator_h, &s_strich_j);
        let left_equation = FieldElement::mul_explicit_mod(&g_exp_s_j, &h_exp_s_strich_j, &self.p);

        //Calculate right equation
        let mut right_equation = FieldElement::one();

        for k in 0..self.recon_thresh {
            let k_usize = k as usize;
            let j_k = FieldElement::exp(
                &FieldElement::from_usize(j.to_usize()),
                &FieldElement::from_usize(k as usize),
            );
            let C_k_j = FieldElement::exp(&C[k_usize], &j_k);
            right_equation = FieldElement::mul_explicit_mod(&right_equation, &C_k_j, &self.p);
        }

        //If left equation equals rigth equation, the share is valid
        FieldElement::equals(&left_equation, &right_equation)
    }

    pub fn combine_secret_key(&self, qual: Vec<SecretShare>) -> PrivateKeyShare {
        let mut secret_key_share = FieldElement::zero();

        for share in qual {
            let s_j = share.get_s_j();
            secret_key_share = FieldElement::add(&secret_key_share, &s_j);
        }

        PrivateKeyShare::from_fieldelement(secret_key_share)
    }

    pub fn calculate_public_key_share(&self) -> FeldmannShare {
        //Generate commitments C_ik to the sharing polynomials
        let mut feldmann_commitment: Vec<FieldElement> = Vec::new();

        for k in 0..self.recon_thresh {
            let a_k = self.f.get_coefficient(k.into());
            let g_exp_a_k = FieldElement::exp(&self.generator_g, &a_k);

            feldmann_commitment.push(g_exp_a_k);
        }

        FeldmannShare::new(feldmann_commitment, self.own_party_id)
    }

    pub fn verify_public_key_share(
        &self,
        secret_share_of_sender: SecretShare,
        share_to_verify: FeldmannShare,
    ) -> bool {
        // Unpack Secret Share of Sender
        let s_j = secret_share_of_sender.get_s_j();
        let j = secret_share_of_sender.get_sender();

        //Calculate left equation
        let left_equation = FieldElement::exp(&self.generator_g, &s_j);

        //Calculate right equation
        let mut right_equation = FieldElement::one();

        for k in 0..self.recon_thresh {
            let k_usize = k as usize;
            let j_k = FieldElement::exp(
                &FieldElement::from_usize(j.to_usize()),
                &FieldElement::from_usize(k as usize),
            );
            right_equation = FieldElement::exp(&share_to_verify.get_A()[k_usize], &j_k);
        }

        //If left equation equals rigth equation, the share is valid
        FieldElement::equals(&left_equation, &right_equation)
    }

    pub fn combine_public_key_share(
        &self,
        qual: Vec<Party>,
        y_i: HashMap<Party, FieldElement>,
    ) -> (PublicKey, HashMap<Party, PublicKey>) {
        let mut public_key = FieldElement::one();
        let mut verification_keys: HashMap<Party, PublicKey> = HashMap::new();

        for party in qual {
            let share = y_i[&party];
            verification_keys.insert(party, PublicKey::from_fieldelement(share));
            public_key = FieldElement::mul(&public_key, &share);
        }

        (PublicKey::from_fieldelement(public_key), verification_keys)
    }

    pub fn execute_dkg(
        &self,
        t: u16,
        n: u16,
        own_party_id: Party,
    ) -> (PrivateKeyShare, PublicKey, HashMap<Party, PublicKey>) {
        // Initialisierungswerte für DKG-Werte
        let f = Polynomial::rand(0);
        let f_prime = Polynomial::rand(0);

        let dkg_p = FieldElement::zero();
        let dkg_q = FieldElement::zero();

        let dkg_gen_g = FieldElement::zero();
        let dkg_gen_h = FieldElement::zero();

        let dkg = GennaroKeyGen {
            f,
            f_strich: f_prime,
            p: dkg_p,
            q: dkg_q,
            generator_g: dkg_gen_g,
            generator_h: dkg_gen_h,
            recon_thresh: t,
            n,
            own_party_id,
            own_secret_shares: todo!(),
        };

        // Generate own Shares for DKG and distribute them
        let shares = dkg.create_secret_shares();

        //for i in 1..n{
        //network.send(i,shares[i]);
        //}

        // Verify Secret Shares from other Parties
        let mut shares_received_from_others = n - 1;
        let mut received_shares: HashMap<Party, SecretShare> = HashMap::new();

        #[allow(clippy::while_immutable_condition)] // TODO remove
        while shares_received_from_others != 0 {
            //network.receive_share();
            let received_share: SecretShare = todo!();
            //SecretShare::new(s_i, s_strich_i, C, sender);

            received_shares.insert(received_share.get_sender(), received_share);
            let share_validity = dkg.verify_secret_share(received_share);

            if !share_validity {
                let complaint = Complaint::new(own_party_id, received_share.get_sender());
                //network.complain(received_share.sender_id);
            }
        }

        let mut qual_set = Vec::new();
        let mut complaint_counts: HashMap<Party, u16> = HashMap::new();

        for party in 1..n {
            qual_set.push(Party::from_u16(party));
            complaint_counts.insert(Party::from_u16(party), 0);
        }

        // Broadcast values for parties that complained
        //let complaints = network.complaints_received();
        let mut complaint_answers = Vec::new();
        let complaints: Vec<Complaint> = Vec::new();

        for complaint in complaints {
            let accused = complaint.get_accused();

            // Create ComplaintAnswer if accused
            if accused == own_party_id {
                let accuser = complaint.get_accuser();
                let accuser_usize = accuser.to_usize();

                let s_accuser = dkg.own_secret_shares[accuser_usize].get_s_j();
                let s_prime_accuser = dkg.own_secret_shares[accuser_usize].get_s_strich_j();

                let complaint_answer =
                    ComplaintAnswer::new(accuser, accused, s_accuser, s_prime_accuser);
                complaint_answers.push(complaint_answer);
            }

            // Count received complaints for all other parties
            complaint_counts[&accused] += 1;
        }

        // Entfernt alle Parteien gegen die mehr als t complaints vorliegen aus dem Qual-Set
        qual_set.retain(|party| match complaint_counts.get(party) {
            Some(&count) => count <= t,
            None => true,
        });

        // Abarbeiten von Complaint Answers
        for complaint_answer in complaint_answers {
            let accused = complaint_answer.get_accused();
            let secret_share_with_own_c = SecretShare::new(
                complaint_answer.get_s_i(),
                complaint_answer.get_s_prime_i(),
                received_shares[&accused].get_C(),
                complaint_answer.get_accuser(),
            );

            let validity = dkg.verify_secret_share(secret_share_with_own_c);

            //Entfernt accussed wenn Werte nicht gültig sind
            if !validity {
                qual_set.retain(|&x| x != accused);
            }
        }

        let mut qual_share_set = Vec::new();

        for party in qual_set {
            qual_share_set.push(received_shares[&party]);
        }

        let final_private_key_share = dkg.combine_secret_key(qual_share_set);

        //-----Zweiter Teil------

        //4(a)
        if qual_set.contains(&own_party_id) {
            let feldmann_shares = dkg.calculate_public_key_share();
            //network.broadcast(feldmann_shares);
        }

        //4(b)
        let mut feldmann_shares_received_from_others = qual_set.len();
        let mut received_A_s: HashMap<Party, Vec<FieldElement>> = HashMap::new();

        #[allow(clippy::while_immutable_condition)] // TODO remove
        while feldmann_shares_received_from_others != 0 {
            //network.receive_share();
            //let received_feldmann_share = FeldmannShare::new(s_i, s_strich_i, C, sender);
            let received_feldmann_share: FeldmannShare = todo!();

            received_A_s.insert(
                received_feldmann_share.get_sender(),
                received_feldmann_share.get_A(),
            );
            let share_validity = dkg.verify_public_key_share(
                received_shares[&received_feldmann_share.get_sender()],
                received_feldmann_share,
            );

            if !share_validity {
                let sender = received_feldmann_share.get_sender();
                let complaint = ComplaintAnswer::new(
                    own_party_id,
                    sender,
                    received_shares[&sender].get_s_j(),
                    received_shares[&sender].get_s_strich_j(),
                );
                //network.broadcast(received_share.sender_id);
            }
        }

        //4(c)
        let feldman_complaints: Vec<ComplaintAnswer> = Vec::new();

        for complaint in feldman_complaints {
            // Erst (4)
            let complaint_s = complaint.get_s_i();
            let complaint_prime_s = complaint.get_s_prime_i();
            let sender = complaint.get_accused();

            let sec_share = SecretShare::new(
                complaint_s,
                complaint_prime_s,
                received_shares[&sender].get_C(),
                sender,
            );
            let validity_sec_share = dkg.verify_secret_share(sec_share);

            // Dann (5)
            let feldmann_share = FeldmannShare::new(received_A_s[&sender], sender);
            let validity_feldmann_share = dkg.verify_public_key_share(sec_share, feldmann_share);

            if (validity_sec_share == true && validity_feldmann_share == false) {
                //reconstruct Values
            }
        }

        let mut y_i: HashMap<Party, FieldElement> = HashMap::new();

        for party in qual_set {
            y_i.insert(party, received_A_s[&party][0]);
        }

        let (public_key, verification_keys) = dkg.combine_public_key_share(qual_set, y_i);

        (final_private_key_share, public_key, verification_keys)
    }
}
