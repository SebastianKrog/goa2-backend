//! Process-wide immutable frozen templates. Session programs clone only the
//! individual records they bind, preserving independent mutable continuations.
use serde_json::Value;
use std::sync::OnceLock;
static INSTRUCTIONS: OnceLock<Result<Value, String>> = OnceLock::new();

pub fn instructions() -> Result<&'static Value, String> {
    INSTRUCTIONS
        .get_or_init(|| {
            let source: Value = serde_json::from_str(include_str!(
                "../../../content/catalogs/instruction-templates.json"
            ))
            .map_err(|e| e.to_string())?;
            let manifest: Value =
                serde_json::from_str(include_str!("../../../oracle-manifest.json"))
                    .map_err(|e| e.to_string())?;
            if source["schema_version"] != 1 || source["source_sha256"] != manifest["source_sha256"]
            {
                return Err("frozen template source identity mismatch".into());
            }
            Ok(source)
        })
        .as_ref()
        .map_err(Clone::clone)
}
