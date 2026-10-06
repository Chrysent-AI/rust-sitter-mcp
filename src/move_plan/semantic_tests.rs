use super::*;
use std::sync::atomic::Ordering;

#[test]
fn controller_cancels_both_live_databases_and_joins() {
    let old = RootDatabase::default();
    let new = RootDatabase::default();
    let cancelled = AtomicBool::new(false);
    let (ready, started) = mpsc::channel();
    std::thread::scope(|threads| {
        let flag = &cancelled;
        let stop = threads.spawn(move || {
            started.recv_timeout(Duration::from_secs(2)).unwrap();
            flag.store(true, Ordering::Relaxed);
        });
        let outcome = controlled(
            &old,
            &new,
            (Instant::now() + Duration::from_secs(10), &cancelled),
            || {
                ready.send(()).unwrap();
                // Real Salsa cancellation unwinding, not an adapter preflight check.
                loop {
                    old.unwind_if_revision_cancelled();
                    new.unwind_if_revision_cancelled();
                    std::thread::yield_now();
                }
            },
        );
        stop.join().unwrap();
        assert_eq!(outcome.unwrap_err().code, "CANCELLED");
        assert!(old.cancellation_token().is_cancelled());
        assert!(new.cancellation_token().is_cancelled());
    });
}

#[test]
fn controller_deadline_is_reported_as_incomplete_planning_not_success() {
    let old = RootDatabase::default();
    let new = RootDatabase::default();
    let cancelled = AtomicBool::new(false);
    let outcome = controlled(
        &old,
        &new,
        (Instant::now() + Duration::from_millis(20), &cancelled),
        || {
            loop {
                new.unwind_if_revision_cancelled();
                std::thread::yield_now();
            }
        },
    );
    assert_eq!(outcome.unwrap_err().code, "planning_deadline");
}

#[test]
fn non_cancellation_panic_still_stops_and_joins_the_controller() {
    let old = RootDatabase::default();
    let new = RootDatabase::default();
    let cancelled = AtomicBool::new(false);
    let outcome = std::panic::catch_unwind(|| {
        controlled(
            &old,
            &new,
            (Instant::now() + Duration::from_secs(10), &cancelled),
            || {
                panic!("test programmer error");
            },
        )
    });
    assert!(outcome.is_err());
    assert!(!old.cancellation_token().is_cancelled());
    assert!(!new.cancellation_token().is_cancelled());
}
