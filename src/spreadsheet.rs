use std::collections::HashMap;

use crate::ast::{eval_expr, Expr};
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
}

impl<'a> EvalContext<'a> {
    pub(crate) fn lookup_number(&self, cell_name: &str) -> Result<f64, SpreadsheetError> {
        if let Some(value) = self.cache.get(cell_name) {
            return Ok(*value);
        }
        if self.text_cells.contains_key(cell_name) {
            return Err(SpreadsheetError::Value);
        }
        Err(SpreadsheetError::UnknownCell(cell_name.to_string()))
    }

    /// Aggregate functions (SUM/AVERAGE/…) skip text cells like Excel.
    /// Returns `Ok(None)` to omit the value from the argument list.
    pub(crate) fn lookup_aggregate_number(
        &self,
        cell_name: &str,
    ) -> Result<Option<f64>, SpreadsheetError> {
        if let Some(value) = self.cache.get(cell_name) {
            return Ok(Some(*value));
        }
        if self.text_cells.contains_key(cell_name) {
            return Ok(None);
        }
        Err(SpreadsheetError::UnknownCell(cell_name.to_string()))
    }

    pub(crate) fn count_cell_kind(&self, cell_name: &str) -> CellKind {
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
}

pub(crate) fn evaluate_formula(
    ctx: &EvalContext<'_>,
    cell_name: &str,
) -> Result<f64, SpreadsheetError> {
    let expr = ctx
        .asts
        .get(cell_name)
        .ok_or_else(|| SpreadsheetError::UnknownCell(cell_name.to_string()))?;
    let source = ctx
        .sources
        .get(cell_name)
        .map(|s| s.as_str())
        .unwrap_or(cell_name);
    eval_expr(expr, ctx, source)
}

pub(crate) fn is_pure_string_expression(expression: &str) -> bool {
    parse_standalone_string_literal(expression).is_some()
}

pub(crate) fn pure_string_value(expression: &str) -> Option<String> {
    parse_standalone_string_literal(expression)
}
