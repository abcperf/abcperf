use splitee_crypto::math::FieldElement;

use crate::Party;

pub struct SecretShare {
    s_i: FieldElement,
    s_strich_i: FieldElement,
    C: Vec<FieldElement>,
    sender: Party,
}

impl SecretShare {
    pub fn new(
        s_i: FieldElement,
        s_strich_i: FieldElement,
        C: Vec<FieldElement>,
        sender: Party,
    ) -> Self {
        Self {
            s_i: (s_i),
            s_strich_i: (s_strich_i),
            C: (C),
            sender: (sender),
        }
    }

    pub fn get_s_j(&self) -> FieldElement {
        unimplemented!()
    }

    pub fn get_s_strich_j(&self) -> FieldElement {
        unimplemented!()
    }

    pub fn get_C(&self) -> Vec<FieldElement> {
        unimplemented!()
    }

    pub fn get_sender(&self) -> Party {
        unimplemented!()
    }
}

pub struct FeldmannShare {
    A: Vec<FieldElement>,
    sender: Party,
}

impl FeldmannShare {
    pub fn new(A: Vec<FieldElement>, sender: Party) -> Self {
        Self { A, sender }
    }

    pub fn get_A(&self) -> Vec<FieldElement> {
        todo!()
    }

    pub fn get_sender(&self) -> Party {
        todo!()
    }
}

pub struct Complaint {
    accuser: Party,
    accused: Party,
}

impl Complaint {
    pub fn new(accuser: Party, accused: Party) -> Self {
        Self { accuser, accused }
    }

    pub fn get_accuser(&self) -> Party {
        todo!()
    }

    pub fn get_accused(&self) -> Party {
        todo!()
    }
}

pub struct ComplaintAnswer {
    accuser: Party,
    accused: Party,
    s_i: FieldElement,
    s_prime_i: FieldElement,
}

impl ComplaintAnswer {
    pub fn new(accuser: Party, accused: Party, s_i: FieldElement, s_prime_i: FieldElement) -> Self {
        Self {
            accuser,
            accused,
            s_i,
            s_prime_i,
        }
    }

    pub fn get_accuser(&self) -> Party {
        todo!()
    }

    pub fn get_accused(&self) -> Party {
        todo!()
    }

    pub fn get_s_i(&self) -> FieldElement {
        todo!()
    }

    pub fn get_s_prime_i(&self) -> FieldElement {
        todo!()
    }
}
