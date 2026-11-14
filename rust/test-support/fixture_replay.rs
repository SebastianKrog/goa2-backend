//! Shared replay accounting for differential and exhaustive restore tests.
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

type ReplayKey = [u8; 32];
type VerifiedReplay = (String, usize);

struct CoverageRow {
    name: String,
    operations: usize,
    fixture_sha256: String,
    key: Option<ReplayKey>,
    representative: String,
}

#[derive(Default)]
pub struct ReplayCoverage {
    share: bool,
    report: bool,
    verified: Mutex<HashMap<ReplayKey, Arc<OnceLock<VerifiedReplay>>>>,
    rows: Mutex<Vec<CoverageRow>>,
    replays: AtomicUsize,
}

impl ReplayCoverage {
    pub fn from_environment() -> Self {
        Self {
            share: std::env::var_os("GOA2_SHARE_IDENTICAL_TRACES").is_some(),
            report: std::env::var_os("GOA2_TRACE_COVERAGE").is_some(),
            ..Self::default()
        }
    }

    pub fn enabled(&self) -> bool {
        self.share || self.report
    }

    pub fn verify(&self, name: String, data: &[u8], replay: impl FnOnce() -> usize) {
        let key = payload_key(data, &name);
        let run = || {
            let operations = replay();
            self.replays.fetch_add(1, Ordering::Relaxed);
            (name.clone(), operations)
        };
        let (representative, operations) = if self.share
            && let Some(key) = key
        {
            let proof = self
                .verified
                .lock()
                .unwrap()
                .entry(key)
                .or_default()
                .clone();
            proof.get_or_init(run).clone()
        } else {
            run()
        };
        if self.report {
            self.rows.lock().unwrap().push(CoverageRow {
                name,
                operations,
                fixture_sha256: format!("{:x}", Sha256::digest(data)),
                key,
                representative,
            });
        }
    }

    pub fn finish(self, gate: &str, checked: usize) {
        if self.report {
            crate::fixture_cache::report_decode_stats();
            let mut rows = self.rows.into_inner().unwrap();
            rows.sort_unstable_by(|left, right| left.name.cmp(&right.name));
            assert_eq!(rows.len(), checked);
            for row in rows {
                eprintln!(
                    "trace-coverage {}",
                    serde_json::json!({
                        "index": gate, "case": row.name, "operations": row.operations,
                        "fixture_sha256": row.fixture_sha256, "replay_key": row.key,
                        "replayed_as": row.representative,
                    })
                );
            }
        }
        if self.share {
            let count = self.replays.load(Ordering::Relaxed);
            assert!((1..=checked).contains(&count));
            eprintln!("{gate}: checked {checked} named cases in {count} replays");
        }
    }
}

pub fn payload_key(data: &[u8], name: &str) -> Option<ReplayKey> {
    let prefix = format!(r#"{{"name":{},"#, serde_json::to_string(name).ok()?);
    Some(Sha256::digest(data.strip_prefix(prefix.as_bytes())?).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_only_reuse_preserves_order_and_alias_assertions() {
        let baseline = br#"{"name":"baseline","events":["move","attack"],"aliases":[[0,1]]}"#;
        let duplicate = br#"{"name":"duplicate","events":["move","attack"],"aliases":[[0,1]]}"#;
        let reordered = br#"{"name":"reordered","events":["attack","move"],"aliases":[[0,1]]}"#;
        let split = br#"{"name":"split","events":["move","attack"],"aliases":[[0],[1]]}"#;

        for share in [false, true] {
            let coverage = ReplayCoverage {
                share,
                ..ReplayCoverage::default()
            };
            let verify = |name: &str, payload: &[u8]| {
                coverage.verify(name.to_owned(), payload, || {
                    let record: serde_json::Value = serde_json::from_slice(payload).unwrap();
                    assert_eq!(record["events"], serde_json::json!(["move", "attack"]));
                    assert_eq!(record["aliases"], serde_json::json!([[0, 1]]));
                    1
                });
            };
            verify("baseline", baseline);
            verify("duplicate", duplicate);
            assert_eq!(
                coverage.replays.load(Ordering::Relaxed),
                if share { 1 } else { 2 }
            );
            assert!(std::panic::catch_unwind(|| verify("reordered", reordered)).is_err());
            assert!(std::panic::catch_unwind(|| verify("split", split)).is_err());
        }
    }
}
