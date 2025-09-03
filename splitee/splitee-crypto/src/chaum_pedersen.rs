use crate::{
    hash::hash,
    math::{ECPoint, FieldElement},
};

#[derive(Clone, Copy)]
pub struct ZKPProofPair {
    pub f: FieldElement,
    pub e: FieldElement,
}

impl ZKPProofPair {
    pub const fn new_proof(f: FieldElement, e: FieldElement) -> Self {
        Self { f, e }
    }

    pub fn get_f(&self) -> FieldElement {
        todo!()
    }

    pub fn get_e(&self) -> FieldElement {
        todo!()
    }
}

pub fn create_zkp(
    gen_1: &ECPoint,
    gen_2: &ECPoint,
    value_to_proof: &FieldElement,
) -> (ZKPProofPair, ECPoint) {
    let s = FieldElement::rand();

    let u = ECPoint::scalar_multiply(value_to_proof, gen_1);
    let w = ECPoint::scalar_multiply(&s, gen_1);

    let u_hat = ECPoint::scalar_multiply(value_to_proof, gen_2);
    let w_hat = ECPoint::scalar_multiply(&s, gen_2);

    let e: FieldElement = hash(
        [u.as_ref(), w.as_ref(), u_hat.as_ref(), w_hat.as_ref()]
            .concat()
            .as_slice(),
    )
    .into(); //TODO Trait für FieldElement implementieren

    let temp = FieldElement::mul(value_to_proof, &e);
    let f = FieldElement::add(&s, &temp);

    let return_proof = ZKPProofPair::new_proof(f, e);
    (return_proof, u_hat)
}

pub fn verify_zkp(
    gen_1: &ECPoint,
    gen_2: &ECPoint,
    u: &ECPoint,
    u_hat: &ECPoint,
    zkp: &ZKPProofPair,
) -> bool {
    let f = zkp.get_f();
    let e = zkp.get_e();

    //Reconstruct w
    let g_prime_f = ECPoint::scalar_multiply(&f, gen_1);
    let e_inv = FieldElement::inv(&e);
    let u_prime_e_inv = ECPoint::scalar_multiply(&e_inv, u);
    let w = ECPoint::point_addition(&g_prime_f, &u_prime_e_inv);

    //Reconstruct w_hat
    let g_hat_prime_f = ECPoint::scalar_multiply(&f, gen_2);
    let u_prime_e_inv = ECPoint::scalar_multiply(&e_inv, u_hat);
    let w_hat = ECPoint::point_addition(&g_hat_prime_f, &u_prime_e_inv);

    let e_recon = hash(
        [u.as_ref(), w.as_ref(), u_hat.as_ref(), w_hat.as_ref()]
            .concat()
            .as_slice(),
    )
    .into();

    FieldElement::equals(&e, &e_recon)
}

impl AsRef<[u8]> for ZKPProofPair {
    fn as_ref(&self) -> &[u8] {
        unimplemented!()
    }
}
