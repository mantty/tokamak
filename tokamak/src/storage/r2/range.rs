//! The bytes of an object a read asks for.

use serde::{Deserialize, Serialize};

use super::failure::Failure;

/// Bytes of an object from `offset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Range {
    pub(crate) offset: u64,
    pub(crate) length: u64,
}

impl Range {
    /// Every byte of a `size`-byte object.
    pub(super) const fn whole(size: u64) -> Self {
        Self {
            offset: 0,
            length: size,
        }
    }
}

/// An offset and length, or a suffix length, that a read asks for.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Bounds {
    offset: Option<u64>,
    length: Option<u64>,
    suffix: Option<u64>,
}

impl Bounds {
    /// The bytes the bounds select of a `size`-byte object, ending at its end.
    pub(super) fn resolve(&self, size: u64) -> Result<Range, Failure> {
        let (offset, length) = match self.suffix {
            Some(0) => return Err(Failure::InvalidRange),
            Some(suffix) => (size - suffix.min(size), suffix.min(size)),
            None => {
                let offset = self.offset.unwrap_or(0);
                let rest = size.checked_sub(offset).ok_or(Failure::InvalidRange)?;
                (offset, self.length.unwrap_or(rest).min(rest))
            }
        };
        if length == 0 {
            return Err(Failure::InvalidRange);
        }
        Ok(Range { offset, length })
    }
}

/// The one range a `Range` header's value selects of a `size`-byte object, or
/// none when the value selects no single satisfiable range.
pub(super) fn header_range(header: &str, size: u64) -> Option<Range> {
    let rest = header.trim_start_matches(' ');
    let (unit, rest) = (rest.get(..5)?, rest.get(5..)?);
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let rest = rest.trim_start_matches(' ').strip_prefix('=')?;
    let mut ranges = Vec::new();
    for spec in rest.split(',') {
        let (start, end) = spec.split_once('-')?;
        match (bound(start)?, bound(end)?) {
            (Bound::At(start), Bound::At(end)) if start <= end && start < size => {
                ranges.push((start, end.min(size - 1)));
            }
            (Bound::At(start), Bound::Absent) if start < size => ranges.push((start, size - 1)),
            (Bound::Absent, Bound::At(0)) => {}
            (Bound::Absent, Bound::At(suffix)) if suffix < size => {
                ranges.push((size - suffix, size - 1));
            }
            _ => return None,
        }
    }
    match ranges[..] {
        [(start, end)] => Some(Range {
            offset: start,
            length: end - start + 1,
        }),
        _ => None,
    }
}

/// One end of a range in a `Range` header.
enum Bound {
    Absent,
    At(u64),
}

/// The end of a range that `text`, surrounded by spaces, gives, or none when
/// it is not a number.
fn bound(text: &str) -> Option<Bound> {
    let digits = text.trim_matches(' ');
    if digits.is_empty() {
        return Some(Bound::Absent);
    }
    digits
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| Bound::At(digits.parse().unwrap_or(u64::MAX)))
}

#[cfg(test)]
mod tests {
    use super::{Bounds, Range, header_range};
    use crate::storage::r2::failure::Failure;

    fn bounds(offset: Option<u64>, length: Option<u64>, suffix: Option<u64>) -> Bounds {
        Bounds {
            offset,
            length,
            suffix,
        }
    }

    fn range(offset: u64, length: u64) -> Range {
        Range { offset, length }
    }

    #[test]
    fn resolves_offsets_lengths_and_suffixes_within_the_object() {
        assert_eq!(bounds(Some(2), None, None).resolve(10), Ok(range(2, 8)));
        assert_eq!(bounds(Some(8), Some(10), None).resolve(10), Ok(range(8, 2)));
        assert_eq!(bounds(None, Some(4), None).resolve(10), Ok(range(0, 4)));
        assert_eq!(bounds(None, None, Some(3)).resolve(10), Ok(range(7, 3)));
        assert_eq!(bounds(None, None, Some(30)).resolve(10), Ok(range(0, 10)));
        assert_eq!(bounds(None, None, None).resolve(10), Ok(range(0, 10)));
    }

    #[test]
    fn rejects_bounds_that_select_nothing() {
        for rejected in [
            bounds(None, None, Some(0)),
            bounds(Some(10), None, None),
            bounds(Some(11), None, None),
            bounds(Some(1), Some(0), None),
        ] {
            assert_eq!(rejected.resolve(10), Err(Failure::InvalidRange));
        }
        assert_eq!(
            bounds(None, None, None).resolve(0),
            Err(Failure::InvalidRange)
        );
    }

    #[test]
    fn reads_one_satisfiable_range_from_a_header() {
        assert_eq!(header_range("bytes=1-3", 10), Some(range(1, 3)));
        assert_eq!(header_range(" Bytes = 5- ", 10), Some(range(5, 5)));
        assert_eq!(header_range("bytes=-2", 10), Some(range(8, 2)));
        assert_eq!(header_range("bytes=8-20", 10), Some(range(8, 2)));
        assert_eq!(header_range("bytes=-0, 2-3", 10), Some(range(2, 2)));
        for unsatisfied in [
            "bytes=1-2,4-5",
            "bytes=20-",
            "bytes=3-1",
            "bytes=-20",
            "bytes=-0",
            "bytes=",
            "items=1-2",
            "bytes=1-2-3",
            "bytes=a-",
            "bytes",
        ] {
            assert_eq!(header_range(unsatisfied, 10), None, "{unsatisfied}");
        }
    }
}
