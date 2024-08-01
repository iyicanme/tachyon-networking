pub struct Sequence {}

impl Sequence {
    #[must_use]
    pub const fn is_greater_then(s1: u16, s2: u16) -> bool {
        ((s1 > s2) && (s1 - s2 <= 32768)) || ((s1 < s2) && (s2 - s1 > 32768))
    }

    #[must_use]
    pub const fn is_less_than(s1: u16, s2: u16) -> bool {
        Self::is_greater_then(s2, s1)
    }

    #[must_use]
    pub const fn is_equal_to_or_less_than(s1: u16, s2: u16) -> bool {
        s1 == s2 || Self::is_greater_then(s2, s1)
    }

    #[must_use]
    pub const fn next_sequence(sequence: u16) -> u16 {
        if sequence >= u16::MAX - 1 {
            0
        } else {
            sequence + 1
        }
    }

    #[must_use]
    pub const fn previous_sequence(sequence: u16) -> u16 {
        if sequence == 0 {
            u16::MAX - 1
        } else {
            sequence - 1
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::sequence::Sequence;

    #[test]
    fn test_basic() {
        assert_eq!(65534, Sequence::next_sequence(65533));
        assert_eq!(0, Sequence::next_sequence(65534));
        assert_eq!(1, Sequence::next_sequence(0));

        assert_eq!(65533, Sequence::previous_sequence(65534));
        assert_eq!(65534, Sequence::previous_sequence(0));
        assert_eq!(0, Sequence::previous_sequence(1));

        assert!(Sequence::is_greater_then(0, 65534));
    }
}
