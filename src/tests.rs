use super::*;

#[test]
fn evaluates_values_and_formulas_recursively() {
    let cells = [
        ("A1", "10"),
        ("A2", "20"),
        ("B1", "=A1 + A2"),
        ("B2", "=B1 * 2"),
        ("C1", "=(B2 - A1) / 5"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();

    assert_eq!(values["A1"], 10.0);
    assert_eq!(values["A2"], 20.0);
    assert_eq!(values["B1"], 30.0);
    assert_eq!(values["B2"], 60.0);
    assert_eq!(values["C1"], 10.0);
}

#[test]
fn detects_circular_references() {
    let cells = [("A1", "=B1"), ("B1", "=A1")];

    assert!(matches!(
        calculate_spreadsheet(&cells),
        Err(SpreadsheetError::CircularReference(_))
    ));
}

#[test]
fn supports_sum_function() {
    let cells = [
        ("A1", "10"),
        ("A2", "20"),
        ("A3", "30"),
        ("B1", "=SUM(A1, A2, A3)"),
        ("B2", "=SUM(A1, 5)"),
        ("B3", "=SUM(A1:A3)"),
        ("B4", "=IF(1, SUM(A1:A2), 0)"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["B1"], 60.0);
    assert_eq!(values["B2"], 15.0);
    assert_eq!(values["B3"], 60.0);
    assert_eq!(values["B4"], 30.0);
}

#[test]
fn multi_cell_range_is_value_in_scalar_context() {
    let cells = [("A1", "1"), ("A2", "2"), ("B1", "=A1:A2")];
    assert_eq!(
        calculate_spreadsheet(&cells),
        Err(SpreadsheetError::Value)
    );
}

#[test]
fn oversized_range_is_num() {
    let cells = [("Out", "=SUM(A1:ZZ9000)")];
    assert_eq!(calculate_spreadsheet(&cells), Err(SpreadsheetError::Num));
}

#[test]
fn sum_skips_text_cells_like_excel() {
    let cells = [
        ("A1", "10"),
        ("A2", "\"hello\""),
        ("A3", "30"),
        ("B1", "=SUM(A1:A3)"),
        ("B2", "=SUM(A1, \"x\", A3)"),
    ];
    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["B1"], 40.0);
    assert_eq!(values["B2"], 40.0);
}

#[test]
fn supports_if_function() {
    let cells = [
        ("A1", "10"),
        ("B1", "=IF(A1 > 5, 1, 0)"),
        ("B2", "=IF(A1 < 5, 1, 0)"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 0.0);
}

#[test]
fn if_short_circuits_unused_branch() {
    let cells = [
        ("A1", "0"),
        ("B1", "=IF(A1, 1 / 0, 42)"),
        ("B2", "=IF(1, 7, 1 / 0)"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["B1"], 42.0);
    assert_eq!(values["B2"], 7.0);
}

#[test]
fn supports_aggregate_functions() {
    let cells = [
        ("A1", "10"),
        ("A2", "20"),
        ("A3", "30"),
        ("B1", "=AVERAGE(A1, A2, A3)"),
        ("B2", "=MIN(A1, A2, A3)"),
        ("B3", "=MAX(A1, A2, A3)"),
        ("B4", "=COUNT(A1, A2, A3)"),
        ("B5", "=COUNTA(A1, A2, A3)"),
        ("B6", "=PRODUCT(A1, A2, A3)"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["B1"], 20.0);
    assert_eq!(values["B2"], 10.0);
    assert_eq!(values["B3"], 30.0);
    assert_eq!(values["B4"], 3.0);
    assert_eq!(values["B5"], 3.0);
    assert_eq!(values["B6"], 6000.0);
}

#[test]
fn count_skips_text_counta_includes_text() {
    let cells = [
        ("A1", "10"),
        ("A2", "\"hello\""),
        ("A3", "30"),
        ("B1", "=COUNT(A1:A3)"),
        ("B2", "=COUNTA(A1:A3)"),
        ("B3", "=COUNT(A1, \"x\", A3)"),
        ("B4", "=COUNTA(A1, \"x\", A3)"),
    ];
    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["B1"], 2.0);
    assert_eq!(values["B2"], 3.0);
    assert_eq!(values["B3"], 2.0);
    assert_eq!(values["B4"], 3.0);
}

#[test]
fn supports_math_functions() {
    let cells = [
        ("A1", "-3.7"),
        ("A2", "16"),
        ("A3", "2.5"),
        ("B1", "=ABS(A1)"),
        ("B2", "=INT(A1)"),
        ("B3", "=SQRT(A2)"),
        ("B4", "=POWER(A3, 2)"),
        ("B5", "=MOD(10, 3)"),
        ("B6", "=ROUND(2.35, 1)"),
        ("B7", "=ROUNDUP(2.31, 1)"),
        ("B8", "=ROUNDDOWN(2.39, 1)"),
        ("B9", "=ROUND(-2.5)"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["B1"], 3.7);
    assert_eq!(values["B2"], -4.0);
    assert_eq!(values["B3"], 4.0);
    assert_eq!(values["B4"], 6.25);
    assert_eq!(values["B5"], 1.0);
    assert!((values["B6"].as_number().unwrap() - 2.4).abs() < 1e-10);
    assert!((values["B7"].as_number().unwrap() - 2.4).abs() < 1e-10);
    assert!((values["B8"].as_number().unwrap() - 2.3).abs() < 1e-10);
    assert_eq!(values["B9"], -3.0);
}

#[test]
fn supports_logical_functions() {
    let cells = [
        ("A1", "1"),
        ("A2", "0"),
        ("B1", "=AND(A1, 1, 2)"),
        ("B2", "=AND(A1, A2)"),
        ("B3", "=OR(A2, 0, 5)"),
        ("B4", "=OR(A2, 0)"),
        ("B5", "=NOT(A2)"),
        ("B6", "=NOT(A1)"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 0.0);
    assert_eq!(values["B3"], 1.0);
    assert_eq!(values["B4"], 0.0);
    assert_eq!(values["B5"], 1.0);
    assert_eq!(values["B6"], 0.0);
}

#[test]
fn supports_ifs_switch_and_iferror() {
    let cells = [
        ("A1", "85"),
        ("A2", "0"),
        ("B1", "=IFS(A1 >= 90, 4, A1 >= 80, 3, A1 >= 70, 2, 1, 1)"),
        ("B2", "=SWITCH(A1, 70, 1, 85, 2, 100, 3, 0)"),
        ("B3", "=IFERROR(A1 / A2, -1)"),
        ("B4", "=IFERROR(A1 / 5, -1)"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["B1"], 3.0);
    assert_eq!(values["B2"], 2.0);
    assert_eq!(values["B3"], -1.0);
    assert_eq!(values["B4"], 17.0);
}

#[test]
fn average_without_args_is_division_by_zero() {
    let cells = [("A1", "=AVERAGE()")];

    assert_eq!(
        calculate_spreadsheet(&cells),
        Err(SpreadsheetError::DivisionByZero)
    );
}

#[test]
fn sqrt_rejects_negative_values() {
    let cells = [("A1", "=SQRT(-1)")];

    assert_eq!(
        calculate_spreadsheet(&cells),
        Err(SpreadsheetError::InvalidFormula("SQRT(-1)".to_string()))
    );
}

#[test]
fn supports_excel_date_parts_and_days() {
    let cells = [
        ("A1", "=DATE(2008, 1, 1)"),
        ("A2", "=DATE(2011, 3, 15)"),
        ("A3", "=DATE(2011, 2, 1)"),
        ("B1", "=YEAR(A1)"),
        ("B2", "=MONTH(A1)"),
        ("B3", "=DAY(A1)"),
        ("B4", "=DAYS(A2, A3)"),
        ("B5", "=YEAR(\"2008-01-01\")"),
        ("B6", "=DATEVALUE(\"2008/1/1\")"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["A1"], 39448.0);
    assert_eq!(values["B1"], 2008.0);
    assert_eq!(values["B2"], 1.0);
    assert_eq!(values["B3"], 1.0);
    assert_eq!(values["B4"], 42.0);
    assert_eq!(values["B5"], 2008.0);
    assert_eq!(values["B6"], 39448.0);
}

#[test]
fn supports_datedif_like_excel() {
    let cells = [
        ("A1", "=DATE(2001, 1, 1)"),
        ("A2", "=DATE(2003, 1, 1)"),
        ("A3", "=DATE(2001, 6, 1)"),
        ("A4", "=DATE(2002, 8, 15)"),
        ("B1", "=DATEDIF(A1, A2, \"Y\")"),
        ("B2", "=DATEDIF(A3, A4, \"D\")"),
        ("B3", "=DATEDIF(A3, A4, \"YD\")"),
        ("B4", "=DATEDIF(A3, A4, \"M\")"),
        ("B5", "=DATEDIF(A3, A4, \"YM\")"),
        ("B6", "=DATEDIF(\"2001/1/1\", \"2003/1/1\", \"Y\")"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["B1"], 2.0);
    assert_eq!(values["B2"], 440.0);
    assert_eq!(values["B3"], 75.0);
    assert_eq!(values["B4"], 14.0);
    assert_eq!(values["B5"], 2.0);
    assert_eq!(values["B6"], 2.0);
}

#[test]
fn datedif_rejects_reversed_dates() {
    let cells = [("A1", "=DATEDIF(DATE(2020, 1, 2), DATE(2020, 1, 1), \"D\")")];

    assert_eq!(
        calculate_spreadsheet(&cells),
        Err(SpreadsheetError::Num)
    );
}

#[test]
fn date_supports_month_and_day_rollover() {
    let cells = [
        ("A1", "=DATE(2024, 13, 1)"),
        ("A2", "=DATE(2024, 1, 32)"),
        ("A3", "=YEAR(A1)"),
        ("A4", "=MONTH(A1)"),
        ("A5", "=MONTH(A2)"),
        ("A6", "=DAY(A2)"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["A3"], 2025.0);
    assert_eq!(values["A4"], 1.0);
    assert_eq!(values["A5"], 2.0);
    assert_eq!(values["A6"], 1.0);
}

#[test]
fn datevalue_rejects_invalid_dates_but_date_can_rollover() {
    let invalid = [("A1", "=DATEVALUE(\"2024-01-32\")")];
    assert_eq!(
        calculate_spreadsheet(&invalid),
        Err(SpreadsheetError::Value)
    );

    let rolled = [("A1", "=DATE(2024, 1, 32)"), ("A2", "=DAY(A1)")];
    let values = calculate_spreadsheet(&rolled).unwrap();
    assert_eq!(values["A2"], 1.0);
}

#[test]
fn days_can_be_negative_and_ignores_time_fraction() {
    let cells = [
        ("A1", "=DATE(2011, 3, 15)"),
        ("A2", "=DATE(2011, 2, 1)"),
        ("B1", "=DAYS(A2, A1)"),
        ("B2", "=YEAR(39448.75)"),
        ("B3", "=DAY(39448.75)"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["B1"], -42.0);
    assert_eq!(values["B2"], 2008.0);
    assert_eq!(values["B3"], 1.0);
}

#[test]
fn datedif_supports_md_and_unit_cell_reference() {
    let cells = [
        ("A1", "=DATE(2007, 1, 1)"),
        ("A2", "=DATE(2007, 1, 31)"),
        ("U1", "\"Y\""),
        ("B1", "=DATEDIF(A1, A2, \"MD\")"),
        ("B2", "=DATEDIF(DATE(2001, 1, 1), DATE(2003, 1, 1), U1)"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["U1"], CellValue::Text("Y".to_string()));
    assert_eq!(values["B1"], 30.0);
    assert_eq!(values["B2"], 2.0);
}

#[test]
fn supports_excel_style_comparison_operators() {
    let cells = [
        ("A1", "10"),
        ("B1", "=IF(A1 = 10, 1, 0)"),
        ("B2", "=IF(A1 <> 10, 1, 0)"),
        ("B3", "=IF(A1 == 10, 1, 0)"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 0.0);
    assert_eq!(values["B3"], 1.0);
}

#[test]
fn huge_date_serial_is_rejected_without_hanging() {
    let cells = [("A1", "=YEAR(1e308)")];
    assert_eq!(
        calculate_spreadsheet(&cells),
        Err(SpreadsheetError::Num)
    );
}

#[test]
fn datevalue_supports_japanese_and_dmy_in_formulas() {
    let cells = [
        ("A1", "=DATEVALUE(\"15/1/2008\")"),
        ("A2", "=DATEVALUE(\"2008年1月15日\")"),
        ("A3", "=DATEVALUE(\"令和6年4月1日\")"),
        ("B1", "=DAY(A1)"),
        ("B2", "=DAY(A2)"),
        ("B3", "=YEAR(A3)"),
    ];

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["B1"], 15.0);
    assert_eq!(values["B2"], 15.0);
    assert_eq!(values["B3"], 2024.0);
}

#[test]
fn datedif_md_can_be_negative_like_excel() {
    let cells = [("A1", "=DATEDIF(DATE(2007, 1, 31), DATE(2007, 3, 1), \"MD\")")];
    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["A1"], -2.0);
}

#[test]
fn evaluates_many_independent_cells_in_parallel_path() {
    // At/above PARALLEL_CELL_THRESHOLD (32), layers use rayon when len > 1.
    let owned: Vec<(String, String)> = (0..40)
        .map(|i| (format!("C{i}"), format!("{}", i * 10)))
        .collect();
    let cells: Vec<(&str, &str)> = owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values.len(), 40);
    assert_eq!(values["C0"], 0.0);
    assert_eq!(values["C39"], 390.0);
}

#[test]
fn parallel_path_respects_dependency_layers() {
    let mut owned: Vec<(String, String)> = (0..32)
        .map(|i| (format!("A{i}"), "1".to_string()))
        .collect();
    owned.push(("B1".to_string(), "=A0 + A1 + A31".to_string()));
    let cells: Vec<(&str, &str)> = owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let values = calculate_spreadsheet(&cells).unwrap();
    assert_eq!(values["B1"], 3.0);
}
