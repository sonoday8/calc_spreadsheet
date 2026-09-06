//! Formula AST: single structure for evaluation and reference analysis.

use crate::error::SpreadsheetError;
use crate::excel_date;
use crate::functions::{bool_to_number, eval_eager_function, is_truthy};
use crate::refs::{
    FormulaRefs, a1_range_size, expand_a1_range, for_each_a1_range, is_cell_name_char,
    is_cell_name_start, parse_a1,
};
use crate::spreadsheet::{CellKind, EvalContext};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
    Number(f64),
    Cell(String),
    /// A1 range kept as endpoints until expand (collect / aggregate flatten / scalar).
    Range {
        start: String,
        end: String,
    },
    Str(String),
    Neg(Box<Expr>),
    BinOp {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    CmpOp {
        op: CmpOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CmpOp {
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
}

pub(crate) fn parse_formula(expression: &str) -> Result<Expr, SpreadsheetError> {
    let expression = expression.trim().strip_prefix('=').unwrap_or(expression.trim());
    let mut parser = AstParser {
        chars: expression.chars().collect(),
        position: 0,
        expression,
    };
    let expr = parser.parse_comparison()?;
    parser.skip_whitespace();
    if parser.position < parser.chars.len() {
        return Err(SpreadsheetError::InvalidFormula(expression.to_string()));
    }
    Ok(expr)
}

pub(crate) fn analyze_from_ast(expr: &Expr) -> Result<FormulaRefs, SpreadsheetError> {
    let mut refs = FormulaRefs {
        cells: Default::default(),
        work: 0,
    };
    collect_refs(expr, &mut refs)?;
    Ok(refs)
}

#[cfg(test)]
pub(crate) fn analyze_formula_refs(expression: &str) -> Result<FormulaRefs, SpreadsheetError> {
    analyze_from_ast(&parse_formula(expression)?)
}

pub(crate) fn eval_expr(
    expr: &Expr,
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<f64, SpreadsheetError> {
    match expr {
        Expr::Number(n) => Ok(*n),
        Expr::Cell(name) => ctx.lookup_number(name),
        Expr::Range { start, end } => eval_range_scalar(start, end, ctx),
        Expr::Str(_) => Err(SpreadsheetError::Value),
        Expr::Neg(inner) => Ok(-eval_expr(inner, ctx, source)?),
        Expr::BinOp { op, left, right } => {
            let l = eval_expr(left, ctx, source)?;
            let r = eval_expr(right, ctx, source)?;
            match op {
                BinOp::Add => Ok(l + r),
                BinOp::Sub => Ok(l - r),
                BinOp::Mul => Ok(l * r),
                BinOp::Div => {
                    if r == 0.0 {
                        Err(SpreadsheetError::DivisionByZero)
                    } else {
                        Ok(l / r)
                    }
                }
            }
        }
        Expr::CmpOp { op, left, right } => {
            let l = eval_expr(left, ctx, source)?;
            let r = eval_expr(right, ctx, source)?;
            let flag = match op {
                CmpOp::Gt => l > r,
                CmpOp::Lt => l < r,
                CmpOp::Ge => l >= r,
                CmpOp::Le => l <= r,
                CmpOp::Eq => l == r,
                CmpOp::Ne => l != r,
            };
            Ok(bool_to_number(flag))
        }
        Expr::Call { name, args } => eval_call(name, args, ctx, source),
    }
}

fn eval_range_scalar(
    start: &str,
    end: &str,
    ctx: &EvalContext<'_>,
) -> Result<f64, SpreadsheetError> {
    let cells = expand_a1_range(start, end)?;
    match cells.as_slice() {
        [only] => ctx.lookup_number(only),
        _ => Err(SpreadsheetError::Value),
    }
}

fn eval_call(
    name: &str,
    args: &[Expr],
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<f64, SpreadsheetError> {
    match name.to_uppercase().as_str() {
        "IF" => {
            require_arity(args, 3, source)?;
            if is_truthy(eval_expr(&args[0], ctx, source)?) {
                eval_expr(&args[1], ctx, source)
            } else {
                eval_expr(&args[2], ctx, source)
            }
        }
        "IFERROR" => {
            if args.is_empty() || args.len() > 2 {
                return Err(SpreadsheetError::InvalidFormula(source.to_string()));
            }
            match eval_expr(&args[0], ctx, source) {
                Ok(v) => Ok(v),
                Err(_) => {
                    if let Some(fb) = args.get(1) {
                        eval_expr(fb, ctx, source)
                    } else {
                        Ok(0.0)
                    }
                }
            }
        }
        "IFS" => {
            if args.len() < 2 || args.len() % 2 != 0 {
                return Err(SpreadsheetError::InvalidFormula(source.to_string()));
            }
            for pair in args.chunks(2) {
                if is_truthy(eval_expr(&pair[0], ctx, source)?) {
                    return eval_expr(&pair[1], ctx, source);
                }
            }
            Err(SpreadsheetError::InvalidFormula(source.to_string()))
        }
        "SWITCH" => eval_switch(args, ctx, source),
        "COUNT" => Ok(count_args(args, ctx, source, false)? as f64),
        "COUNTA" => Ok(count_args(args, ctx, source, true)? as f64),
        "DATE" => {
            require_arity(args, 3, source)?;
            let year = eval_expr(&args[0], ctx, source)?;
            let month = eval_expr(&args[1], ctx, source)?;
            let day = eval_expr(&args[2], ctx, source)?;
            excel_date::excel_date(year.trunc() as i32, month.trunc() as i32, day.trunc() as i32)
                .map_err(|_| SpreadsheetError::Num)
        }
        "DATEVALUE" => {
            require_arity(args, 1, source)?;
            let text = eval_text_arg(&args[0], ctx)?;
            excel_date::datevalue(&text).map_err(|_| SpreadsheetError::Value)
        }
        "YEAR" | "MONTH" | "DAY" => {
            require_arity(args, 1, source)?;
            let serial = eval_date_arg(&args[0], ctx, source)?;
            let civil = excel_date::serial_to_ymd(serial).map_err(|_| SpreadsheetError::Num)?;
            Ok(match name.to_uppercase().as_str() {
                "YEAR" => f64::from(civil.year),
                "MONTH" => f64::from(civil.month),
                _ => f64::from(civil.day),
            })
        }
        "DAYS" => {
            require_arity(args, 2, source)?;
            let end = eval_date_arg(&args[0], ctx, source)?;
            let start = eval_date_arg(&args[1], ctx, source)?;
            let end_civil = excel_date::serial_to_ymd(end).map_err(|_| SpreadsheetError::Num)?;
            let start_civil = excel_date::serial_to_ymd(start).map_err(|_| SpreadsheetError::Num)?;
            let end_serial =
                excel_date::ymd_to_serial(end_civil.year, end_civil.month, end_civil.day)
                    .map_err(|_| SpreadsheetError::Num)?;
            let start_serial =
                excel_date::ymd_to_serial(start_civil.year, start_civil.month, start_civil.day)
                    .map_err(|_| SpreadsheetError::Num)?;
            Ok(end_serial - start_serial)
        }
        "DATEDIF" => {
            require_arity(args, 3, source)?;
            let start = eval_date_arg(&args[0], ctx, source)?;
            let end = eval_date_arg(&args[1], ctx, source)?;
            let unit = eval_text_arg(&args[2], ctx)?;
            let start = excel_date::serial_to_ymd(start).map_err(|_| SpreadsheetError::Num)?;
            let end = excel_date::serial_to_ymd(end).map_err(|_| SpreadsheetError::Num)?;
            excel_date::datedif(start, end, &unit).map_err(|err| match err {
                excel_date::DatedifError::Num => SpreadsheetError::Num,
                excel_date::DatedifError::Value => SpreadsheetError::Value,
            })
        }
        _ => {
            let flat = flatten_numeric_args(args, ctx, source)?;
            eval_eager_function(name, &flat, source)
        }
    }
}

fn eval_switch(
    args: &[Expr],
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<f64, SpreadsheetError> {
    if args.is_empty() {
        return Err(SpreadsheetError::InvalidFormula(source.to_string()));
    }
    let target = eval_expr(&args[0], ctx, source)?;
    let rest = &args[1..];
    let mut i = 0;
    while i + 1 < rest.len() {
        if eval_expr(&rest[i], ctx, source)? == target {
            return eval_expr(&rest[i + 1], ctx, source);
        }
        i += 2;
    }
    if rest.len() % 2 == 1 {
        eval_expr(&rest[rest.len() - 1], ctx, source)
    } else {
        Err(SpreadsheetError::InvalidFormula(source.to_string()))
    }
}

fn flatten_numeric_args(
    args: &[Expr],
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<Vec<f64>, SpreadsheetError> {
    let mut out = Vec::new();
    for arg in args {
        match arg {
            Expr::Range { start, end } => {
                for_each_a1_range(start, end, |cell| {
                    if let Some(n) = ctx.lookup_aggregate_number(&cell)? {
                        out.push(n);
                    }
                    Ok(())
                })?;
            }
            Expr::Str(_) => {
                // Excel SUM/AVERAGE/… ignore text literals.
            }
            Expr::Cell(name) => {
                if let Some(n) = ctx.lookup_aggregate_number(name)? {
                    out.push(n);
                }
            }
            other => out.push(eval_expr(other, ctx, source)?),
        }
    }
    Ok(out)
}

/// Excel COUNT counts numbers only; COUNTA counts non-blank (numbers + text).
fn count_args(
    args: &[Expr],
    ctx: &EvalContext<'_>,
    source: &str,
    count_text: bool,
) -> Result<usize, SpreadsheetError> {
    let mut n = 0usize;
    for arg in args {
        match arg {
            Expr::Range { start, end } => {
                for_each_a1_range(start, end, |cell| {
                    match ctx.count_cell_kind(&cell) {
                        CellKind::Number => n += 1,
                        CellKind::Text if count_text => n += 1,
                        CellKind::Text | CellKind::Blank => {}
                    }
                    Ok(())
                })?;
            }
            Expr::Str(_) => {
                if count_text {
                    n += 1;
                }
            }
            Expr::Cell(name) => match ctx.count_cell_kind(name) {
                CellKind::Number => n += 1,
                CellKind::Text if count_text => n += 1,
                CellKind::Text | CellKind::Blank => {}
            },
            other => {
                let _ = eval_expr(other, ctx, source)?;
                n += 1;
            }
        }
    }
    Ok(n)
}

fn eval_date_arg(
    expr: &Expr,
    ctx: &EvalContext<'_>,
    source: &str,
) -> Result<f64, SpreadsheetError> {
    match expr {
        Expr::Str(text) => excel_date::datevalue(text).map_err(|_| SpreadsheetError::Value),
        other => eval_expr(other, ctx, source),
    }
}

fn eval_text_arg(expr: &Expr, ctx: &EvalContext<'_>) -> Result<String, SpreadsheetError> {
    match expr {
        Expr::Str(text) => Ok(text.clone()),
        Expr::Cell(name) => ctx.cell_text_value(name),
        _ => Err(SpreadsheetError::Value),
    }
}

fn require_arity(args: &[Expr], n: usize, source: &str) -> Result<(), SpreadsheetError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(SpreadsheetError::InvalidFormula(source.to_string()))
    }
}

fn collect_refs(expr: &Expr, refs: &mut FormulaRefs) -> Result<(), SpreadsheetError> {
    match expr {
        Expr::Number(_) | Expr::Str(_) => {}
        Expr::Cell(name) => {
            refs.work += 1;
            refs.cells.insert(name.clone());
        }
        Expr::Range { start, end } => {
            let size = a1_range_size(start, end)?;
            refs.work += size;
            for_each_a1_range(start, end, |cell| {
                refs.cells.insert(cell);
                Ok(())
            })?;
        }
        Expr::Neg(inner) => collect_refs(inner, refs)?,
        Expr::BinOp { left, right, .. } | Expr::CmpOp { left, right, .. } => {
            collect_refs(left, refs)?;
            collect_refs(right, refs)?;
        }
        Expr::Call { args, .. } => {
            for arg in args {
                collect_refs(arg, refs)?;
            }
        }
    }
    Ok(())
}

struct AstParser<'a> {
    chars: Vec<char>,
    position: usize,
    expression: &'a str,
}

impl<'a> AstParser<'a> {
    fn parse_comparison(&mut self) -> Result<Expr, SpreadsheetError> {
        let mut left = self.parse_sum()?;
        loop {
            self.skip_whitespace();
            let op = if self.match_str(">=") {
                CmpOp::Ge
            } else if self.match_str("<=") {
                CmpOp::Le
            } else if self.match_str("<>") || self.match_str("!=") {
                CmpOp::Ne
            } else if self.match_str("==") || self.match_str("=") {
                CmpOp::Eq
            } else if self.match_str(">") {
                CmpOp::Gt
            } else if self.match_str("<") {
                CmpOp::Lt
            } else {
                return Ok(left);
            };
            let right = self.parse_sum()?;
            left = Expr::CmpOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn parse_sum(&mut self) -> Result<Expr, SpreadsheetError> {
        let mut left = self.parse_product()?;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some('+') => {
                    self.position += 1;
                    let right = self.parse_product()?;
                    left = Expr::BinOp {
                        op: BinOp::Add,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                Some('-') => {
                    self.position += 1;
                    let right = self.parse_product()?;
                    left = Expr::BinOp {
                        op: BinOp::Sub,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                _ => return Ok(left),
            }
        }
    }

    fn parse_product(&mut self) -> Result<Expr, SpreadsheetError> {
        let mut left = self.parse_factor()?;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some('*') => {
                    self.position += 1;
                    let right = self.parse_factor()?;
                    left = Expr::BinOp {
                        op: BinOp::Mul,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                Some('/') => {
                    self.position += 1;
                    let right = self.parse_factor()?;
                    left = Expr::BinOp {
                        op: BinOp::Div,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                _ => return Ok(left),
            }
        }
    }

    fn parse_factor(&mut self) -> Result<Expr, SpreadsheetError> {
        self.skip_whitespace();
        match self.peek() {
            Some('+') => {
                self.position += 1;
                self.parse_factor()
            }
            Some('-') => {
                self.position += 1;
                Ok(Expr::Neg(Box::new(self.parse_factor()?)))
            }
            Some('(') => {
                self.position += 1;
                let inner = self.parse_comparison()?;
                self.skip_whitespace();
                if self.peek() != Some(')') {
                    return Err(SpreadsheetError::InvalidFormula(self.expression.to_string()));
                }
                self.position += 1;
                Ok(inner)
            }
            Some('"') => Ok(Expr::Str(self.parse_string_literal()?)),
            Some(ch) if ch.is_ascii_digit() || ch == '.' => Ok(Expr::Number(self.parse_number()?)),
            Some(ch) if is_cell_name_start(ch) => self.parse_identifier_or_call(),
            _ => Err(SpreadsheetError::InvalidFormula(self.expression.to_string())),
        }
    }

    fn parse_identifier_or_call(&mut self) -> Result<Expr, SpreadsheetError> {
        let start = self.position;
        while matches!(self.peek(), Some(ch) if is_cell_name_char(ch)) {
            self.position += 1;
        }
        let name: String = self.chars[start..self.position].iter().collect();
        self.skip_whitespace();

        if self.peek() == Some('(') {
            self.position += 1;
            let args = self.parse_call_args(&name)?;
            self.skip_whitespace();
            if self.peek() != Some(')') {
                return Err(SpreadsheetError::InvalidFormula(self.expression.to_string()));
            }
            self.position += 1;
            return Ok(Expr::Call { name, args });
        }

        if self.peek() == Some(':') && parse_a1(&name).is_some() {
            let saved = self.position;
            self.position += 1;
            self.skip_whitespace();
            if matches!(self.peek(), Some(ch) if is_cell_name_start(ch)) {
                let end_start = self.position;
                while matches!(self.peek(), Some(ch) if is_cell_name_char(ch)) {
                    self.position += 1;
                }
                let end_name: String = self.chars[end_start..self.position].iter().collect();
                if parse_a1(&end_name).is_some() {
                    a1_range_size(&name, &end_name)?;
                    return Ok(Expr::Range {
                        start: name,
                        end: end_name,
                    });
                }
            }
            self.position = saved;
        }

        Ok(Expr::Cell(name))
    }

    fn parse_call_args(&mut self, name: &str) -> Result<Vec<Expr>, SpreadsheetError> {
        self.skip_whitespace();
        if self.peek() == Some(')') {
            return Ok(Vec::new());
        }

        // Text-oriented first args for DATEVALUE / DATEDIF unit still use factor (Str | Cell | expr).
        let upper = name.to_uppercase();
        let mut args = Vec::new();
        loop {
            // Allow ranges and expressions uniformly via parse_comparison,
            // except bare strings already handled in parse_factor.
            if upper == "DATEDIF" && args.len() == 2 {
                args.push(self.parse_factor()?);
            } else if upper == "DATEVALUE" {
                args.push(self.parse_factor()?);
            } else {
                args.push(self.parse_comparison()?);
            }
            self.skip_whitespace();
            match self.peek() {
                Some(',') => {
                    self.position += 1;
                    self.skip_whitespace();
                }
                Some(')') => break,
                _ => {
                    return Err(SpreadsheetError::InvalidFormula(self.expression.to_string()))
                }
            }
        }
        Ok(args)
    }

    fn parse_string_literal(&mut self) -> Result<String, SpreadsheetError> {
        if self.peek() != Some('"') {
            return Err(SpreadsheetError::InvalidFormula(self.expression.to_string()));
        }
        self.position += 1;
        let mut value = String::new();
        while let Some(ch) = self.peek() {
            self.position += 1;
            if ch == '"' {
                if self.peek() == Some('"') {
                    self.position += 1;
                    value.push('"');
                } else {
                    return Ok(value);
                }
            } else {
                value.push(ch);
            }
        }
        Err(SpreadsheetError::InvalidFormula(self.expression.to_string()))
    }

    fn parse_number(&mut self) -> Result<f64, SpreadsheetError> {
        let start = self.position;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_digit() || ch == '.') {
            self.position += 1;
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.position += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.position += 1;
            }
            let exponent_start = self.position;
            while matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
                self.position += 1;
            }
            if self.position == exponent_start {
                return Err(SpreadsheetError::InvalidFormula(self.expression.to_string()));
            }
        }
        let number: String = self.chars[start..self.position].iter().collect();
        number
            .parse::<f64>()
            .map_err(|_| SpreadsheetError::InvalidFormula(self.expression.to_string()))
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(ch) if ch.is_whitespace()) {
            self.position += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.position).copied()
    }

    fn match_str(&mut self, s: &str) -> bool {
        let s_chars: Vec<char> = s.chars().collect();
        if self.position + s_chars.len() <= self.chars.len()
            && self.chars[self.position..self.position + s_chars.len()] == s_chars
        {
            self.position += s_chars.len();
            true
        } else {
            false
        }
    }
}
