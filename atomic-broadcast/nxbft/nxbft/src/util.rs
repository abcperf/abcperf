use crate::{Round, Wave};

pub(super) fn round(wave: Wave, round_in_wave: u64) -> Round {
    assert!(round_in_wave > 0 && round_in_wave < 5);
    Round::from(4 * (wave - 1).as_u64() + round_in_wave)
}

#[cfg(test)]
mod tests {
    use super::round;
    #[test]
    fn test_round_calc() {
        assert_eq!(round(1.into(), 1), 1.into());
        assert_eq!(round(1.into(), 2), 2.into());
        assert_eq!(round(1.into(), 3), 3.into());
        assert_eq!(round(1.into(), 4), 4.into());
        assert_eq!(round(2.into(), 1), 5.into());
        assert_eq!(round(2.into(), 2), 6.into());
        assert_eq!(round(2.into(), 3), 7.into());
        assert_eq!(round(2.into(), 4), 8.into());
    }

    #[should_panic]
    #[test]
    fn round_to_small() {
        round(1.into(), 0);
    }

    #[should_panic]
    #[test]
    fn round_to_big() {
        round(1.into(), 5);
    }
}
