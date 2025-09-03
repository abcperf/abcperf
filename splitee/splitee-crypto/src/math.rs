use curve25519_dalek::{EdwardsPoint, Scalar};
use group::ff::Field;
use sha2::{digest::consts::U64, Digest};

use crate::hash::HashValue;

#[derive(Clone, Copy)]
pub struct FieldElement {
    inner: Scalar,
}

impl From<HashValue> for FieldElement {
    fn from(hash: HashValue) -> Self {
        Self {
            inner: Scalar::hash_from_bytes::<sha2::Sha512>(hash.as_ref()),
        }
    }
}

impl AsRef<[u8]> for FieldElement {
    fn as_ref(&self) -> &[u8] {
        self.inner.as_bytes()
    }
}

impl FieldElement {
    pub fn exp(base: &FieldElement, exponent: &FieldElement) -> FieldElement {
        let exponent = exponent.inner.to_bytes();
        let exponent1 = u64::from_le_bytes(exponent[0..8].try_into().unwrap());
        let exponent2 = u64::from_le_bytes(exponent[8..16].try_into().unwrap());
        let exponent3 = u64::from_le_bytes(exponent[16..24].try_into().unwrap());
        let exponent4 = u64::from_le_bytes(exponent[24..32].try_into().unwrap());
        let exponent = [exponent1, exponent2, exponent3, exponent4];
        Self {
            inner: base.inner.pow(exponent),
        }
    }

    pub fn mul(factor_1: &FieldElement, factor_2: &FieldElement) -> FieldElement {
        Self {
            inner: factor_1.inner * factor_2.inner,
        }
    }

    pub fn mul_explicit_mod(
        factor_1: &FieldElement,
        factor_2: &FieldElement,
        modulus: &FieldElement,
    ) -> FieldElement {
        FieldElement {
            inner: factor_1.inner * factor_2.inner * modulus.inner.invert(),
        }
    }

    pub fn add(summand_1: &FieldElement, summand_2: &FieldElement) -> FieldElement {
        Self {
            inner: summand_1.inner + summand_2.inner,
        }
    }

    pub fn sub(input_1: &FieldElement, input_2: &FieldElement) -> FieldElement {
        Self {
            inner: input_1.inner - input_2.inner,
        }
    }

    pub const fn zero() -> FieldElement {
        Self {
            inner: Scalar::ZERO,
        }
    }

    pub const fn one() -> FieldElement {
        Self { inner: Scalar::ONE }
    }

    pub fn equals(input_1: &FieldElement, input_2: &FieldElement) -> bool {
        input_1.inner == input_2.inner
    }

    pub fn inv(input: &FieldElement) -> FieldElement {
        Self {
            inner: input.inner.invert(),
        }
    }

    pub fn from_u8(input: u8) -> FieldElement {
        Self {
            inner: Scalar::from(input),
        }
    }

    pub fn from_u16(input: u16) -> FieldElement {
        Self {
            inner: Scalar::from(input),
        }
    }

    pub fn from_usize(input: usize) -> FieldElement {
        Self {
            inner: Scalar::from(input as u64),
        }
    }

    pub fn rand() -> FieldElement {
        let mut rng = rand::thread_rng();
        Self {
            inner: Scalar::random(&mut rng),
        }
    }

    pub fn from_hasher<D: Digest<OutputSize = U64>>(input: D) -> Self {
        Self {
            inner: Scalar::from_hash(input),
        }
    }
}

impl From<u8> for FieldElement {
    fn from(input: u8) -> Self {
        Self {
            inner: Scalar::from(input),
        }
    }
}

impl From<u16> for FieldElement {
    fn from(input: u16) -> Self {
        Self {
            inner: Scalar::from(input),
        }
    }
}

impl From<u32> for FieldElement {
    fn from(input: u32) -> Self {
        Self {
            inner: Scalar::from(input),
        }
    }
}

impl From<u64> for FieldElement {
    fn from(input: u64) -> Self {
        Self {
            inner: Scalar::from(input),
        }
    }
}

impl From<u128> for FieldElement {
    fn from(input: u128) -> Self {
        Self {
            inner: Scalar::from(input),
        }
    }
}

impl From<usize> for FieldElement {
    fn from(input: usize) -> Self {
        Self::from(input as u64)
    }
}

pub struct ECPoint {
    point: EdwardsPoint,
}

impl AsRef<[u8]> for ECPoint {
    fn as_ref(&self) -> &[u8] {
        unimplemented!()
    }
}

impl ECPoint {
    pub fn get_point_at_infinity() -> ECPoint {
        ECPoint {
            point: EdwardsPoint::default(),
        }
    }

    pub fn scalar_multiply(scalar: &FieldElement, point: &ECPoint) -> ECPoint {
        ECPoint {
            point: point.point * scalar.inner,
        }
    }

    pub fn point_addition(point_1: &ECPoint, point_2: &ECPoint) -> ECPoint {
        ECPoint {
            point: point_1.point + point_2.point,
        }
    }

    pub fn hash_to_curve(input: &[u8]) -> ECPoint {
        ECPoint {
            point: EdwardsPoint::hash_to_curve::<sha2::Sha512>(&[input], &[]),
        }
    }
}

pub struct Polynomial {
    coefficients: Vec<FieldElement>,
}

impl Polynomial {
    pub fn get_coefficient(&self, index: usize) -> FieldElement {
        self.coefficients[index]
    }

    pub fn evaluate(&self, x: impl Into<FieldElement>) -> FieldElement {
        let x = x.into();
        self.coefficients
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let x_pow = FieldElement::exp(&x, &FieldElement::from_usize(i));
                FieldElement::mul(c, &x_pow)
            })
            .fold(FieldElement::zero(), |acc, val| {
                FieldElement::add(&acc, &val)
            })
    }

    pub fn rand(order: usize) -> Polynomial {
        Polynomial {
            coefficients: (0..order).map(|_| FieldElement::rand()).collect(),
        }
    }
}

pub struct PrivateKey {
    pub(crate) inner: ed25519_dalek::SigningKey,
}

impl PrivateKey {
    pub fn to_bignum(&self) -> FieldElement {
        FieldElement {
            inner: self.inner.to_scalar(),
        }
    }
}

pub struct PublicKey {
    pub(crate) inner: ed25519_dalek::VerifyingKey,
}

impl PublicKey {
    pub fn to_ecpoint(&self) -> ECPoint {
        ECPoint {
            point: self.inner.to_edwards(),
        }
    }

    pub const fn from_fieldelement(field_el: FieldElement) -> PublicKey {
        unimplemented!()
    }
}

pub struct PrivateKeyShare {
    pub(crate) inner: FieldElement,
}

impl PrivateKeyShare {
    pub fn to_fieldelement(&self) -> FieldElement {
        self.inner
    }

    pub const fn from_fieldelement(input: FieldElement) -> PrivateKeyShare {
        PrivateKeyShare { inner: input }
    }
}
