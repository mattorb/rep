use super::rendered::{count_occurrences_before, newlines_before_byte, nth_occurrence};

#[test]
fn newlines_before_byte_basic() {
    assert_eq!(newlines_before_byte("a\nb\nc", 0), 0);
    assert_eq!(newlines_before_byte("a\nb\nc", 1), 0);
    assert_eq!(newlines_before_byte("a\nb\nc", 2), 1);
    assert_eq!(newlines_before_byte("a\nb\nc", 4), 2);
    assert_eq!(newlines_before_byte("a\nb\nc", 5), 2);
    assert_eq!(newlines_before_byte("abc", 999), 0);
}

#[test]
fn count_occurrences_before_empty_needle_returns_zero() {
    assert_eq!(count_occurrences_before("a b c", "", 5), 0);
}

#[test]
fn nth_occurrence_empty_needle_returns_none() {
    assert_eq!(nth_occurrence("a b c", "", 0), None);
}

#[test]
fn count_occurrences_before_basic() {
    assert_eq!(count_occurrences_before("a b a c a", "a", 0), 0);
    assert_eq!(count_occurrences_before("a b a c a", "a", 1), 1);
    assert_eq!(count_occurrences_before("a b a c a", "a", 4), 1);
    assert_eq!(count_occurrences_before("a b a c a", "a", 5), 2);
    assert_eq!(count_occurrences_before("a b a c a", "a", 9), 3);
}

#[test]
fn nth_occurrence_basic() {
    assert_eq!(nth_occurrence("a b a c a", "a", 0), Some(0));
    assert_eq!(nth_occurrence("a b a c a", "a", 1), Some(4));
    assert_eq!(nth_occurrence("a b a c a", "a", 2), Some(8));
    assert_eq!(nth_occurrence("a b a c a", "a", 3), None);
}
