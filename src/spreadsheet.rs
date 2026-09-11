use std::collections::HashMap;

use crate::ast::{eval_value, Expr, EvalValue};
use crate::error::SpreadsheetError;
use crate::parser::parse_standalone_string_literal;

/// Classification used by COUNT / COUNTA (blank = missing or empty).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CellKind {
    Number,
    Text,
    Blank,
}

/// Read-only evaluation context: cached ASTs, computed numbers, and text literals.
pub(crate) struct EvalContext<'a> {
    pub asts: &'a HashMap<String, Expr>,
    pub sources: &'a HashMap<String, String>,
    pub cache: &'a HashMap<String, f64>,
    pub text_cells: &'a HashMap<String, String>,
    /// Cell currently being evaluated (for `@` implicit intersection).
    pub eval_cell: &'a str,
    /// Spill footprints written by earlier layers: anchor → (height, width).
    pub spill_meta: &'a HashMap<String, (usize, usize)>,
    /// `LET` name bindings for the active evaluation scope.
    pub bindings: Option<&'a HashMap<String, EvalValue>>,
}

impl<'a> EvalContext<'a> {
    pub(crate) fn lookup_binding(&self, name: &str) -> Option<&'a EvalValue> {
        self.bindings.and_then(|b| b.get(name))
    }

    pub(crate) fn lookup_number(&self, cell_name: &str) -> Result<f64, SpreadsheetError> {
        if let Some(value) = self.lookup_binding(cell_name) {
            return match value {
                EvalValue::Number(n) => Ok(*n),
                EvalValue::Array(rows) => rows
                    .first()
                    .and_then(|row| row.first())
                    .copied()
                    .ok_or(SpreadsheetError::Value),
                EvalValue::Text(_) => Err(SpreadsheetError::Value),
            };
        }
        if let Some(value) = self.cache.get(cell_name) {
            return Ok(*value);
        }
        if self.text_cells.contains_key(cell_name) {
            return Err(SpreadsheetError::Value);
        }
        // Excel: blank / missing cells coerce to 0 in numeric contexts.
        Ok(0.0)
    }

    /// Aggregate functions (SUM/AVERAGE/…) skip text and blanks like Excel.
    /// Returns `Ok(None)` to omit the value from the argument list.
    pub(crate) fn lookup_aggregate_number(
        &self,
        cell_name: &str,
    ) -> Result<Option<f64>, SpreadsheetError> {
        if let Some(value) = self.lookup_binding(cell_name) {
            return match value {
                EvalValue::Number(n) => Ok(Some(*n)),
                EvalValue::Array(_) | EvalValue::Text(_) => Ok(None),
            };
        }
        if let Some(value) = self.cache.get(cell_name) {
            return Ok(Some(*value));
        }
        if self.text_cells.contains_key(cell_name) {
            return Ok(None);
        }
        // Blank / missing: omit from SUM/AVERAGE/… (same as Excel).
        Ok(None)
    }

    pub(crate) fn count_cell_kind(&self, cell_name: &str) -> CellKind {
        if let Some(value) = self.lookup_binding(cell_name) {
            return match value {
                EvalValue::Number(_) | EvalValue::Array(_) => CellKind::Number,
                EvalValue::Text(_) => CellKind::Text,
            };
        }
        if self.cache.contains_key(cell_name) {
            CellKind::Number
        } else if self.text_cells.contains_key(cell_name) {
            CellKind::Text
        } else {
            CellKind::Blank
        }
    }

    pub(crate) fn cell_text_value(&self, cell_name: &str) -> Result<String, SpreadsheetError> {
        self.text_cells
            .get(cell_name)
            .cloned()
            .ok_or(SpreadsheetError::Value)
    }

    /// Text coercion for `&`: blank/missing → `""`, numbers via [`crate::format_number`].
    pub(crate) fn concat_cell_text(&self, cell_name: &str) -> Result<String, SpreadsheetError> {
        if let Some(text) = self.text_cells.get(cell_name) {
            return Ok(text.clone());
        }
        if let Some(n) = self.cache.get(cell_name) {
            return Ok(crate::format_number(*n));
        }
        Ok(String::new())
    }
}

pub(crate) fn evaluate_formula(
    ctx: &EvalContext<'_>,
    cell_name: &str,
) -> Result<EvalValue, SpreadsheetError> {
    let expr = ctx
        .asts
        .get(cell_name)
        .ok_or_else(|| SpreadsheetError::UnknownCell(cell_name.to_string()))?;
    let source = ctx
        .sources
        .get(cell_name)
        .map(|s| s.as_str())
        .unwrap_or(cell_name);
    eval_value(expr, ctx, source)
}

pub(crate) fn is_pure_string_expression(expression: &str) -> bool {
    parse_standalone_string_literal(expression).is_some()
}

pub(crate) fn pure_string_value(expression: &str) -> Option<String> {
    parse_standalone_string_literal(expression)
}
