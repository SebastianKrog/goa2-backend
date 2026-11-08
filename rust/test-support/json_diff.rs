use serde_json::Value;

pub fn difference(actual: &Value, expected: &Value) -> Option<(String, String, String)> {
    if actual == expected {
        return None;
    }
    let within =
        |key: &str, a, b| difference(a, b).map(|(path, a, b)| (format!("/{key}{path}"), a, b));
    match (actual, expected) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().chain(b.keys()) {
                if let Some(diff) = within(
                    key,
                    a.get(key).unwrap_or(&Value::Null),
                    b.get(key).unwrap_or(&Value::Null),
                ) {
                    return Some(diff);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (index, (a, b)) in a.iter().zip(b).enumerate() {
                if let Some(diff) = within(&index.to_string(), a, b) {
                    return Some(diff);
                }
            }
        }
        _ => {}
    }
    Some((
        "".into(),
        actual.to_string().chars().take(160).collect(),
        expected.to_string().chars().take(160).collect(),
    ))
}
