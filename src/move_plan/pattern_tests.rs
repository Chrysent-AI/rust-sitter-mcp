//! Adversarial tests of private copied pattern identity, not compiler semantics.
use super::*;
use std::time::Duration;

fn copies(before: &str, after: &str, needle: &str, replacement: &str) -> Vec<MoveOrigin> {
    let at = before.find(needle).unwrap();
    assert_eq!(before.replacen(needle, replacement, 1), after);
    [
        (0..at, 0..at),
        (
            at + needle.len()..before.len(),
            at + replacement.len()..after.len(),
        ),
    ]
    .into_iter()
    .map(|(source, output)| origin(source, output))
    .collect()
}
fn origin(source: std::ops::Range<usize>, output: std::ops::Range<usize>) -> MoveOrigin {
    MoveOrigin {
        id: String::new(),
        source_path: "source.rs".into(),
        source_range: range(source.start, source.end),
        output_path: "target.rs".into(),
        output_range: range(output.start, output.end),
        role: "item".into(),
    }
}
fn probe(
    before: &str,
    after: &str,
    origins: Vec<MoveOrigin>,
    change: impl FnOnce(&mut Vec<items::PatternBinding>),
    controls: (Instant, &AtomicBool),
) -> Result<MoveEnvelope, DomainError> {
    let files: BTreeMap<String, FileSnapshot> = BTreeMap::from([
        (
            "lib.rs".into(),
            FileSnapshot {
                path: "lib.rs".into(),
                source: "mod source; mod target;".into(),
                mode: 0o100644,
            },
        ),
        (
            "source.rs".into(),
            FileSnapshot {
                path: "source.rs".into(),
                source: before.into(),
                mode: 0o100644,
            },
        ),
        (
            "target.rs".into(),
            FileSnapshot {
                path: "target.rs".into(),
                source: String::new(),
                mode: 0o100644,
            },
        ),
    ]);
    // Prepare source evidence with an independent live budget; the supplied
    // controls target interruption of the final audit rather than collection.
    let live = AtomicBool::new(false);
    let preparation = (Instant::now() + Duration::from_secs(30), &live);
    let mut observed = 0;
    let parsed: BTreeMap<_, _> = files
        .iter()
        .map(|(p, f)| {
            (
                p.clone(),
                items::parse(f, 0, preparation.0, preparation.1, &mut observed).unwrap(),
            )
        })
        .collect();
    let routes = items::GlobRoutes::new(&files, &parsed, "lib.rs", preparation)?;
    let item = parsed["source.rs"]
        .items
        .iter()
        .find(|i| i.name.as_deref() == Some("selected"))
        .unwrap()
        .clone();
    let mut ledger = items::pattern_bindings(
        &files,
        &parsed,
        &[("source.rs".into(), item, "target.rs".into())],
        &routes,
        preparation,
    )?;
    ledger.retain(|e| e.spelling == "binding");
    assert!(!ledger.is_empty(), "{before}");
    change(&mut ledger);
    let request: MoveRequest = serde_json::from_value(
        serde_json::json!({"repo_path":".", "crate_root":"lib.rs", "paths":["."], "moves":[]}),
    )
    .unwrap();
    let outputs = BTreeMap::from([("target.rs".into(), (after.into(), Vec::new()))]);
    let mut result = MoveEnvelope::empty(request.limits.clone().into());
    result.plan.origins = origins;
    result.select_schema(3);
    result.plan.edits = Some(Vec::new());
    result.plan.created_files = Some(Vec::new());
    result.plan.patch = Some("preview".into());
    result.plan.directory_preconditions = Some(Some(Vec::new()));
    audit(
        &request,
        &files,
        &parsed,
        &outputs,
        &[],
        &BTreeMap::new(),
        (&ledger, &BTreeMap::new()),
        controls,
        &mut result,
    )?;
    if !result.plan.blockers.is_empty() {
        result.withhold();
    }
    Ok(result)
}
fn check(
    before: &str,
    after: &str,
    origins: Vec<MoveOrigin>,
    change: impl FnOnce(&mut Vec<items::PatternBinding>),
    refused: bool,
) {
    let flag = AtomicBool::new(false);
    let result = probe(
        before,
        after,
        origins,
        change,
        (Instant::now() + Duration::from_secs(30), &flag),
    )
    .unwrap();
    assert_eq!(
        !result.plan.blockers.is_empty(),
        refused,
        "{before} → {after}: {result:?}"
    );
    assert_eq!(result.plan.integrity.semantic, "not_performed");
    if refused {
        assert!(
            result.plan.edits.is_none()
                && result.plan.created_files.is_none()
                && result.plan.patch.is_none()
        );
        assert!(matches!(result.plan.directory_preconditions, Some(None)));
        assert!(
            result
                .plan
                .blockers
                .iter()
                .all(|b| b.code == "BINDING_COLLISION")
        );
        assert!(
            result
                .plan
                .decisions
                .iter()
                .all(|d| d.reason == DecisionReason::LexicalContextUnproved
                    && d.anchors[0].expected_text == "binding")
        );
    }
}
#[test]
fn scope_repairs_outside_patterns_keep_identity() {
    for (before, needle, replacement) in [
        (
            "fn selected(binding: crate::Old) { binding; }",
            "crate::Old",
            "crate::New",
        ),
        (
            "fn selected(Record { binding }: crate::Old) { binding; }",
            "crate::Old",
            "crate::New",
        ),
        (
            "fn selected(binding: crate::Old) {}",
            "crate::Old",
            "crate::New",
        ),
        (
            "fn selected() { let binding = crate::old(); binding; }",
            "crate::old",
            "crate::new",
        ),
        (
            "fn selected() { let call = |binding: crate::Old| binding; }",
            "crate::Old",
            "crate::New",
        ),
        (
            "fn selected() { for mut binding in crate::old() { binding; } }",
            "crate::old",
            "crate::new",
        ),
        (
            "fn selected() { for mut binding in [1] { crate::old(binding); } }",
            "crate::old",
            "crate::new",
        ),
        (
            "fn selected() { match 1 { binding @ _ => crate::old(binding) } }",
            "crate::old",
            "crate::new",
        ),
    ] {
        let after = before.replacen(needle, replacement, 1);
        check(
            before,
            &after,
            copies(before, &after, needle, replacement),
            |_| {},
            false,
        );
    }
}
#[test]
fn explicit_shorthand_still_checks_competition_after_type_repair() {
    let before = "fn selected(Record { binding }: crate::Old) { binding; }";
    let repaired = before.replacen("crate::Old", "crate::New", 1);
    let after = format!("{repaired} const binding: u8 = 0;");
    let origins = copies(before, &repaired, "crate::Old", "crate::New");
    check(before, &after, origins, |_| {}, true);
}

#[test]
fn pattern_internal_repairs_still_refuse() {
    for (before, needle, replacement) in [
        (
            "fn selected() { let crate::Old(binding) = value; binding; }",
            "crate::Old",
            "crate::New",
        ),
        (
            "fn selected() { match value { binding @ _ if crate::old(binding) => binding, _ => 0 } }",
            "crate::old",
            "crate::new",
        ),
    ] {
        let after = before.replacen(needle, replacement, 1);
        check(
            before,
            &after,
            copies(before, &after, needle, replacement),
            |_| {},
            true,
        );
    }
}
#[test]
fn identifier_and_pattern_each_need_one_unchanged_unsplit_origin() {
    let before = "fn selected() { let [binding] = [1]; binding; }";
    check(
        before,
        before,
        vec![origin(0..before.len(), 0..before.len())],
        |_| {},
        false,
    );
    let copies_for = |start: usize, end: usize, after: &str, origins: &[MoveOrigin]| {
        let flag = AtomicBool::new(false);
        let files = BTreeMap::from([(
            "source.rs".into(),
            FileSnapshot {
                path: "source.rs".into(),
                source: before.into(),
                mode: 0o100644,
            },
        )]);
        let final_files = BTreeMap::from([(
            "target.rs".into(),
            FileSnapshot {
                path: "target.rs".into(),
                source: after.into(),
                mode: 0o100644,
            },
        )]);
        mapped(
            &items::LexicalLocation {
                path: "source.rs".into(),
                range: range(start, end),
                kind: "identifier".into(),
            },
            "target.rs",
            &files,
            &final_files,
            &BTreeMap::from([("target.rs".into(), (after.into(), Vec::new()))]),
            origins,
            (Instant::now() + Duration::from_secs(30), &flag),
        )
        .unwrap()
    };
    for pattern in [false, true] {
        let start = if pattern {
            before.find('[').unwrap()
        } else {
            before.find("binding").unwrap()
        };
        let end = start
            + if pattern {
                "[binding]".len()
            } else {
                "binding".len()
            };
        let one = origin(start..end, start..end);
        let other_start = if pattern { start + 1 } else { start - 1 };
        let other_end = if pattern { end - 1 } else { end + 1 };
        let other = origin(other_start..other_end, other_start..other_end);
        for origins in [
            vec![other.clone()],
            vec![one.clone(), one.clone(), other.clone()],
            vec![
                origin(start..start + 2, start..start + 2),
                origin(start + 2..end, start + 2..end),
                other,
            ],
        ] {
            // For identifier cases a whole pattern copy would also cover the
            // identifier. Remove it so missing/split origins remain decisive.
            let origins = if pattern {
                origins
            } else {
                origins
                    .into_iter()
                    .filter(|o| o.source_range.start_byte >= start)
                    .collect()
            };
            assert!(copies_for(start, end, before, &origins).is_none());
            check(before, before, origins, |_| {}, true);
        }
        let after = before.replacen(
            if pattern { "[binding]" } else { "binding" },
            if pattern { "(binding)" } else { "changed" },
            1,
        );
        assert!(
            copies_for(
                start,
                end,
                &after,
                &[origin(0..before.len(), 0..after.len())]
            )
            .is_none()
        );
        check(
            before,
            &after,
            vec![origin(0..before.len(), 0..after.len())],
            |_| {},
            true,
        );
    }
}
#[test]
fn independently_copied_nodes_must_form_the_same_final_pattern() {
    let text = "fn selected() { let (mut binding,) = (1,); let (mut binding,) = (2,); binding; }";
    let second = text.rfind("mut binding").unwrap() + 4;
    check(
        text,
        text,
        vec![origin(0..text.len(), 0..text.len())],
        |ledger| {
            ledger.truncate(1);
            // Both independently witnessed nodes are exact copies in the final CST,
            // but the identifier belongs to the other declaration's pattern.
            ledger[0].identifier.range = range(second, second + 7);
            ledger[0].references.clear();
        },
        true,
    );
}
#[test]
fn equal_range_match_wrapper_and_invalid_derived_scope() {
    let text = "fn selected() { match value { Wrap(binding) => binding } }";
    let tree = trivia::parse(
        text,
        Instant::now() + Duration::from_secs(30),
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    let at = text.find("Wrap(binding)").unwrap();
    assert!(exact(&tree, &range(at, at + 13), "match_pattern").is_some());
    check(
        text,
        text,
        vec![origin(0..text.len(), 0..text.len())],
        |_| {},
        false,
    );
    check(
        text,
        text,
        vec![origin(0..text.len(), 0..text.len())],
        |ledger| ledger[0].scope.kind = "parameter".into(),
        true,
    );
    let before = "fn selected(binding: u8) {}";
    let after = "fn selected(binding: ) {}";
    check(before, after, copies(before, after, "u8", ""), |_| {}, true);
    let after = "fn selected(binding: & ) {}";
    check(
        before,
        after,
        copies(before, after, "u8", "& "),
        |_| {},
        true,
    );
}
#[test]
fn exact_references_reject_wrong_visibility_or_same_spelled_shadow() {
    for (before, from, to) in [
        (
            "fn selected(binding: u8) { binding; { let binding = 2; binding; } }",
            "{ binding;",
            "= 2; binding;",
        ),
        (
            "fn selected(binding: u8) { if let mut binding = binding && binding > 0 { binding; } }",
            "= binding",
            "&& binding",
        ),
        (
            "fn selected(binding: u8) { let binding = binding; binding; }",
            "= binding",
            "; binding;",
        ),
        (
            "fn selected(binding: u8) { match 1 { binding @ _ if binding > 0 => binding, _ => binding } }",
            "_ => binding",
            "if binding",
        ),
        (
            "fn selected(binding: u8) { let call = || binding; let call = |binding: u8| binding; }",
            "|| binding",
            "u8| binding",
        ),
        (
            "fn selected(binding: u8) { for mut binding in [binding] { binding; } }",
            "[binding]",
            "{ binding;",
        ),
    ] {
        // Keep declarations byte-exact and redirect a uniquely copied original
        // reference into a valid but wrong lexical region in identical bytes.
        let start = before.find(from).unwrap() + from.find("binding").unwrap();
        let target = before.find(to).unwrap() + to.find("binding").unwrap();
        let origins = vec![
            origin(0..start, 0..start),
            origin(start..start + 7, target..target + 7),
            origin(start + 7..before.len(), start + 7..before.len()),
        ];
        check(before, before, origins, |_| {}, true);
        check(
            before,
            before,
            vec![origin(0..before.len(), 0..before.len())],
            |_| {},
            false,
        );
    }
}
#[test]
fn final_corpus_budget_exhaustion_after_overlay_work_is_fail_closed() {
    let before = "fn selected(binding: u8) {}";
    let flag = AtomicBool::new(false);
    // The overlay reaches the descriptor guard after smaller corpus entries
    // have been admitted, before attempting to parse the oversized final file.
    let after = " ".repeat(128 * 1024 * 1024);
    let result = probe(
        before,
        &after,
        Vec::new(),
        |_| {},
        (Instant::now() + Duration::from_secs(30), &flag),
    );
    assert_eq!(result.unwrap_err().code, "analysis_descriptor_bytes");
}

#[test]
fn final_audit_interruption_is_never_positive_evidence() {
    let before = "fn selected(binding: u8) {}";
    let flag = AtomicBool::new(true);
    let result = probe(
        before,
        before,
        vec![origin(0..before.len(), 0..before.len())],
        |_| {},
        (Instant::now() + Duration::from_secs(30), &flag),
    );
    assert_eq!(result.unwrap_err().code, "CANCELLED");
    flag.store(false, std::sync::atomic::Ordering::Relaxed);
    let result = probe(
        before,
        before,
        vec![origin(0..before.len(), 0..before.len())],
        |_| {},
        (Instant::now() - Duration::from_secs(1), &flag),
    );
    assert_eq!(result.unwrap_err().code, "planning_deadline");
}
