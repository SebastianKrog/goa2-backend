use super::*;

pub(super) fn object_session(name: &str) -> NativeSession {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let index: Value = serde_json::from_reader(std::io::BufReader::new(
        crate::fixture_cache::open(base.join("object-execution.json")).unwrap(),
    ))
    .unwrap();
    let entry = index["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .unwrap();
    let case: Value = serde_json::from_reader(std::io::BufReader::new(
        crate::fixture_cache::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
    ))
    .unwrap();
    NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
}
