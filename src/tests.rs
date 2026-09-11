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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;

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
        calculate_spreadsheet(&cells, CalculateOptions::default()),
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 60.0);
    assert_eq!(values["B2"], 15.0);
    assert_eq!(values["B3"], 60.0);
    assert_eq!(values["B4"], 30.0);
}

#[test]
fn multi_cell_range_spills_into_empty_cells() {
    let cells = [("A1", "1"), ("A2", "2"), ("B1", "=A1:A2")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 2.0);
}

#[test]
fn element_wise_product_sum() {
    let cells = [
        ("A1", "2"),
        ("A2", "3"),
        ("A3", "4"),
        ("B1", "10"),
        ("B2", "20"),
        ("B3", "30"),
        ("C1", "=SUM(A1:A3*B1:B3)"),
        ("D1", "=A1:A3*B1:B3"),
        ("E1", "=SUM(A1:A3*2)"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["C1"], 200.0);
    assert_eq!(values["D1"], 20.0);
    assert_eq!(values["D2"], 60.0);
    assert_eq!(values["D3"], 120.0);
    assert_eq!(values["E1"], 18.0);
}

#[test]
fn element_wise_shape_mismatch_is_value() {
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("B1", "10"),
        ("B2", "20"),
        ("B3", "30"),
        ("C1", "=SUM(A1:A2*B1:B3)"),
    ];
    assert_eq!(
        calculate_spreadsheet(&cells, CalculateOptions::default()),
        Err(SpreadsheetError::Value)
    );
}

#[test]
fn spill_blocked_by_occupied_cell() {
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("B1", "=A1:A2"),
        ("B2", "99"),
    ];
    assert_eq!(
        calculate_spreadsheet(&cells, CalculateOptions::default()),
        Err(SpreadsheetError::Spill)
    );
}

#[test]
fn spilled_values_are_readable_by_other_formulas() {
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "3"),
        ("B1", "=A1:A3"),
        ("C1", "=SUM(B1:B3)"),
        ("C2", "=B2"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 2.0);
    assert_eq!(values["B3"], 3.0);
    assert_eq!(values["C1"], 6.0);
    assert_eq!(values["C2"], 2.0);
}

#[test]
fn if_can_spill_array_branch() {
    let cells = [
        ("A1", "10"),
        ("A2", "20"),
        ("B1", "=IF(1, A1:A2, 0)"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 10.0);
    assert_eq!(values["B2"], 20.0);
}

#[test]
fn sequence_spills_grid() {
    let cells = [("A1", "=SEQUENCE(2, 3, 1, 1)")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["B1"], 2.0);
    assert_eq!(values["C1"], 3.0);
    assert_eq!(values["A2"], 4.0);
    assert_eq!(values["B2"], 5.0);
    assert_eq!(values["C2"], 6.0);
}

#[test]
fn unique_sort_filter_pipeline() {
    let cells = [
        ("A1", "3"),
        ("A2", "1"),
        ("A3", "3"),
        ("A4", "2"),
        ("B1", "=UNIQUE(A1:A4)"),
        ("C1", "=SORT(B1:B3)"),
        ("D1", "=FILTER(A1:A4, A1:A4>2)"),
        ("E1", "=FILTER(A1:A4, A1:A4>10, 0)"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    // UNIQUE keeps first occurrence: 3, 1, 2
    assert_eq!(values["B1"], 3.0);
    assert_eq!(values["B2"], 1.0);
    assert_eq!(values["B3"], 2.0);
    // SORT ascending: 1, 2, 3
    assert_eq!(values["C1"], 1.0);
    assert_eq!(values["C2"], 2.0);
    assert_eq!(values["C3"], 3.0);
    // FILTER > 2: 3, 3
    assert_eq!(values["D1"], 3.0);
    assert_eq!(values["D2"], 3.0);
    assert!(!values.contains_key("D3"));
    // empty FILTER with if_empty
    assert_eq!(values["E1"], 0.0);
}

#[test]
fn filter_empty_without_fallback_is_calc() {
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("B1", "=FILTER(A1:A2, A1:A2>10)"),
    ];
    assert_eq!(
        calculate_spreadsheet(&cells, CalculateOptions::default()),
        Err(SpreadsheetError::Calc)
    );
}

#[test]
fn sort_descending_by_column() {
    let cells = [
        ("A1", "10"),
        ("B1", "1"),
        ("A2", "30"),
        ("B2", "2"),
        ("A3", "20"),
        ("B3", "3"),
        ("D1", "=SORT(A1:B3, 1, -1)"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["D1"], 30.0);
    assert_eq!(values["E1"], 2.0);
    assert_eq!(values["D2"], 20.0);
    assert_eq!(values["E2"], 3.0);
    assert_eq!(values["D3"], 10.0);
    assert_eq!(values["E3"], 1.0);
}

#[test]
fn spill_ref_operator_copies_spill() {
    let cells = [
        ("A1", "=SEQUENCE(3)"),
        ("B1", "=A1#"),
        ("C1", "=SUM(A1#)"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["A3"], 3.0);
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 2.0);
    assert_eq!(values["B3"], 3.0);
    assert_eq!(values["C1"], 6.0);
}

#[test]
fn implicit_intersection_at_operator() {
    let cells = [
        ("A1", "10"),
        ("A2", "20"),
        ("A3", "30"),
        ("B1", "1"),
        ("B2", "2"),
        ("B3", "3"),
        ("C2", "=@A1:A3"),
        ("D1", "=@B1:B3"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["C2"], 20.0); // same row as C2 → A2
    assert_eq!(values["D1"], 1.0); // same row as D1 → B1
}

#[test]
fn broadcast_row_times_column() {
    let cells = [
        ("A1", "1"),
        ("B1", "2"),
        ("C1", "3"),
        ("D1", "10"),
        ("D2", "20"),
        ("D3", "30"),
        ("E1", "=A1:C1*D1:D3"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["E1"], 10.0);
    assert_eq!(values["F1"], 20.0);
    assert_eq!(values["G1"], 30.0);
    assert_eq!(values["E2"], 20.0);
    assert_eq!(values["F2"], 40.0);
    assert_eq!(values["G2"], 60.0);
    assert_eq!(values["E3"], 30.0);
    assert_eq!(values["F3"], 60.0);
    assert_eq!(values["G3"], 90.0);
}

#[test]
fn oversized_range_is_num() {
    let cells = [("Out", "=SUM(A1:ZZ9000)")];
    assert_eq!(calculate_spreadsheet(&cells, CalculateOptions::default()), Err(SpreadsheetError::Num));
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
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
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
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 3.0);
    assert_eq!(values["B2"], 2.0);
    assert_eq!(values["B3"], -1.0);
    assert_eq!(values["B4"], 17.0);
}

#[test]
fn average_without_args_is_division_by_zero() {
    let cells = [("A1", "=AVERAGE()")];

    assert_eq!(
        calculate_spreadsheet(&cells, CalculateOptions::default()),
        Err(SpreadsheetError::DivisionByZero)
    );
}

#[test]
fn sqrt_rejects_negative_values() {
    let cells = [("A1", "=SQRT(-1)")];

    assert_eq!(calculate_spreadsheet(&cells, CalculateOptions::default()), Err(SpreadsheetError::Num));
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
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
        calculate_spreadsheet(&cells, CalculateOptions::default()),
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A3"], 2025.0);
    assert_eq!(values["A4"], 1.0);
    assert_eq!(values["A5"], 2.0);
    assert_eq!(values["A6"], 1.0);
}

#[test]
fn datevalue_rejects_invalid_dates_but_date_can_rollover() {
    let invalid = [("A1", "=DATEVALUE(\"2024-01-32\")")];
    assert_eq!(
        calculate_spreadsheet(&invalid, CalculateOptions::default()),
        Err(SpreadsheetError::Value)
    );

    let rolled = [("A1", "=DATE(2024, 1, 32)"), ("A2", "=DAY(A1)")];
    let values = calculate_spreadsheet(&rolled, CalculateOptions::default()).unwrap().values;
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 0.0);
    assert_eq!(values["B3"], 1.0);
}

#[test]
fn huge_date_serial_is_rejected_without_hanging() {
    let cells = [("A1", "=YEAR(1e308)")];
    assert_eq!(
        calculate_spreadsheet(&cells, CalculateOptions::default()),
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 15.0);
    assert_eq!(values["B2"], 15.0);
    assert_eq!(values["B3"], 2024.0);
}

#[test]
fn datedif_md_can_be_negative_like_excel() {
    let cells = [("A1", "=DATEDIF(DATE(2007, 1, 31), DATE(2007, 3, 1), \"MD\")")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
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

    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 3.0);
}

#[test]
fn unique_exactly_once_and_by_col() {
    let cells = [
        ("A1", "3"),
        ("A2", "1"),
        ("A3", "3"),
        ("A4", "2"),
        ("B1", "=UNIQUE(A1:A4, 0, 1)"),
        ("C1", "1"),
        ("D1", "1"),
        ("E1", "2"),
        ("C2", "10"),
        ("D2", "10"),
        ("E2", "20"),
        ("F1", "=UNIQUE(C1:E2, 1)"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    // exactly_once: 1 and 2 (3 appears twice)
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 2.0);
    assert!(!values.contains_key("B3"));
    // by_col: columns [1,10] and [2,20] (duplicate [1,10] dropped)
    assert_eq!(values["F1"], 1.0);
    assert_eq!(values["G1"], 2.0);
    assert_eq!(values["F2"], 10.0);
    assert_eq!(values["G2"], 20.0);
}

#[test]
fn sort_by_col_reorders_columns() {
    let cells = [
        ("A1", "3"),
        ("B1", "1"),
        ("C1", "2"),
        ("A2", "30"),
        ("B2", "10"),
        ("C2", "20"),
        ("D1", "=SORT(A1:C2, 1, 1, 1)"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    // Sort columns by row 1 ascending: 1, 2, 3
    assert_eq!(values["D1"], 1.0);
    assert_eq!(values["E1"], 2.0);
    assert_eq!(values["F1"], 3.0);
    assert_eq!(values["D2"], 10.0);
    assert_eq!(values["E2"], 20.0);
    assert_eq!(values["F2"], 30.0);
}

#[test]
fn filter_columns_with_row_include() {
    let cells = [
        ("A1", "1"),
        ("B1", "2"),
        ("C1", "3"),
        ("D1", "=FILTER(A1:C1, A1:C1>1)"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["D1"], 2.0);
    assert_eq!(values["E1"], 3.0);
    assert!(!values.contains_key("F1"));
}

#[test]
fn sequence_zero_rows_is_calc_and_defaults_work() {
    assert_eq!(
        calculate_spreadsheet(&[("A1", "=SEQUENCE(0)")], CalculateOptions::default()),
        Err(SpreadsheetError::Calc)
    );
    let values = calculate_spreadsheet(&[("B1", "=SEQUENCE(3)")], CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 2.0);
    assert_eq!(values["B3"], 3.0);
}

#[test]
fn implicit_intersection_out_of_range_is_value() {
    let cells = [
        ("A1", "10"),
        ("A2", "20"),
        ("A3", "30"),
        ("D5", "=@A1:A3"),
    ];
    assert_eq!(
        calculate_spreadsheet(&cells, CalculateOptions::default()),
        Err(SpreadsheetError::Value)
    );
}

#[test]
fn implicit_intersection_on_spill_ref() {
    let cells = [
        ("A1", "=SEQUENCE(3)"),
        ("B2", "=@A1#"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B2"], 2.0);
}

#[test]
fn two_spills_into_same_cell_is_spill_error() {
    // A2 spills into A2:B2; B1 spills into B1:B2 → conflict on B2.
    let cells = [
        ("A2", "=SEQUENCE(1, 2)"),
        ("B1", "=SEQUENCE(2)"),
    ];
    assert_eq!(
        calculate_spreadsheet(&cells, CalculateOptions::default()),
        Err(SpreadsheetError::Spill)
    );
}

#[test]
fn non_a1_anchor_does_not_spill_geometrically() {
    let cells = [("Foo", "=SEQUENCE(3)")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["Foo"], 1.0);
    assert!(!values.contains_key("Foo2"));
    assert_eq!(values.len(), 1);
}

#[test]
fn array_division_by_zero_is_error() {
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("B1", "0"),
        ("B2", "1"),
        ("C1", "=A1:A2/B1:B2"),
    ];
    assert_eq!(
        calculate_spreadsheet(&cells, CalculateOptions::default()),
        Err(SpreadsheetError::DivisionByZero)
    );
}

#[test]
fn broadcast_column_times_row() {
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "3"),
        ("B1", "10"),
        ("C1", "20"),
        ("D1", "=A1:A3*B1:C1"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["D1"], 10.0);
    assert_eq!(values["E1"], 20.0);
    assert_eq!(values["D2"], 20.0);
    assert_eq!(values["E2"], 40.0);
    assert_eq!(values["D3"], 30.0);
    assert_eq!(values["E3"], 60.0);
}

#[test]
fn unknown_cell_and_invalid_formula_errors() {
    // Missing cells coerce to 0 (Excel-like); typos in function names still error.
    let values = calculate_spreadsheet(&[("A1", "=Z99"), ("A2", "=Z99+1")], CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 0.0);
    assert_eq!(values["A2"], 1.0);
    assert!(matches!(
        calculate_spreadsheet(&[("A1", "=((((")], CalculateOptions::default()),
        Err(SpreadsheetError::InvalidFormula(_))
    ));
    assert!(matches!(
        calculate_spreadsheet(&[("A1", "=NO_SUCH_FUNC(1)")], CalculateOptions::default()),
        Err(SpreadsheetError::InvalidFormula(_))
    ));
}

#[test]
fn iferror_can_spill_array_result() {
    let cells = [
        ("A1", "10"),
        ("A2", "20"),
        ("B1", "=IFERROR(A1:A2, 0)"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 10.0);
    assert_eq!(values["B2"], 20.0);
}

#[test]
fn count_counts_elements_of_array_expression() {
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "3"),
        ("B1", "=COUNT(A1:A3*2)"),
        ("B2", "=COUNTA(A1:A3*2)"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 3.0);
    assert_eq!(values["B2"], 3.0);
}

#[test]
fn filter_bad_include_shape_is_value() {
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "3"),
        ("B1", "1"),
        ("B2", "0"),
        ("C1", "=FILTER(A1:A3, B1:B2)"),
    ];
    assert_eq!(
        calculate_spreadsheet(&cells, CalculateOptions::default()),
        Err(SpreadsheetError::Value)
    );
}

#[test]
fn if_unused_array_arm_does_not_create_false_cycle() {
    // Const-false arm is pruned from refs and spill shape, so A1 does not spill
    // into A2 and B1→A1 is not required (A2 stays blank → 0).
    let cells = [("A1", "=IF(1, B1, SEQUENCE(2))"), ("B1", "=A2+5")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 5.0);
    assert_eq!(values["A1"], 5.0);
    assert!(!values.contains_key("A2"));
}

#[test]
fn if_taken_array_arm_spill_is_readable_by_dependents() {
    let cells = [
        ("A1", "=IF(1, SEQUENCE(3), 0)"),
        ("B1", "=A2"),
        ("C1", "=SUM(A1:A3)"),
        ("D1", "=IFERROR(SEQUENCE(2), 0)"),
        ("E1", "=D2"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["A3"], 3.0);
    assert_eq!(values["B1"], 2.0);
    assert_eq!(values["C1"], 6.0);
    assert_eq!(values["D1"], 1.0);
    assert_eq!(values["D2"], 2.0);
    assert_eq!(values["E1"], 2.0);
}

#[test]
fn if_taken_spill_with_unused_arm_ref_still_spills() {
    // Complementary pattern: taken SEQUENCE arm must spill; unused B1 ref is pruned.
    let cells = [("A1", "=IF(1, SEQUENCE(2), B1)"), ("B1", "=A2")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["B1"], 2.0);
}

#[test]
fn if_const_cell_condition_prunes_unused_arm() {
    let cells = [
        ("C1", "1"),
        ("A1", "=IF(C1, SEQUENCE(2), B1)"),
        ("B1", "=A2"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["B1"], 2.0);
}

#[test]
fn if_unused_scalar_arm_ref_does_not_create_false_cycle() {
    let cells = [("A1", "=IF(1, 5, B1)"), ("B1", "=A1")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 5.0);
    assert_eq!(values["B1"], 5.0);
}

#[test]
fn sequence_size_from_const_cell_registers_spill_deps() {
    let cells = [
        ("N1", "3"),
        ("A1", "=SEQUENCE(N1)"),
        ("B1", "=SUM(A1:A3)"),
        ("C1", "=A2"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["A3"], 3.0);
    assert_eq!(values["B1"], 6.0);
    assert_eq!(values["C1"], 2.0);
}

#[test]
fn sequence_size_from_binop_registers_spill_deps() {
    let cells = [("A1", "=SEQUENCE(1+2)"), ("B1", "=A2"), ("C1", "=SUM(A1:A3)")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["A3"], 3.0);
    assert_eq!(values["B1"], 2.0);
    assert_eq!(values["C1"], 6.0);
}

#[test]
fn sequence_size_from_blank_plus_const_registers_spill_deps() {
    let cells = [("A1", "=SEQUENCE(Z1+2)"), ("B1", "=A2")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["B1"], 2.0);
}

#[test]
fn if_binop_condition_prunes_unused_arm() {
    let cells = [("A1", "=IF(1+0, SEQUENCE(2), B1)"), ("B1", "=A2")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["B1"], 2.0);
}

#[test]
fn iferror_known_primary_spill_is_readable() {
    let cells = [("A1", "=IFERROR(SEQUENCE(2), B1)"), ("B1", "=A2")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["B1"], 2.0);
}

#[test]
fn sequence_size_from_sum_registers_spill_deps() {
    let cells = [
        ("N1", "3"),
        ("A1", "=SEQUENCE(SUM(N1))"),
        ("B1", "=A2"),
        ("C1", "=SUM(A1:A3)"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["A3"], 3.0);
    assert_eq!(values["B1"], 2.0);
    assert_eq!(values["C1"], 6.0);
}

#[test]
fn if_and_condition_prunes_unused_arm() {
    let cells = [("A1", "=IF(AND(1, 1), SEQUENCE(2), B1)"), ("B1", "=A2")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["B1"], 2.0);
}

#[test]
fn if_sum_cell_condition_prunes_unused_arm() {
    let cells = [
        ("F1", "1"),
        ("E1", "=SUM(F1)"),
        ("A1", "=IF(E1, SEQUENCE(2), B1)"),
        ("B1", "=A2"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["B1"], 2.0);
}

#[test]
fn filter_if_empty_ref_does_not_create_false_cycle() {
    // if_empty is lazy; collecting C1 would cycle with B1's spill into B2.
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "3"),
        ("B1", "=FILTER(A1:A3, A1:A3>0, C1)"),
        ("C1", "=B2"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 2.0);
    assert_eq!(values["B3"], 3.0);
    assert_eq!(values["C1"], 2.0);
}

#[test]
fn product_skips_blanks_in_const_fold_for_if_spill() {
    // Blank A2 must not make PRODUCT fold to 0 and prune the SEQUENCE arm.
    let cells = [
        ("A1", "2"),
        ("A3", "3"),
        ("X1", "=IF(PRODUCT(A1:A3), SEQUENCE(2), Y1)"),
        ("Y1", "=X2"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["X1"], 1.0);
    assert_eq!(values["X2"], 2.0);
    assert_eq!(values["Y1"], 2.0);
}

#[test]
fn sequence_size_from_count_registers_spill_deps() {
    let cells = [
        ("N1", "1"),
        ("N2", "2"),
        ("N3", "3"),
        ("A1", "=SEQUENCE(COUNT(N1:N3))"),
        ("B1", "=A2"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["A3"], 3.0);
    assert_eq!(values["B1"], 2.0);
}

#[test]
fn filter_provably_empty_uses_if_empty_spill() {
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("B1", "=FILTER(A1:A2, A1:A2>10, SEQUENCE(3))"),
        ("C1", "=B2"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 2.0);
    assert_eq!(values["B3"], 3.0);
    assert_eq!(values["C1"], 2.0);
}

#[test]
fn filter_unknown_empty_large_if_empty_spill_readable() {
    // ROUND is not const-folded → include emptiness is unknown statically.
    // Runtime empty + large if_empty SEQUENCE must still feed distant readers.
    let cells = [
        ("K1", "=ROUND(99.6, 0)"),
        ("A1", "1"),
        ("A2", "2"),
        ("B1", "=FILTER(A1:A2, A1:A2>K1, SEQUENCE(5))"),
        ("C1", "=B5"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["K1"], 100.0);
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B5"], 5.0);
    assert_eq!(values["C1"], 5.0);
}

#[test]
fn filter_unknown_if_empty_self_spill_no_false_cycle() {
    // ROUND keeps include non-foldable; non-empty path with if_empty → self-spill reader.
    let cells = [
        ("K1", "=ROUND(0.4, 0)"),
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "3"),
        ("B1", "=FILTER(A1:A3, A1:A3>K1, C1)"),
        ("C1", "=B2"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["K1"], 0.0);
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 2.0);
    assert_eq!(values["B3"], 3.0);
    assert_eq!(values["C1"], 2.0);
}

#[test]
fn filter_soft_skip_updates_transitive_dependents() {
    // Include range references C1=B2 → spill edge would cycle; soft-skip + fixup must
    // refresh C1 and E1=C1*2 (not only the soft-watched reader).
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "3"),
        ("A4", "4"),
        ("A5", "5"),
        ("C1", "=B2"),
        ("C2", "1"),
        ("C3", "1"),
        ("C4", "1"),
        ("C5", "1"),
        ("B1", "=FILTER(A1:A5, C1:C5)"),
        ("E1", "=C1*2"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 2.0);
    assert_eq!(values["B3"], 3.0);
    assert_eq!(values["B4"], 4.0);
    assert_eq!(values["B5"], 5.0);
    assert_eq!(values["C1"], 2.0);
    assert_eq!(values["E1"], 4.0);
}

#[test]
fn iferror_filter_soft_skip_include_self_spill() {
    // Wrapped FILTER must soft-skip the same include↔spill cycle as bare FILTER.
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "3"),
        ("A4", "4"),
        ("A5", "5"),
        ("C1", "=B2"),
        ("C2", "1"),
        ("C3", "1"),
        ("C4", "1"),
        ("C5", "1"),
        ("B1", "=IFERROR(FILTER(A1:A5, C1:C5), 0)"),
        ("E1", "=C1*2"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 2.0);
    assert_eq!(values["B5"], 5.0);
    assert_eq!(values["C1"], 2.0);
    assert_eq!(values["E1"], 4.0);
}

#[test]
fn nested_iferror_filter_soft_skip() {
    // First pass keeps rows 2–3 (C1 blank→0); fixup then includes row 1 once C1=B2.
    let cells = [
        ("A1", "10"),
        ("A2", "20"),
        ("A3", "30"),
        ("C1", "=B2"),
        ("C2", "1"),
        ("C3", "1"),
        ("B1", "=IFERROR(IFERROR(FILTER(A1:A3, C1:C3), -1), -2)"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 10.0);
    assert_eq!(values["B2"], 20.0);
    assert_eq!(values["B3"], 30.0);
    assert_eq!(values["C1"], 20.0);
}

#[test]
fn if_wrapped_filter_soft_skip() {
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "3"),
        ("C1", "=B2"),
        ("C2", "1"),
        ("C3", "1"),
        ("B1", "=IF(1, FILTER(A1:A3, C1:C3), 0)"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 2.0);
    assert_eq!(values["B3"], 3.0);
    assert_eq!(values["C1"], 2.0);
}

#[test]
fn iferror_sequence_spill_cycle_still_circular() {
    // Soft-skip is FILTER-only; SEQUENCE under IFERROR/IF stays fail-closed.
    let cells = [
        ("B1", "=IFERROR(IF(C1, SEQUENCE(2), SEQUENCE(2)), 0)"),
        ("C1", "=B2"),
    ];
    assert!(matches!(
        calculate_spreadsheet(&cells, CalculateOptions::default()),
        Err(SpreadsheetError::CircularReference(_))
    ));
}

#[test]
fn dead_filter_arm_does_not_soft_skip_sequence_cycle() {
    // Const-false IF: FILTER is dead; live SEQUENCE arm still refs C1 → fail-closed CR.
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("C1", "=B2"),
        ("C2", "1"),
        (
            "B1",
            "=IF(0, FILTER(A1:A2, C1:C2), IF(C1, SEQUENCE(2), SEQUENCE(2)))",
        ),
    ];
    assert!(matches!(
        calculate_spreadsheet(&cells, CalculateOptions::default()),
        Err(SpreadsheetError::CircularReference(_))
    ));
}

#[test]
fn iferror_fallback_filter_ignored_when_primary_spills() {
    // Primary spills SEQUENCE and refs C1; fallback FILTER must not enable soft-skip.
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("C1", "=B2"),
        ("C2", "1"),
        (
            "B1",
            "=IFERROR(IF(C1, SEQUENCE(2), SEQUENCE(2)), FILTER(A1:A2, C1:C2))",
        ),
    ];
    assert!(matches!(
        calculate_spreadsheet(&cells, CalculateOptions::default()),
        Err(SpreadsheetError::CircularReference(_))
    ));
}

#[test]
fn filter_soft_skip_long_dependent_chain() {
    // Fixup pass bound scales with wave size; a long explicit chain must still refresh.
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "3"),
        ("A4", "4"),
        ("A5", "5"),
        ("C1", "=B2"),
        ("C2", "1"),
        ("C3", "1"),
        ("C4", "1"),
        ("C5", "1"),
        ("B1", "=FILTER(A1:A5, C1:C5)"),
        ("D1", "=C1"),
        ("D2", "=D1"),
        ("D3", "=D2"),
        ("D4", "=D3"),
        ("D5", "=D4"),
        ("D6", "=D5*2"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["C1"], 2.0);
    assert_eq!(values["D6"], 4.0);
}

#[test]
fn spill_into_occupied_does_not_partial_write_anchor() {
    // #SPILL! must not leave a half-applied footprint (validate-before-write).
    let cells = [("A1", "=SEQUENCE(3)"), ("A3", "9")];
    assert!(matches!(
        calculate_spreadsheet(&cells, CalculateOptions::default()),
        Err(SpreadsheetError::Spill)
    ));
}

#[test]
fn let_binds_locals_and_spills() {
    let cells = [("A1", "=LET(n, 3, SEQUENCE(n))"), ("B1", "=A2")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["A3"], 3.0);
    assert_eq!(values["B1"], 2.0);
}

#[test]
fn let_wrapped_filter_soft_skip() {
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "3"),
        ("C1", "=B2"),
        ("C2", "1"),
        ("C3", "1"),
        ("B1", "=LET(data, A1:A3, FILTER(data, C1:C3))"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 2.0);
    assert_eq!(values["B3"], 3.0);
    assert_eq!(values["C1"], 2.0);
}

#[test]
fn let_nested_in_iferror_filter_soft_skip() {
    let cells = [
        ("A1", "10"),
        ("A2", "20"),
        ("A3", "30"),
        ("C1", "=B2"),
        ("C2", "1"),
        ("C3", "1"),
        (
            "B1",
            "=IFERROR(LET(src, A1:A3, FILTER(src, C1:C3)), -1)",
        ),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 10.0);
    assert_eq!(values["B2"], 20.0);
    assert_eq!(values["B3"], 30.0);
    assert_eq!(values["C1"], 20.0);
}

#[test]
fn filter_inside_binop_does_not_soft_skip() {
    // Soft-skip is only when FILTER is the formula root (or passthrough wrapper).
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("C1", "=B2"),
        ("C2", "1"),
        ("B1", "=SEQUENCE(2)+FILTER(A1:A2, C1:C2)"),
    ];
    assert!(matches!(
        calculate_spreadsheet(&cells, CalculateOptions::default()),
        Err(SpreadsheetError::CircularReference(_))
    ));
}

#[test]
fn let_binding_name_does_not_create_false_cycle_with_spill_reader() {
    // Local name Z1 must not be treated as a sheet ref to Z1=Y2.
    let cells = [("Z1", "=Y2"), ("Y1", "=LET(Z1, 2, SEQUENCE(Z1))")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["Y1"], 1.0);
    assert_eq!(values["Y2"], 2.0);
    assert_eq!(values["Z1"], 2.0);
}

#[test]
fn let_later_value_uses_earlier_binding_not_sheet_cell() {
    let cells = [
        ("X1", "100"),
        ("A1", "=LET(x, 3, y, x+1, SEQUENCE(y))"),
        ("B1", "=A3"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["A3"], 3.0);
    assert_eq!(values["A4"], 4.0);
    assert_eq!(values["B1"], 3.0);
}

#[test]
fn let_bound_name_does_not_inflate_analyze_work() {
    // Binding token `n` in the calculation must not add work (parallel threshold).
    let with_let = crate::ast::analyze_formula_refs("=LET(n, A1, SEQUENCE(n)+A2)").unwrap();
    let equivalent = crate::ast::analyze_formula_refs("=SEQUENCE(A1)+A2").unwrap();
    assert_eq!(with_let.work, equivalent.work);
    assert_eq!(with_let.cells, equivalent.cells);
}


#[test]
fn filter_shrink_with_distant_reader_no_false_cycle() {
    // Non-foldable include (MOD) so footprint stays array-sized; distant blank is 0.
    let cells = [
        ("A1", "1"),
        ("A2", "0"),
        ("A3", "0"),
        ("A4", "0"),
        ("A5", "0"),
        ("T1", "=MOD(1, 2)"),
        ("M1", "=T1"),
        ("M2", "0"),
        ("M3", "0"),
        ("M4", "0"),
        ("M5", "0"),
        ("B1", "=FILTER(A1:A5, M1:M5)"),
        ("C1", "=B1"),
        ("D1", "=B5+0"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["C1"], 1.0);
    assert_eq!(values["D1"], 0.0); // shrunk result does not write B5
}

#[test]
fn blank_cells_in_ranges_are_zero_or_skipped() {
    let cells = [
        ("A1", "10"),
        ("A3", "30"),
        ("B1", "=SUM(A1:A3)"),
        ("B2", "=A1:A3"),
        ("C1", "=A2+1"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 40.0); // blank A2 skipped in SUM
    assert_eq!(values["B2"], 10.0);
    assert_eq!(values["B3"], 0.0); // blank as 0 in array arithmetic/spill
    assert_eq!(values["B4"], 30.0);
    assert_eq!(values["C1"], 1.0); // blank A2 → 0
}

#[test]
fn cell_refs_are_case_insensitive() {
    let cells = [("A1", "=SEQUENCE(2)"), ("B1", "=a2"), ("c1", "=A1")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], 1.0);
    assert_eq!(values["A2"], 2.0);
    assert_eq!(values["B1"], 2.0);
    assert_eq!(values["C1"], 1.0);
}

#[test]
fn filter_if_empty_is_lazy() {
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("B1", "=FILTER(A1:A2, A1:A2>0, 1/0)"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 1.0);
    assert_eq!(values["B2"], 2.0);
}

#[test]
fn min_max_of_all_blanks_are_zero() {
    let cells = [("B1", "=MIN(A1:A3)"), ("B2", "=MAX(A1:A3)")];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["B1"], 0.0);
    assert_eq!(values["B2"], 0.0);
}

#[test]
fn replacements_end_to_end_match_readme_scenario() {
    let cells = [
        ("A1", "__NAME__"),
        ("B1", "NAME"),
        ("C1", "__UNKNOWN__"),
        ("D1", "=__RATE__*2"),
        ("E1", "=F1+1"),
        ("F1", "=__A1__"),
    ];
    let mut replacements = HashMap::new();
    replacements.insert("__NAME__".into(), ReplacementValue::from_text("Alice"));
    replacements.insert("__RATE__".into(), ReplacementValue::from_i64(10));
    replacements.insert("__A1__".into(), ReplacementValue::from_i64(7));

    let outcome = calculate_spreadsheet(&cells, CalculateOptions { replacements: Some(&replacements), ..Default::default() }).unwrap();
    let values = outcome.values;
    assert!(outcome.ignored_replacement_keys.is_empty());
    assert_eq!(values["A1"], CellValue::Text("Alice".into()));
    assert_eq!(values["B1"], CellValue::Text("NAME".into()));
    assert_eq!(values["C1"], CellValue::Text("__UNKNOWN__".into()));
    assert_eq!(values["D1"], 20.0);
    assert_eq!(values["E1"], 8.0);
    assert_eq!(values["F1"], 7.0);
}

#[test]
fn replacements_unknown_in_formula_coerces_like_missing_cell() {
    // Missing / non-input names coerce to 0 (same as `=Z99`), not UnknownCell.
    let cells = [("A1", "=__MISS__*2"), ("A2", "=__MISS__+1")];
    let replacements = HashMap::new();
    let values = calculate_spreadsheet(&cells, CalculateOptions { replacements: Some(&replacements), ..Default::default() })
        .unwrap()
        .values;
    assert_eq!(values["A1"], 0.0);
    assert_eq!(values["A2"], 1.0);
}

#[test]
fn replacements_skip_placeholders_inside_string_literals() {
    let mut replacements = HashMap::new();
    replacements.insert("__NAME__".into(), ReplacementValue::from_text("Alice"));
    let values = calculate_spreadsheet(
        &[
            ("A1", "=\"Hi __NAME__\""),
            ("B1", "=\"Hi \"&__NAME__"),
        ],
        CalculateOptions { replacements: Some(&replacements), ..Default::default() },
    )
    .unwrap()
    .values;
    assert_eq!(values["A1"], CellValue::Text("Hi __NAME__".into()));
    assert_eq!(values["B1"], CellValue::Text("Hi Alice".into()));
}

#[test]
fn formulas_require_leading_equals_excel_like() {
    // Without `=`, SUM(1) is text — not evaluated.
    let values = calculate_spreadsheet(
        &[("A1", "SUM(1)"), ("B1", "hello"), ("C1", "=SUM(1)")],
        CalculateOptions::default(),
    )
    .unwrap()
    .values;
    assert_eq!(values["A1"], CellValue::Text("SUM(1)".into()));
    assert_eq!(values["B1"], CellValue::Text("hello".into()));
    assert_eq!(values["C1"], 1.0);
}

#[test]
fn amp_concatenates_text_and_cells() {
    let cells = [
        ("C1", "Alice"),
        ("A1", "=\"Hi \"&C1"),
        ("A2", "=\"Hi \"&C1&\"!\""),
        ("N1", "10"),
        ("A3", "=\"x\"&N1"),
        ("A4", "=\"x\"&Z99"), // blank → empty string
        ("A5", "=N1&N1"),
    ];
    let values = calculate_spreadsheet(&cells, CalculateOptions::default()).unwrap().values;
    assert_eq!(values["A1"], CellValue::Text("Hi Alice".into()));
    assert_eq!(values["A2"], CellValue::Text("Hi Alice!".into()));
    assert_eq!(values["A3"], CellValue::Text("x10".into()));
    assert_eq!(values["A4"], CellValue::Text("x".into()));
    assert_eq!(values["A5"], CellValue::Text("1010".into()));
}

#[test]
fn amp_concat_with_placeholder_replacement() {
    let cells = [("A1", "=\"Hi \"&__NAME__"), ("B1", "=__NAME__&\" \"&__RATE__")];
    let mut replacements = HashMap::new();
    replacements.insert("__NAME__".into(), ReplacementValue::from_text("Alice"));
    replacements.insert("__RATE__".into(), ReplacementValue::from_i64(10));
    let values = calculate_spreadsheet(&cells, CalculateOptions { replacements: Some(&replacements), ..Default::default() })
        .unwrap()
        .values;
    assert_eq!(values["A1"], CellValue::Text("Hi Alice".into()));
    assert_eq!(values["B1"], CellValue::Text("Alice 10".into()));
}

#[test]
fn amp_has_lower_precedence_than_arithmetic() {
    // 1+2&3 → "33" in Excel ( (1+2) & 3 )
    let values = calculate_spreadsheet(&[("A1", "=1+2&3")], CalculateOptions::default())
        .unwrap()
        .values;
    assert_eq!(values["A1"], CellValue::Text("33".into()));
}

#[test]
fn replacements_with_custom_thresholds() {
    let cells = [("A1", "=__N__+1")];
    let mut replacements = HashMap::new();
    replacements.insert("__N__".into(), ReplacementValue::from_f64(10.0));
    let thresholds = ParallelThresholds {
        min_layer_width: usize::MAX,
        min_layer_work: usize::MAX,
    };
    let values =
        calculate_spreadsheet(&cells, CalculateOptions { replacements: Some(&replacements), thresholds: Some(thresholds) })
            .unwrap()
            .values;
    assert_eq!(values["A1"], 11.0);
}

#[test]
fn replacements_report_ignored_invalid_keys() {
    let cells = [("A1", "=__OK__+1")];
    let mut replacements = HashMap::new();
    replacements.insert("__OK__".into(), ReplacementValue::from_i64(3));
    replacements.insert("bad".into(), ReplacementValue::from_i64(9));
    replacements.insert("__name__".into(), ReplacementValue::from_text("x"));
    let outcome = calculate_spreadsheet(&cells, CalculateOptions { replacements: Some(&replacements), ..Default::default() }).unwrap();
    assert_eq!(outcome.values["A1"], 4.0);
    assert_eq!(
        outcome.ignored_replacement_keys,
        vec!["__name__".to_string(), "bad".to_string()]
    );
}
