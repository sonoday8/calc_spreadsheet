use std::fmt;

/// Evaluated cell value. Numbers are Excel-compatible serials / formula results;
/// text is preserved for literals such as DATEDIF units.
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Number(f64),
    Text(String),
}

impl CellValue {
    pub fn as_number(&self) -> Option<f64> {
        match self {
            CellValue::Number(value) => Some(*value),
            CellValue::Text(_) => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            CellValue::Number(_) => None,
            CellValue::Text(value) => Some(value),
        }
    }
}

impl fmt::Display for CellValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CellValue::Number(value) => write!(f, "{value}"),
            CellValue::Text(value) => write!(f, "{value}"),
        }
    }
}

impl PartialEq<f64> for CellValue {
    fn eq(&self, other: &f64) -> bool {
        matches!(self, CellValue::Number(value) if value == other)
    }
}
