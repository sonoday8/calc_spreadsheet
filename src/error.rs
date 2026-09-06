use std::fmt;

#[derive(Debug, PartialEq)]
pub enum SpreadsheetError {
    UnknownCell(String),
    CircularReference(String),
    InvalidFormula(String),
    DivisionByZero,
    /// Excel `#NUM!`
    Num,
    /// Excel `#VALUE!`
    Value,
}

impl fmt::Display for SpreadsheetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpreadsheetError::UnknownCell(cell_name) => {
                write!(f, "unknown cell: {cell_name}")
            }
            SpreadsheetError::CircularReference(cell_name) => {
                write!(f, "circular reference detected at: {cell_name}")
            }
            SpreadsheetError::InvalidFormula(expression) => {
                write!(f, "invalid formula: {expression}")
            }
            SpreadsheetError::DivisionByZero => write!(f, "division by zero"),
            SpreadsheetError::Num => write!(f, "#NUM!"),
            SpreadsheetError::Value => write!(f, "#VALUE!"),
        }
    }
}

impl std::error::Error for SpreadsheetError {}
