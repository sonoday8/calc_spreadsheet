#![cfg_attr(windows, feature(abi_vectorcall))]

use std::collections::HashMap;

use calc_spreadsheet::{calculate_spreadsheet_with_thresholds, CellValue, ParallelThresholds};
use ext_php_rs::prelude::*;

/// PHP から受け取るセル値。数値リテラルは計算エンジン向けに文字列化する。
#[derive(Debug, ZvalConvert)]
pub enum PhpCellInput {
    Long(i64),
    Number(f64),
    Text(String),
}

/// PHP へ返す計算結果。数値と文字列をそのままの型で返す。
#[derive(Debug, ZvalConvert)]
pub enum PhpCellOutput {
    Number(f64),
    Text(String),
}

impl From<PhpCellInput> for String {
    fn from(value: PhpCellInput) -> Self {
        match value {
            PhpCellInput::Long(n) => n.to_string(),
            PhpCellInput::Number(n) => n.to_string(),
            PhpCellInput::Text(s) => s,
        }
    }
}

impl From<CellValue> for PhpCellOutput {
    fn from(value: CellValue) -> Self {
        match value {
            CellValue::Number(n) => PhpCellOutput::Number(n),
            CellValue::Text(s) => PhpCellOutput::Text(s),
        }
    }
}

fn resolve_thresholds(
    min_layer_width: Option<i64>,
    min_layer_work: Option<i64>,
) -> Result<ParallelThresholds, PhpException> {
    let defaults = ParallelThresholds::default();
    let min_layer_width = match min_layer_width {
        None => defaults.min_layer_width,
        Some(n) if n < 0 => {
            return Err(PhpException::default(
                "min_layer_width must be >= 0".to_string(),
            ));
        }
        Some(n) => n as usize,
    };
    let min_layer_work = match min_layer_work {
        None => defaults.min_layer_work,
        Some(n) if n < 0 => {
            return Err(PhpException::default(
                "min_layer_work must be >= 0".to_string(),
            ));
        }
        Some(n) => n as usize,
    };
    Ok(ParallelThresholds {
        min_layer_width,
        min_layer_work,
    })
}

/// 連想配列 `['A1' => '=1+2', ...]` を受け取り、計算後の連想配列を返す。
///
/// 第2・第3引数で並列適応の閾値（層幅・仕事量）を上書きできる。省略時はエンジン既定値。
#[php_function]
pub fn calc_spreadsheet(
    cells: HashMap<String, PhpCellInput>,
    min_layer_width: Option<i64>,
    min_layer_work: Option<i64>,
) -> Result<HashMap<String, PhpCellOutput>, PhpException> {
    let thresholds = resolve_thresholds(min_layer_width, min_layer_work)?;
    let owned: Vec<(String, String)> = cells
        .into_iter()
        .map(|(name, value)| (name, String::from(value)))
        .collect();
    let input: Vec<(&str, &str)> = owned
        .iter()
        .map(|(name, expr)| (name.as_str(), expr.as_str()))
        .collect();

    calculate_spreadsheet_with_thresholds(&input, thresholds)
        .map(|values| {
            values
                .into_iter()
                .map(|(name, value)| (name, PhpCellOutput::from(value)))
                .collect()
        })
        .map_err(|err| PhpException::default(err.to_string()))
}

#[php_module]
pub fn get_module(module: ModuleBuilder) -> ModuleBuilder {
    module.function(wrap_function!(calc_spreadsheet))
}
