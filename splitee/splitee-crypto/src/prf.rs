use sha2::{Digest, Sha512};

use crate::math::FieldElement;

pub struct PRF {
    seed: FieldElement,
}

impl PRF {
    pub fn new(seed: FieldElement) -> Self {
        Self { seed }
    }

    pub fn evaluate(&self, eval_point: FieldElement) -> FieldElement {
        FieldElement::from_hasher(
            Sha512::new()
                .chain_update(self.seed.as_ref())
                .chain_update(eval_point.as_ref()),
        )
    }
}
