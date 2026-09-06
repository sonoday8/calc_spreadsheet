use calc_spreadsheet::calculate_spreadsheet;

fn main() {
    let cells = [
        ("A1", "=DATE(2020, 4, 1)"),
        ("A2", "=DATE(2024, 4, 1)"),
        ("U1", "\"Y\""),
        ("B1", "=DATEDIF(A1, A2, U1)"),
        ("B2", "=YEAR(A2)"),
        ("B3", "=DATEVALUE(\"令和6年4月1日\")"),
        ("B4", "=DATEDIF(DATE(2007, 1, 31), DATE(2007, 3, 1), \"MD\")"),
    ];

    match calculate_spreadsheet(&cells) {
        Ok(values) => {
            let mut sorted_keys: Vec<_> = values.keys().collect();
            sorted_keys.sort();
            for cell in sorted_keys {
                println!("{cell}: {}", values[cell]);
            }
        }
        Err(error) => eprintln!("error: {error}"),
    }
}
