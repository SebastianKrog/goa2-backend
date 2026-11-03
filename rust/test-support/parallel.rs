// Test-only, bounded fixture execution. Sessions and case snapshots stay private.
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

fn workers() -> usize {
    let requested = std::env::var("GOA2_ORACLE_WORKERS").ok().map_or(4, |raw| {
        let count: usize = raw.parse().expect("GOA2_ORACLE_WORKERS must be 1..=4");
        assert!(
            (1..=4).contains(&count),
            "GOA2_ORACLE_WORKERS must be 1..=4"
        );
        count
    });
    requested.min(std::thread::available_parallelism().map_or(1, usize::from))
}

pub fn for_each_case<T: Send>(
    cases: impl Iterator<Item = T> + Send,
    verify: impl Fn(T) + Sync,
) -> usize {
    let workers = workers();
    if workers == 1 {
        let mut checked = 0;
        for case in cases {
            verify(case);
            checked += 1;
        }
        return checked;
    }
    let cases = Mutex::new(cases);
    let checked = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let case = { cases.lock().expect("fixture iterator poisoned").next() };
                    let Some(case) = case else { break };
                    verify(case);
                    checked.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
    checked.load(Ordering::Relaxed)
}
