use crate::math::FieldElement;

pub struct LagrangeCoefficients {
    positive_precomputed_inverse: Vec<FieldElement>,
    negative_precomputed_inverse: Vec<FieldElement>,
    optimized: bool,
}

impl LagrangeCoefficients {
    pub fn calculate_lagrange_coefficients(
        &self,
        party_set: Vec<FieldElement>,
    ) -> Vec<FieldElement> {
        if self.optimized {
            self.calculate_lagrange_coefficients_optimized(party_set)
        } else {
            self.calculate_lagrange_coefficients_unoptimized(party_set)
        }
    }

    pub fn calculate_lagrange_coefficients_unoptimized(
        &self,
        party_set: Vec<FieldElement>,
    ) -> Vec<FieldElement> {
        let mut output_coefficients: Vec<FieldElement> = Vec::new();

        // Loop für jede Partei der Koeffizient
        for i in 0..party_set.len() {
            // Berechnung des Koeffizienten für Partei i
            let mut party_coefficient = FieldElement::one();
            for j in 0..party_set.len() {
                // Skippe Faktor für eigene Partei
                if j == i {
                    continue;
                }

                let denom = FieldElement::sub(&party_set[i], &party_set[j]);
                let denom_inv = FieldElement::inv(&denom);
                let factor = FieldElement::mul(&party_set[j], &denom_inv);
                party_coefficient = FieldElement::mul(&party_coefficient, &factor);
            }

            output_coefficients.push(party_coefficient);
        }

        output_coefficients
    }

    pub fn calculate_lagrange_coefficients_optimized(
        &self,
        party_set: Vec<FieldElement>,
    ) -> Vec<FieldElement> {
        let mut output_coefficients: Vec<FieldElement> = Vec::new();

        // Loop für jede Partei der Koeffizient
        for i in 0..party_set.len() {
            let i_isize = i as isize;
            let mut party_coefficient = FieldElement::one();
            //Berechnung der Koeffizienten für Partei i
            for (j, s) in party_set.iter().enumerate() {
                let j_isize = j as isize;
                // Skippe Faktor für eigene Partei
                if j == i {
                    continue;
                }

                let denom = i_isize - j_isize;
                let denom_inv = if denom < 0 {
                    self.negative_precomputed_inverse[denom.unsigned_abs()]
                } else {
                    self.positive_precomputed_inverse[denom.unsigned_abs()]
                };
                let factor = FieldElement::mul(s, &denom_inv);
                party_coefficient = FieldElement::mul(&party_coefficient, &factor);
            }

            output_coefficients.push(party_coefficient);
        }

        output_coefficients
    }

    pub fn precompute_inverse(&mut self, number_of_parties: u16) {
        self.negative_precomputed_inverse.push(FieldElement::zero());
        self.positive_precomputed_inverse.push(FieldElement::zero());

        for i in 1..number_of_parties {
            // Positive i
            let i_as_field_element = FieldElement::from_u16(i);
            let i_inverse = FieldElement::inv(&i_as_field_element);
            self.positive_precomputed_inverse.push(i_inverse);

            //Negative i
            let zero_as_field_element = FieldElement::zero();
            let minus_i_as_field_element =
                FieldElement::sub(&zero_as_field_element, &i_as_field_element);
            let minus_i_inverse = FieldElement::inv(&minus_i_as_field_element);
            self.negative_precomputed_inverse.push(minus_i_inverse);
        }

        self.optimized = true;
    }
}
