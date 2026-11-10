//! Current native boundary identities. Historical migrations and frozen payloads
//! retain their original literal versions; these constants do not upgrade data.
#![warn(missing_docs)]
/// Admitted Python source revision; keep the codec's existing public alias.
pub const ORACLE_REVISION: &str = "00becf4a68615afa410f8eabe0c841809af0427b";
/// Lossless state envelope schema.
pub const STATE_SCHEMA: u32 = 1;
/// Admitted gameplay rules version.
pub const RULES: u32 = 9;
/// Native continuation metadata version.
pub const NATIVE_RESUME: u32 = 1;
/// Authoritative game checkpoint version.
pub const CHECKPOINT: u32 = 1;
/// Static setup-content pack format.
pub const CONTENT: u32 = 1;
/// JSON-lines worker protocol.
pub const WORKER_SCHEMA: u32 = 1;

/// Return the embedded, hash-admitted oracle identity metadata.
/// Parsing failure is an error, never a default identity or an accepted repin.
pub fn oracle_identity() -> Result<&'static serde_json::Value, String> {
    crate::codec::oracle_manifest()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admitted_identity_agrees_with_constants_without_upgrading_payloads() {
        let identity = oracle_identity().unwrap();
        assert_eq!(identity["oracle_revision"], ORACLE_REVISION);
        assert_eq!(identity["rules_version"], RULES);
        assert_eq!(
            (
                STATE_SCHEMA,
                NATIVE_RESUME,
                CHECKPOINT,
                CONTENT,
                WORKER_SCHEMA
            ),
            (1, 1, 1, 1, 1)
        );
        assert_eq!(identity["source_sha256"].as_str().unwrap().len(), 64);
    }
}
