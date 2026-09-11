use crate::error::SpreadsheetError;

pub(crate) fn is_truthy(value: f64) -> bool {
    value != 0.0
}

pub(crate) fn bool_to_number(value: bool) -> f64 {
    if value {
        1.0
    } else {
        0.0
    }
}

fn require_args(args: &[f64], expected: usize, expression: &str) -> Result<(), SpreadsheetError> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(SpreadsheetError::InvalidFormula(expression.to_string()))
    }
}

fn excel_mod(number: f64, divisor: f64) -> Result<f64, SpreadsheetError> {
    if divisor == 0.0 {
        return Err(SpreadsheetError::DivisionByZero);
    }
    Ok(number - divisor * (number / divisor).floor())
}

#[derive(Clone, Copy)]
enum RoundMode {
    HalfAwayFromZero,
    Up,
    Down,
}

fn round_with_mode(
    args: &[f64],
    mode: RoundMode,
    expression: &str,
) -> Result<f64, SpreadsheetError> {
    if args.is_empty() || args.len() > 2 {
        return Err(SpreadsheetError::InvalidFormula(expression.to_string()));
    }

    let number = args[0];
    let digits = if args.len() == 2 {
        args[1].trunc() as i32
    } else {
        0
    };

    let factor = 10f64.powi(digits);
    let scaled = number * factor;
    let rounded = match mode {
        RoundMode::HalfAwayFromZero => scaled.round(),
        RoundMode::Up => {
            if number >= 0.0 {
                scaled.ceil()
            } else {
                scaled.floor()
            }
        }
        RoundMode::Down => {
            if number >= 0.0 {
                scaled.floor()
            } else {
                scaled.ceil()
            }
        }
    };

    Ok(rounded / factor)
}

/// Eager numeric / logical functions that take already-evaluated arguments.
pub(crate) fn eval_eager_function(
    name: &str,
    args: &[f64],
    expression: &str,
) -> Result<f64, SpreadsheetError> {
    match name.to_uppercase().as_str() {
        "SUM" => Ok(args.iter().sum()),
        "AVERAGE" => {
            if args.is_empty() {
                return Err(SpreadsheetError::DivisionByZero);
            }
            Ok(args.iter().sum::<f64>() / args.len() as f64)
        }
        "MIN" => {
            if args.is_empty() {
                Ok(0.0)
            } else {
                Ok(args.iter().copied().fold(f64::INFINITY, f64::min))
            }
        }
        "MAX" => {
            if args.is_empty() {
                Ok(0.0)
            } else {
                Ok(args.iter().copied().fold(f64::NEG_INFINITY, f64::max))
            }
        }
        "PRODUCT" => {
            if args.is_empty() {
                Ok(0.0)
            } else {
                Ok(args.iter().product())
            }
        }
        "ABS" => {
            require_args(args, 1, expression)?;
            Ok(args[0].abs())
        }
        "INT" => {
            require_args(args, 1, expression)?;
            Ok(args[0].floor())
        }
        "SQRT" => {
            require_args(args, 1, expression)?;
            if args[0] < 0.0 {
                return Err(SpreadsheetError::Num);
            }
            Ok(args[0].sqrt())
        }
        "POWER" => {
            require_args(args, 2, expression)?;
            Ok(args[0].powf(args[1]))
        }
        "MOD" => {
            require_args(args, 2, expression)?;
            excel_mod(args[0], args[1])
        }
        "ROUND" => round_with_mode(args, RoundMode::HalfAwayFromZero, expression),
        "ROUNDUP" => round_with_mode(args, RoundMode::Up, expression),
        "ROUNDDOWN" => round_with_mode(args, RoundMode::Down, expression),
        "AND" => Ok(bool_to_number(args.iter().all(|&v| is_truthy(v)))),
        "OR" => Ok(bool_to_number(args.iter().any(|&v| is_truthy(v)))),
        "NOT" => {
            require_args(args, 1, expression)?;
            Ok(bool_to_number(!is_truthy(args[0])))
        }
        _ => Err(SpreadsheetError::InvalidFormula(expression.to_string())),
    }
}
