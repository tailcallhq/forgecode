use std::ops::{Deref, RangeInclusive};

/// Collects references to the inner values of a slice of `Deref`-able wrappers.
///
/// Useful for converting a `&[Message<T>]` to a `Vec<&T>` before passing to callbacks
/// that operate on bare item references.
pub fn deref_messages<W: Deref>(messages: &[W]) -> Vec<&W::Target> {
    messages.iter().map(|m| m.deref()).collect()
}

/// Replaces all items within `range` in `items` with the single `replacement` item.
///
/// Returns a new `Vec` containing the elements before the range, the replacement, and the
/// elements after the range. Returns `items` unchanged if the range is out of bounds.
pub fn replace_range<Item>(
    items: Vec<Item>,
    replacement: Item,
    range: RangeInclusive<usize>,
) -> Vec<Item> {
    let start = *range.start();
    let end = *range.end();

    if items.is_empty() || start >= items.len() || end >= items.len() {
        return items;
    }

    let mut result = Vec::with_capacity(items.len() - (end - start));
    let mut iter = items.into_iter();

    result.extend(iter.by_ref().take(start));
    result.push(replacement);
    iter.by_ref().nth(end - start); // skip the items covered by the range
    result.extend(iter);

    result
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::replace_range;

    #[test]
    fn test_replace_range_middle() {
        let items = vec![1, 2, 3, 4, 5];
        let actual = replace_range(items, 99, 1..=3);
        let expected = vec![1, 99, 5];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_replace_range_start() {
        let items = vec![1, 2, 3, 4, 5];
        let actual = replace_range(items, 99, 0..=2);
        let expected = vec![99, 4, 5];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_replace_range_end() {
        let items = vec![1, 2, 3, 4, 5];
        let actual = replace_range(items, 99, 3..=4);
        let expected = vec![1, 2, 3, 99];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_replace_range_single_element() {
        let items = vec![1, 2, 3];
        let actual = replace_range(items, 99, 1..=1);
        let expected = vec![1, 99, 3];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_replace_range_entire_vec() {
        let items = vec![1, 2, 3];
        let actual = replace_range(items, 99, 0..=2);
        let expected = vec![99];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_replace_range_empty_vec() {
        let items: Vec<i32> = vec![];
        let actual = replace_range(items, 99, 0..=0);
        let expected: Vec<i32> = vec![];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_replace_range_start_out_of_bounds() {
        let items = vec![1, 2, 3];
        let actual = replace_range(items, 99, 5..=6);
        let expected = vec![1, 2, 3];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_replace_range_end_out_of_bounds() {
        let items = vec![1, 2, 3];
        let actual = replace_range(items, 99, 1..=10);
        let expected = vec![1, 2, 3];
        assert_eq!(actual, expected);
    }
}
