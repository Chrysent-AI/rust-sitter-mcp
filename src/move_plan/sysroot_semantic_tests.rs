use super::{tests::evaluate, *};
const LIBRARY: &str = "@sysroot/0/library/fixture/src/lib.rs";
const CALL: &str = "thing.written";
fn inputs(library: &str, caller: &str, enabled: bool, controls: (Instant, &AtomicBool)) -> Inputs {
    let configuration: Configuration = serde_json::from_value(serde_json::json!({"crates":[
        {"name":"caller", "root_file":"lib.rs", "edition":"2024", "features":[], "cfg":[], "dependencies":[{"name":"dep","crate_name":"sysroot_fixture"}]},
        {"name":"sysroot_fixture", "root_file":LIBRARY, "edition":"2024", "features":[], "cfg":[], "dependencies":[]}
    ]})).unwrap();
    let mut inputs = Inputs::new(
        BTreeMap::from([
            ("lib.rs".into(), caller.into()),
            (LIBRARY.into(), library.into()),
        ]),
        &configuration,
        controls,
    )
    .unwrap();
    if enabled {
        inputs.sysroot = Some(super::super::sysroot::Profile {
            configuration,
            origins: BTreeMap::from([("sysroot_fixture".into(), "alloc".into())]),
            cfg: BTreeMap::from([(
                "sysroot_fixture".into(),
                BTreeMap::from([(("test".into(), None), false)]),
            )]),
        });
        inputs.sysroot_origins.insert(
            "sysroot_fixture".into(),
            "sysroot:alloc:synthetic-mechanism-test".into(),
        );
    }
    inputs
}
fn anchor(caller: &str) -> SourceAnchor {
    let at = caller.find(CALL).unwrap();
    SourceAnchor {
        path: "lib.rs".into(),
        range: range(at, at + CALL.len()),
        expected_text: CALL.into(),
    }
}
fn coverage(config: Configuration) -> ResolutionCoverage {
    ResolutionCoverage {
        decisions: 0,
        configuration: config,
        snapshot_id: "synthetic-mechanism-test".into(),
        semantic_input_digest: "input".into(),
        final_overlay_digest: "final".into(),
        analyzer: ANALYZER.into(),
        statement: String::new(),
        omissions: vec![],
        context_evaluations: vec![],
    }
}
fn check_pair(
    old: &Inputs,
    new: &Inputs,
    caller: &str,
    expected: bool,
    controls: (Instant, &AtomicBool),
) {
    let occurrence = anchor(caller);
    let mut needs = vec![Need {
        path: occurrence.path.clone(),
        range: occurrence.range.clone(),
        item_ids: vec![],
        reason: DecisionReason::MemberOrConstructorUnproved,
        category: "method",
        message: "unproved".into(),
        attribute_range: None,
        choice_target: None,
        lexical_uncertainty: None,
        refusal_basis: vec![],
    }];
    let proofs = evaluate(
        old,
        new,
        &mut needs,
        &[],
        &coverage(old.configuration.clone()),
        controls,
    )
    .unwrap();
    assert_eq!(proofs.len(), usize::from(expected));
    assert_eq!(needs.len(), usize::from(!expected));
    if !expected {
        assert!(
            needs[0]
                .refusal_basis
                .iter()
                .any(|basis| basis.class.starts_with("semantic_")
                    && basis.anchor.path == occurrence.path
                    && basis.anchor.range.as_ref() == Some(&occurrence.range))
        );
    }
}
#[test]
fn sysroot_profile_admission_is_origin_scoped_and_unknown_attributes_stay_unproved() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(60), &flag);
    let caller = "fn selected(thing: dep::Thing) { thing.written(); }";
    for (attribute, enabled, expected) in [
        (
            "#[stable(feature=\"fixture\", since=\"1.0.0\")]",
            true,
            true,
        ),
        (
            "#[stable(feature=\"fixture\", since=\"1.0.0\")]",
            false,
            false,
        ),
        ("#[unexamined]", true, false),
        ("#[cfg_attr(test, unexamined)]", true, true),
        ("#[cfg_attr(test, unexamined)]", false, false),
    ] {
        let library =
            format!("{attribute} pub struct Thing; impl Thing {{ pub fn written(&self) {{}} }}");
        let before = inputs(&library, caller, enabled, controls);
        let after = inputs(&library, caller, enabled, controls);
        check_pair(&before, &after, caller, expected, controls);
        if attribute == "#[unexamined]" {
            assert!(
                before
                    .context
                    .borrow()
                    .values()
                    .any(|record| record.reason == "unsupported_attribute")
            );
        }
    }
    let library = "pub struct Thing; impl Thing { pub fn written(&self) {} }";
    let caller_with_attr = "#[unexamined] fn selected(thing: dep::Thing) { thing.written(); }";
    assert!(
        fact(
            &inputs(library, caller_with_attr, true, controls),
            &anchor(caller_with_attr),
            false
        )
        .is_none(),
        "sysroot profile must never admit caller attributes"
    );
}
#[test]
fn sysroot_trait_only_namespace_audit_does_not_admit_generated_members_or_trait_generic_facts() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(60), &flag);
    let ordinary = "fn selected(thing: dep::Thing) { thing.written(); }";
    for (library, caller, expected) in [
        (
            "pub struct Thing; impl Thing { pub fn written(&self) {} } trait Marker {} macro_rules! mark { () => { impl Marker for Thing {} }; } mark!();",
            ordinary,
            true,
        ),
        (
            "pub struct Thing; macro_rules! generated { () => { impl Thing { pub fn written(&self) {} } }; } generated!();",
            ordinary,
            false,
        ),
        (
            "pub struct Thing; impl Thing { pub fn written(&self) {} } macro_rules! extra { () => { impl Thing { pub fn extra(&self) {} } }; } extra!();",
            ordinary,
            false,
        ),
        (
            "pub struct Thing; impl Thing { pub fn written(&self) {} } macro_rules! namespace { () => { pub struct Extra; }; } namespace!();",
            ordinary,
            false,
        ),
        (
            "pub struct Thing; impl Thing { pub fn written(&self) {} } trait Marker {} macro_rules! mark { () => { #[unexamined] impl Marker for Thing {} }; } mark!();",
            ordinary,
            false,
        ),
        (
            "pub struct Thing; pub trait Operation { fn written(&self); } impl Operation for Thing { fn written(&self) {} }",
            "use dep::Operation; fn selected(thing: dep::Thing) { thing.written(); }",
            false,
        ),
        (
            "pub struct Thing; impl Thing { pub fn written<T>(&self) {} }",
            ordinary,
            false,
        ),
        (
            "pub struct Thing; impl Thing { pub fn written(&self) {} }",
            "fn selected<T>(thing: dep::Thing) { thing.written(); }",
            false,
        ),
        (
            "pub struct Thing; impl Thing { pub fn written(&self) {} } include!(\"missing.rs\");",
            ordinary,
            false,
        ),
    ] {
        let before = inputs(library, caller, true, controls);
        let after = inputs(library, caller, true, controls);
        check_pair(&before, &after, caller, expected, controls);
    }
}
#[test]
fn sysroot_both_overlay_identity_and_composite_withholding_preserve_real_counts() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(60), &flag);
    let caller = "fn selected(thing: dep::Thing) { thing.written(); }";
    let library = "pub struct Thing; impl Thing { pub fn written(&self) {} }";
    let old = inputs(library, caller, true, controls);
    let new = inputs(library, caller, true, controls);
    let a = anchor(caller);
    let need = || Need {
        path: a.path.clone(),
        range: a.range.clone(),
        item_ids: vec![],
        reason: DecisionReason::MemberOrConstructorUnproved,
        category: "method",
        message: "unproved".into(),
        attribute_range: None,
        choice_target: None,
        lexical_uncertainty: None,
        refusal_basis: vec![],
    };
    let mut needs = vec![need()];
    let proof = evaluate(
        &old,
        &new,
        &mut needs,
        &[],
        &coverage(old.configuration.clone()),
        controls,
    )
    .unwrap()
    .pop()
    .unwrap();
    assert!(needs.is_empty());
    assert_eq!(proof.declaration, proof.final_declaration);
    assert_eq!(proof.original_receiver, proof.final_original_receiver);
    assert_eq!(proof.adjusted_receiver, proof.final_adjusted_receiver);
    let changed = inputs(
        &library.replace("pub struct Thing", "pub struct Other; pub struct Thing"),
        caller,
        true,
        controls,
    );
    let mut needs = vec![need()];
    assert!(
        evaluate(
            &old,
            &changed,
            &mut needs,
            &[],
            &coverage(old.configuration.clone()),
            controls
        )
        .unwrap()
        .is_empty()
    );
    assert!(
        needs[0]
            .refusal_basis
            .iter()
            .any(|basis| basis.class == "semantic_identity_unproved")
    );
    let final_unsupported = inputs(
        "#[unexamined] pub struct Thing; impl Thing { pub fn written(&self) {} }",
        caller,
        true,
        controls,
    );
    let mut final_needs = vec![need()];
    assert!(
        evaluate(
            &old,
            &final_unsupported,
            &mut final_needs,
            &[],
            &coverage(old.configuration.clone()),
            controls
        )
        .unwrap()
        .is_empty()
    );
    assert!(
        final_needs[0]
            .refusal_basis
            .iter()
            .any(|basis| basis.class == "semantic_final_fact_unproved")
    );
    let mut envelope = MoveEnvelope::empty(crate::result::Limits::default());
    envelope
        .plan
        .binding_proofs
        .push(super::super::BindingProof::RaResolved(Box::new(proof)));
    envelope.plan.resolution_coverage = Some(coverage(old.configuration.clone()));
    envelope.coverage.ra_resolved = 1;
    envelope.coverage.assumed_declared_identity = 1;
    let mut capture: super::super::sysroot::Coverage = super::super::sysroot::test_coverage();
    capture.complete = true;
    capture.corpus_digest = "synthetic-mechanism-test".into();
    capture.freshness = "failed_or_interrupted".into();
    envelope.coverage.scan.sysroot_sources = Some(capture);
    envelope.withhold();
    envelope.withhold_external_resolution();
    assert!(!envelope.plan.applicable);
    assert!(envelope.plan.patch.is_none());
    assert!(envelope.plan.binding_proofs.is_empty());
    assert_eq!(envelope.coverage.ra_resolved, 0);
    assert_eq!(envelope.coverage.assumed_declared_identity, 0);
    assert!(envelope.plan.resolution_coverage.is_none());
    assert!(
        envelope
            .coverage
            .scan
            .sysroot_sources
            .unwrap()
            .corpus_digest
            .is_empty()
    );
}

#[test]
fn sysroot_response_budget_overflow_withholds_real_composite_proofs_and_artifacts() {
    let flag = AtomicBool::new(false);
    let controls = (Instant::now() + Duration::from_secs(60), &flag);
    let caller = "fn selected(thing: dep::Thing) { thing.written(); }";
    let library = "pub struct Thing; impl Thing { pub fn written(&self) {} }";
    let old = inputs(library, caller, true, controls);
    let new = inputs(library, caller, true, controls);
    let occurrence = anchor(caller);
    let mut needs = vec![Need {
        path: occurrence.path.clone(),
        range: occurrence.range.clone(),
        item_ids: vec![],
        reason: DecisionReason::MemberOrConstructorUnproved,
        category: "method",
        message: "unproved".into(),
        attribute_range: None,
        choice_target: None,
        lexical_uncertainty: None,
        refusal_basis: vec![],
    }];
    let proof = evaluate(
        &old,
        &new,
        &mut needs,
        &[],
        &coverage(old.configuration.clone()),
        controls,
    )
    .unwrap()
    .pop()
    .unwrap();
    assert!(needs.is_empty());
    let envelope = || {
        let mut result = MoveEnvelope::empty(crate::result::Limits::default());
        result.plan.applicable = true;
        result.plan.patch = Some("synthetic replay artifact\n".repeat(100));
        result.plan.edits = Some(vec![]);
        result.plan.created_files = Some(vec![]);
        result
            .plan
            .binding_proofs
            .push(super::super::BindingProof::RaResolved(Box::new(
                proof.clone(),
            )));
        result.plan.resolution_coverage = Some(coverage(old.configuration.clone()));
        result.coverage.ra_resolved = 1;
        result.coverage.assumed_declared_identity = 1;
        let mut capture = super::super::sysroot::test_coverage();
        capture.complete = true;
        capture.corpus_digest = "synthetic-mechanism-test".into();
        capture.freshness = "observationally_unchanged".into();
        result.coverage.scan.sysroot_sources = Some(capture);
        result
    };
    let mut exact = envelope();
    // The serialized limit is part of its own budget; stabilize its digit width
    // before exercising the exact inclusive boundary.
    for _ in 0..3 {
        exact.limits.response_bytes = exact.planning_wire_bytes();
    }
    assert_eq!(exact.planning_wire_bytes(), exact.limits.response_bytes);
    exact.fit_plan();
    assert!(exact.plan.applicable);
    assert_eq!(exact.coverage.ra_resolved, 1);
    assert_eq!(exact.plan.binding_proofs.len(), 1);
    assert!(exact.truncation_reasons.is_empty());
    for mandatory_overflow in [false, true] {
        let mut result = envelope();
        if mandatory_overflow {
            result
                .coverage
                .scan
                .sysroot_sources
                .as_mut()
                .unwrap()
                .omissions
                .push("mandatory capture disclosure ".repeat(4096));
            result.limits.response_bytes = 65_536;
        } else {
            for _ in 0..3 {
                result.limits.response_bytes = result.planning_wire_bytes() - 1;
            }
            assert_eq!(
                result.planning_wire_bytes(),
                result.limits.response_bytes + 1
            );
        }
        result.fit_plan();
        assert_eq!(result.status, "partial");
        assert_eq!(result.plan.state, "incomplete");
        assert!(
            result
                .truncation_reasons
                .iter()
                .any(|reason| reason == "response_bytes")
        );
        assert!(
            result
                .plan
                .blockers
                .iter()
                .any(|blocker| blocker.code == "response_bytes")
        );
        assert!(!result.plan.applicable);
        assert!(result.plan.patch.is_none());
        assert!(result.plan.edits.is_none());
        assert!(result.plan.created_files.is_none());
        assert!(result.plan.binding_proofs.is_empty());
        assert!(result.plan.resolution_coverage.is_none());
        assert_eq!(result.coverage.ra_resolved, 0);
        assert_eq!(result.coverage.assumed_declared_identity, 0);
        let capture = result.coverage.scan.sysroot_sources.as_ref().unwrap();
        assert!(!capture.complete);
        assert!(capture.corpus_digest.is_empty());
        assert_eq!(capture.freshness, "failed_or_interrupted");
        assert_eq!(capture.capture_usage.max_output_bytes_per_stream, 65_536);
        if mandatory_overflow {
            assert!(capture.omissions.iter().any(|text| text.len() > 65_536));
            assert!(result.planning_wire_bytes() > result.limits.response_bytes);
        } else {
            assert!(result.planning_wire_bytes() <= result.limits.response_bytes);
        }
    }
}
