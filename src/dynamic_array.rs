//! Dynamic array functions: SEQUENCE, UNIQUE, SORT, FILTER.

use crate::ast::EvalValue;
use crate::error::SpreadsheetError;
use crate::functions::is_truthy;
use crate::refs::MAX_RANGE_CELLS;

pub(crate) fn value_to_matrix(value: EvalValue) -> Result<Vec<Vec<f64>>, SpreadsheetError> {
    match value {
        EvalValue::Number(n) => Ok(vec![vec![n]]),
        EvalValue::Array(rows) => Ok(rows),
        EvalValue::Text(_) => Err(SpreadsheetError::Value),
    }
}

pub(crate) fn matrix_to_value(rows: Vec<Vec<f64>>) -> Result<EvalValue, SpreadsheetError> {
    if rows.is_empty() || rows.first().is_some_and(|r| r.is_empty()) {
        return Err(SpreadsheetError::Calc);
    }
    let width = rows[0].len();
    if width == 0 || rows.iter().any(|r| r.len() != width) {
        return Err(SpreadsheetError::Value);
    }
    let size = rows.len().checked_mul(width).ok_or(SpreadsheetError::Num)?;
    if size > MAX_RANGE_CELLS {
        return Err(SpreadsheetError::Num);
    }
    if rows.len() == 1 && width == 1 {
        Ok(EvalValue::Number(rows[0][0]))
    } else {
        Ok(EvalValue::Array(rows))
    }
}

/// `SEQUENCE(rows, [columns], [start], [step])` — fills row-major.
pub(crate) fn sequence(
    rows: f64,
    columns: f64,
    start: f64,
    step: f64,
) -> Result<EvalValue, SpreadsheetError> {
    if !rows.is_finite() || !columns.is_finite() || !start.is_finite() || !step.is_finite() {
        return Err(SpreadsheetError::Num);
    }
    let rows = rows.trunc() as i64;
    let columns = columns.trunc() as i64;
    if rows <= 0 || columns <= 0 {
        return Err(SpreadsheetError::Calc);
    }
    let rows = rows as usize;
    let columns = columns as usize;
    let size = rows.checked_mul(columns).ok_or(SpreadsheetError::Num)?;
    if size > MAX_RANGE_CELLS {
        return Err(SpreadsheetError::Num);
    }

    let mut out = Vec::with_capacity(rows);
    let mut current = start;
    for _ in 0..rows {
        let mut row = Vec::with_capacity(columns);
        for _ in 0..columns {
            row.push(current);
            current += step;
        }
        out.push(row);
    }
    matrix_to_value(out)
}

/// `UNIQUE(array, [by_col], [exactly_once])`
pub(crate) fn unique(
    array: EvalValue,
    by_col: bool,
    exactly_once: bool,
) -> Result<EvalValue, SpreadsheetError> {
    let mut rows = value_to_matrix(array)?;
    if by_col {
        rows = transpose(rows)?;
    }

    let result = if exactly_once {
        let mut counts: Vec<(Vec<f64>, usize)> = Vec::new();
        for row in &rows {
            if let Some((_, count)) = counts.iter_mut().find(|(r, _)| r == row) {
                *count += 1;
            } else {
                counts.push((row.clone(), 1));
            }
        }
        counts
            .into_iter()
            .filter(|(_, c)| *c == 1)
            .map(|(r, _)| r)
            .collect::<Vec<_>>()
    } else {
        let mut seen = Vec::new();
        let mut out = Vec::new();
        for row in rows {
            if !seen.iter().any(|s: &Vec<f64>| s == &row) {
                seen.push(row.clone());
                out.push(row);
            }
        }
        out
    };

    let result = if by_col { transpose(result)? } else { result };
    if result.is_empty() {
        return Err(SpreadsheetError::Calc);
    }
    matrix_to_value(result)
}

/// `SORT(array, [sort_index], [sort_order], [by_col])`
pub(crate) fn sort(
    array: EvalValue,
    sort_index: f64,
    sort_order: f64,
    by_col: bool,
) -> Result<EvalValue, SpreadsheetError> {
    let mut rows = value_to_matrix(array)?;
    if rows.is_empty() {
        return Err(SpreadsheetError::Calc);
    }
    let width = rows[0].len();
    let height = rows.len();

    let index = sort_index.trunc() as i64;
    if index < 1 {
        return Err(SpreadsheetError::Value);
    }
    let index = (index - 1) as usize;

    let ascending = match sort_order.trunc() as i64 {
        1 => true,
        -1 => false,
        _ => return Err(SpreadsheetError::Value),
    };

    if by_col {
        if index >= height {
            return Err(SpreadsheetError::Value);
        }
        rows = transpose(rows)?;
        // After transpose, sorting "columns" means sorting rows by column `index`
        // where index was a row index in the original — the key is column `index` in transposed.
        sort_rows_by_column(&mut rows, index, ascending);
        rows = transpose(rows)?;
    } else {
        if index >= width {
            return Err(SpreadsheetError::Value);
        }
        sort_rows_by_column(&mut rows, index, ascending);
    }

    matrix_to_value(rows)
}

fn sort_rows_by_column(rows: &mut [Vec<f64>], col: usize, ascending: bool) {
    rows.sort_by(|a, b| {
        let cmp = a[col].total_cmp(&b[col]);
        if ascending {
            cmp
        } else {
            cmp.reverse()
        }
    });
}

/// `FILTER(array, include, [if_empty])`
pub(crate) fn filter(
    array: EvalValue,
    include: EvalValue,
    if_empty: Option<EvalValue>,
) -> Result<EvalValue, SpreadsheetError> {
    let array = value_to_matrix(array)?;
    let include = value_to_matrix(include)?;
    let height = array.len();
    let width = array[0].len();
    let ih = include.len();
    let iw = include[0].len();

    let filtered = if ih == height && iw == 1 {
        // Filter rows.
        let mut out = Vec::new();
        for (i, row) in array.into_iter().enumerate() {
            if is_truthy(include[i][0]) {
                out.push(row);
            }
        }
        out
    } else if ih == 1 && iw == width {
        // Filter columns.
        let keep: Vec<bool> = (0..width).map(|c| is_truthy(include[0][c])).collect();
        if keep.iter().all(|k| !k) {
            Vec::new()
        } else {
            array
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .enumerate()
                        .filter_map(|(c, v)| keep[c].then_some(v))
                        .collect()
                })
                .collect()
        }
    } else if ih == height && iw == width && width == 1 {
        // Column vector include matching a column array.
        let mut out = Vec::new();
        for (i, row) in array.into_iter().enumerate() {
            if is_truthy(include[i][0]) {
                out.push(row);
            }
        }
        out
    } else {
        return Err(SpreadsheetError::Value);
    };

    if filtered.is_empty() || filtered.first().is_some_and(|r| r.is_empty()) {
        return match if_empty {
            Some(v) => Ok(v),
            None => Err(SpreadsheetError::Calc),
        };
    }
    matrix_to_value(filtered)
}

fn transpose(rows: Vec<Vec<f64>>) -> Result<Vec<Vec<f64>>, SpreadsheetError> {
    if rows.is_empty() {
        return Ok(rows);
    }
    let width = rows[0].len();
    if width == 0 || rows.iter().any(|r| r.len() != width) {
        return Err(SpreadsheetError::Value);
    }
    let mut out = vec![Vec::with_capacity(rows.len()); width];
    for row in rows {
        for (c, v) in row.into_iter().enumerate() {
            out[c].push(v);
        }
    }
    Ok(out)
}

/// Spill footprint when SEQUENCE args are numeric literals.
pub(crate) fn sequence_spill_shape_from_consts(
    rows: Option<f64>,
    columns: Option<f64>,
) -> Option<(usize, usize)> {
    let rows = rows?.trunc();
    let columns = columns.unwrap_or(1.0).trunc();
    if !(rows.is_finite() && columns.is_finite()) || rows <= 0.0 || columns <= 0.0 {
        return None;
    }
    let h = rows as usize;
    let w = columns as usize;
    if h.checked_mul(w).is_none_or(|s| s > MAX_RANGE_CELLS) {
        return None;
    }
    if h == 1 && w == 1 {
        None
    } else {
        Some((h, w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_fills_row_major() {
        let v = sequence(2.0, 3.0, 1.0, 1.0).unwrap();
        assert_eq!(
            value_to_matrix(v).unwrap(),
            vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]]
        );
    }

    #[test]
    fn filter_rows_keeps_truthy() {
        let array = EvalValue::Array(vec![vec![10.0], vec![20.0], vec![30.0]]);
        let include = EvalValue::Array(vec![vec![1.0], vec![0.0], vec![1.0]]);
        let v = filter(array, include, None).unwrap();
        assert_eq!(value_to_matrix(v).unwrap(), vec![vec![10.0], vec![30.0]]);
    }
}
