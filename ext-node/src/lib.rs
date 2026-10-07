use std::collections::HashMap;

use calc_spreadsheet::{
    calculate_spreadsheet, format_number, CalculateOptions, CellValue, ParallelThresholds,
    ReplacementValue,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;

fn cell_input_to_string(value: Either<String, f64>) -> String {
    match value {
        Either::A(s) => s,
        Either::B(n) => format_number(n),
    }
}

fn cell_input_to_replacement(value: Either<String, f64>) -> ReplacementValue {
    match value {
        Either::A(s) => ReplacementValue::from_text(s),
        Either::B(n) => {
            if n.is_finite() && n.fract() == 0.0 && n.abs() < (1i64 << 53) as f64 {
                ReplacementValue::from_i64(n as i64)
            } else {
                ReplacementValue::from_f64(n)
            }
        }
    }
}

fn cell_value_to_js(value: CellValue) -> Either<String, f64> {
    match value {
        CellValue::Number(n) => Either::B(n),
        CellValue::Text(s) => Either::A(s),
    }
}

fn resolve_thresholds(
    min_layer_width: Option<i64>,
    min_layer_work: Option<i64>,
) -> Result<ParallelThresholds> {
    let defaults = ParallelThresholds::default();
    let min_layer_width = match min_layer_width {
        None => defaults.min_layer_width,
        Some(n) if n < 0 => {
            return Err(Error::from_reason(
                "min_layer_width must be >= 0".to_string(),
            ));
        }
        Some(n) => n as usize,
    };
    let min_layer_work = match min_layer_work {
        None => defaults.min_layer_work,
        Some(n) if n < 0 => {
            return Err(Error::from_reason(
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

fn warn_ignored_replacement_keys(env: &Env, keys: &[String]) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    let list = keys.join(", ");
    let message = format!(
        "calc_spreadsheet: ignored invalid replacement key(s): {list} (expected __[A-Z0-9]+__)"
    );
    let console = env
        .get_global()?
        .get_named_property::<Object>("console")?;
    let warn = console.get_named_property::<Function<'_, String, ()>>("warn")?;
    warn.apply(&console, message)?;
    Ok(())
}

/// オブジェクト `{ A1: '=1+2', ... }` を受け取り、計算後のオブジェクトを返す。
///
/// 第2引数で `__NAME__` 形式のプレースホルダを置換する。
/// 不正な置換キーは無視し、`console.warn` を出す（計算は続行）。
/// 第3・第4引数で並列適応の閾値（層幅・仕事量）を上書きできる。省略時はエンジン既定値。
#[napi(js_name = "calcSpreadsheet")]
pub fn calc_spreadsheet(
    env: Env,
    cells: HashMap<String, Either<String, f64>>,
    replacements: Option<HashMap<String, Either<String, f64>>>,
    min_layer_width: Option<i64>,
    min_layer_work: Option<i64>,
) -> Result<HashMap<String, Either<String, f64>>> {
    let thresholds = resolve_thresholds(min_layer_width, min_layer_work)?;
    let replacements: HashMap<String, ReplacementValue> = replacements
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, cell_input_to_replacement(v)))
        .collect();
    let owned: Vec<(String, String)> = cells
        .into_iter()
        .map(|(name, value)| (name, cell_input_to_string(value)))
        .collect();
    let input: Vec<(&str, &str)> = owned
        .iter()
        .map(|(name, expr)| (name.as_str(), expr.as_str()))
        .collect();

    let outcome = calculate_spreadsheet(
        &input,
        CalculateOptions {
            replacements: Some(&replacements),
            thresholds: Some(thresholds),
        },
    )
    .map_err(|err| Error::from_reason(err.to_string()))?;

    warn_ignored_replacement_keys(&env, &outcome.ignored_replacement_keys)?;

    Ok(outcome
        .values
        .into_iter()
        .map(|(name, value)| (name, cell_value_to_js(value)))
        .collect())
}
