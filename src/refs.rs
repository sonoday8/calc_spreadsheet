//! A1 addressing helpers and formula-ref analysis types.
//!
//! Reference analysis walks the shared [`crate::ast`] (single grammar with evaluation).

use std::collections::HashSet;

use crate::error::SpreadsheetError;

/// Soft cap on expanded A1 ranges (analyze + eval). Larger ranges return `#NUM!`.
pub(crate) const MAX_RANGE_CELLS: usize = 100_000;

/// Number of cells in an A1 range without allocating names.
pub(crate) fn a1_range_size(start: &str, end: &str) -> Result<usize, SpreadsheetError> {
    let (c1, r1) = parse_a1(start).ok_or(SpreadsheetError::Value)?;
    let (c2, r2) = parse_a1(end).ok_or(SpreadsheetError::Value)?;
    let (cmin, cmax) = if c1 <= c2 { (c1, c2) } else { (c2, c1) };
    let (rmin, rmax) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };
    let width = (cmax - cmin + 1) as usize;
    let height = (rmax - rmin + 1) as usize;
    let size = width.checked_mul(height).ok_or(SpreadsheetError::Num)?;
    if size > MAX_RANGE_CELLS {
        return Err(SpreadsheetError::Num);
    }
    Ok(size)
}

/// Visit each cell in an A1 range without allocating the full name list.
pub(crate) fn for_each_a1_range(
    start: &str,
    end: &str,
    mut visit: impl FnMut(String) -> Result<(), SpreadsheetError>,
) -> Result<(), SpreadsheetError> {
    let _size = a1_range_size(start, end)?;
    let (c1, r1) = parse_a1(start).ok_or(SpreadsheetError::Value)?;
    let (c2, r2) = parse_a1(end).ok_or(SpreadsheetError::Value)?;
    let (cmin, cmax) = if c1 <= c2 { (c1, c2) } else { (c2, c1) };
    let (rmin, rmax) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };
    for c in cmin..=cmax {
        for r in rmin..=rmax {
            visit(format_a1(c, r))?;
        }
    }
    Ok(())
}

/// Unique cells (for deps) plus occurrence/expanded work units (for adaptive rayon).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct FormulaRefs {
    pub cells: HashSet<String>,
    pub work: usize,
}

/// Excel-style `A1` → (column 1-based, row 1-based). Non-A1 identifiers return `None`.
pub(crate) fn parse_a1(name: &str) -> Option<(u32, u32)> {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        i += 1;
    }
    if i == 0 || i == bytes.len() {
        return None;
    }
    let col_part = &name[..i];
    let row_part = &name[i..];
    if !row_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if row_part.starts_with('0') {
        return None;
    }
    let row: u32 = row_part.parse().ok()?;
    if row == 0 {
        return None;
    }
    let mut col: u32 = 0;
    for ch in col_part.chars() {
        let v = (ch.to_ascii_uppercase() as u8 - b'A') as u32 + 1;
        col = col.checked_mul(26)?.checked_add(v)?;
    }
    Some((col, row))
}

pub(crate) fn format_a1(col: u32, row: u32) -> String {
    let mut n = col;
    let mut letters = Vec::new();
    while n > 0 {
        n -= 1;
        letters.push((b'A' + (n % 26) as u8) as char);
        n /= 26;
    }
    letters.reverse();
    let mut out: String = letters.into_iter().collect();
    out.push_str(&row.to_string());
    out
}

/// Canonical A1 spelling (uppercase letters). Non-A1 names are returned unchanged.
pub(crate) fn canonical_cell_name(name: &str) -> String {
    if let Some((col, row)) = parse_a1(name) {
        format_a1(col, row)
    } else {
        name.to_string()
    }
}

pub(crate) fn is_cell_name_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

pub(crate) fn is_cell_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format_a1_round_trip() {
        assert_eq!(parse_a1("A1"), Some((1, 1)));
        assert_eq!(parse_a1("AA10"), Some((27, 10)));
        assert_eq!(format_a1(1, 1), "A1");
        assert_eq!(format_a1(27, 10), "AA10");
        assert_eq!(parse_a1("L0_0"), None);
    }

    #[test]
    fn for_each_visits_without_full_vec_semantics() {
        let mut n = 0usize;
        for_each_a1_range("A1", "A3", |_| {
            n += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(n, 3);
        assert_eq!(a1_range_size("A1", "A3").unwrap(), 3);
    }

    #[test]
    fn rejects_oversized_ranges() {
        let err = a1_range_size("A1", "ZZ9000").unwrap_err();
        assert_eq!(err, SpreadsheetError::Num);
    }
}
