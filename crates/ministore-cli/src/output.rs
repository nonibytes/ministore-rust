use serde_json::Value;
pub fn print_json(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap());
}

#[allow(dead_code)]
pub fn print_error(e: &ministore::MinistoreError) {
    eprintln!("Error: {}", e);
}
