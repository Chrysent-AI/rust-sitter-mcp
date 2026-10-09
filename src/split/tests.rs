use super::*;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "rust-sitter-advice-unit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet", "--template="])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        fs::write(
            root.join("lib.rs"),
            "fn alpha_read() { alpha_parse(); }\nfn alpha_parse() { alpha_read(); }\nfn retained() {}\n",
        )
        .unwrap();
        Self(root)
    }
    fn request(&self) -> SuggestSplitRequest {
        serde_json::from_value(serde_json::json!({"repo_path":self.0,"crate_root":"lib.rs","source_path":"lib.rs","limits":{"text_bytes":0}})).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn final_recheck_change_withholds_every_complete_draft() {
    let repo = Fixture::new();
    let result = run_with_recheck(&repo.0, repo.request(), &AtomicBool::new(false), || {
        fs::write(repo.0.join("lib.rs"), "fn changed() {}\n").unwrap();
    });
    assert_eq!(result.status, "partial");
    assert!(result.drafts.is_empty());
    assert!(
        result
            .truncation_reasons
            .iter()
            .any(|r| r == "SOURCE_CHANGED")
    );
    assert!(result.counts.omissions["drafts"] > 0);
    assert!(result.signals.is_empty() && result.decisions.is_empty());
}
#[test]
fn creation_and_ignore_changes_are_not_execution_authority() {
    for ignore in [false, true] {
        let repo = Fixture::new();
        let result = run_with_recheck(&repo.0, repo.request(), &AtomicBool::new(false), || {
            if ignore {
                fs::write(repo.0.join(".gitignore"), "alpha.rs\n").unwrap();
            } else {
                fs::write(repo.0.join("alpha.rs"), "fn competing() {}\n").unwrap();
            }
        });
        assert!(result.drafts.is_empty());
        assert_ne!(result.status, "complete");
        assert_eq!(result.integrity.semantic, "not_performed");
    }
}
#[test]
fn cancellation_after_analysis_keeps_a_typed_failed_envelope() {
    let repo = Fixture::new();
    let cancelled = AtomicBool::new(false);
    let result = run_with_recheck(&repo.0, repo.request(), &cancelled, || {
        cancelled.store(true, Ordering::Relaxed);
    });
    assert_eq!(result.status, "failed");
    assert_eq!(result.error.unwrap().code, "CANCELLED");
    assert!(result.drafts.is_empty());
}
#[test]
fn stopped_chain_analysis_omits_provisional_diagnostics_and_links_together() {
    for cancel in [false, true] {
        let repo = Fixture::new();
        fs::write(repo.0.join("lib.rs"), "use fixture_library::scheduler;\n").unwrap();
        fs::write(repo.0.join("leaf.rs"), "fn one() {}\nfn two() {}\n").unwrap();
        let mut request = repo.request();
        request.source_path = "leaf.rs".into();
        let cancelled = AtomicBool::new(false);
        let result = run_with_recheck(&repo.0, request, &cancelled, || {
            if cancel {
                cancelled.store(true, Ordering::Relaxed);
            } else {
                fs::write(repo.0.join("leaf.rs"), "fn changed() {}\n").unwrap();
            }
        });
        if cancel {
            assert_eq!(result.error.as_ref().unwrap().code, "CANCELLED");
        } else {
            assert!(
                result
                    .truncation_reasons
                    .iter()
                    .any(|r| r == "SOURCE_CHANGED")
            );
        }
        assert!(result.chain_diagnostics.is_empty());
        assert!(result.decisions.is_empty());
        assert!(result.drafts.is_empty());
        assert_eq!(result.counts.omissions["chain_diagnostics"], 1);
        assert_eq!(result.counts.omissions["chain_diagnostic_references"], 1);
    }
}

#[test]
fn injected_small_candidate_cap_stops_at_its_first_excess() {
    let repo = Fixture::new();
    fs::write(
        repo.0.join("lib.rs"),
        "fn target() {}\nfn caller() { target(); target(); }\n",
    )
    .unwrap();
    let request = repo.request();
    let cancelled = AtomicBool::new(false);
    let controls = Controls {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &cancelled,
    };
    let mut result = SuggestSplitEnvelope::empty(request.limits.clone().into());
    result.effective_work_limits.reference_candidates = 1;
    let error = build(&repo.0, &request, controls, || {}, &mut result, &mut None).unwrap_err();
    assert_eq!(error.code, "reference_work_limit");
    assert_eq!(result.counts.reference_candidates, 2);
    assert!(result.drafts.is_empty());
}
#[test]
fn final_fit_tier_accounts_membership_without_losing_root_or_snapshot() {
    let repo = Fixture::new();
    let source = (0..1500)
        .map(|i| {
            format!(
                "fn unit_{i}() {{ {} }}\n",
                if i < 2 {
                    format!("unit_{}();", i ^ 1)
                } else {
                    String::new()
                }
            )
        })
        .collect::<String>();
    fs::write(repo.0.join("lib.rs"), source).unwrap();
    let mut request = repo.request();
    request.max_items = 1500;
    request.limits.response_bytes = 16 * 1024 * 1024;
    let mut result = run(&repo.0, request, &AtomicBool::new(false));
    assert_eq!(result.status, "complete");
    let root = result.root.clone();
    let snapshot = result.snapshot_id.clone();
    let drafts = result.drafts.len();
    assert!(drafts > 0);
    result.limits.response_bytes = 65536;
    result
        .fit(Controls {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &AtomicBool::new(false),
        })
        .unwrap();
    assert_eq!(result.root, root);
    assert_eq!(result.snapshot_id, snapshot);
    assert_eq!(result.status, "partial");
    assert!(result.drafts.is_empty());
    assert_eq!(result.counts.omissions["draft_summaries"], drafts);
    assert_eq!(
        result.counts.omissions["draft_summary_membership_references"],
        drafts * 1500
    );
    assert!(result.wire_bytes() <= result.limits.response_bytes);
}

#[test]
fn fitting_withheld_drafts_preserves_overlap_labels_and_accounts_records() {
    let repo = Fixture::new();
    fs::write(repo.0.join("lib.rs"),
        "struct Recorder;\nimpl Recorder {\nfn record_start() {}\nfn record_stop() {}\nfn other() {}\n}\n").unwrap();
    let mut result = run(&repo.0, repo.request(), &AtomicBool::new(false));
    assert_eq!(result.status, "complete");
    let expected: Vec<_> = result
        .drafts
        .iter()
        .map(|draft| {
            draft
                .groups
                .iter()
                .map(|g| {
                    (
                        g.overlap_ids.clone(),
                        serde_json::to_value(&g.size_interpretation).unwrap(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let overlap_count = result.overlaps.len();
    let group_links: usize = result.overlaps.iter().map(|o| o.draft_groups.len()).sum();
    assert_eq!(overlap_count, 3);
    result.drafts[0].rationale = "x".repeat(100_000);
    result.limits.response_bytes = 65_536;
    result
        .fit(Controls {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &AtomicBool::new(false),
        })
        .unwrap();
    assert_eq!(result.status, "partial");
    assert!(result.drafts.is_empty());
    assert!(result.overlaps.is_empty());
    assert_eq!(result.counts.omissions["overlaps"], overlap_count);
    assert_eq!(
        result.counts.omissions["overlap_draft_group_links"],
        group_links
    );
    let actual: Vec<_> = result
        .draft_summaries
        .iter()
        .map(|draft| {
            draft
                .groups
                .iter()
                .map(|g| {
                    (
                        g.overlap_ids.clone(),
                        serde_json::to_value(&g.size_interpretation).unwrap(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(actual, expected);
    assert!(
        result
            .inventory
            .iter()
            .filter(|i| i.enclosing_impl.is_some())
            .all(|i| i.enclosing_impl_id.is_some() && i.size_interpretation.is_some())
    );
    assert!(result.wire_bytes() <= result.limits.response_bytes);
}

#[test]
fn final_fit_counts_omitted_decision_ids_not_compressed_runs() {
    let repo = Fixture::new();
    fs::write(
        repo.0.join("lib.rs"),
        "fn retained() {}\nfn risky(x: u8) { x.a(); x.b(); x.c(); }\n",
    )
    .unwrap();
    let mut result = run(&repo.0, repo.request(), &AtomicBool::new(false));
    assert_eq!(result.decision_groups.len(), 1);
    assert_eq!(result.decision_groups[0].count, 3);
    assert_eq!(result.decision_groups[0].decision_ids.len(), 1);
    // Force the final summary tier to overflow independently of preview detail.
    result.decision_groups[0].unresolved_consequence = "x".repeat(100_000);
    result.limits.response_bytes = 65_536;
    result
        .fit(Controls {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &AtomicBool::new(false),
        })
        .unwrap();
    assert!(result.decision_groups.is_empty());
    assert_eq!(result.counts.omissions["decision_groups"], 1);
    assert_eq!(result.counts.omissions["decision_group_references"], 3);
    assert_eq!(result.counts.omissions["decisions"], 3);
    assert!(result.wire_bytes() <= result.limits.response_bytes);
}

#[test]
fn advice_groups_separate_consequences_and_actions_with_exact_interleaved_runs() {
    let repo = Fixture::new();
    fs::write(
        repo.0.join("lib.rs"),
        "fn retained() {}\nfn risky(x: u8) { x.a(); x.b(); x.c(); x.d(); x.e(); x.f(); }\n",
    )
    .unwrap();
    let mut request = repo.request();
    request.limits.diagnostic_count = 100_000;
    let mut result = run(&repo.0, request, &AtomicBool::new(false));
    assert_eq!(result.decisions.len(), 6);
    let original_consequence = result.decisions[0].unresolved_consequence.clone();
    for index in [1, 3] {
        result.decisions[index].unresolved_consequence = "another consequence".into();
    }
    result.decisions[4].action = DecisionAction::UnsupportedInEngine {
        construct: "another_construct".into(),
        instruction: "another instruction".into(),
    };
    result.drafts.clear();
    drafts::finalize(
        &mut result,
        Controls {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &AtomicBool::new(false),
        },
    )
    .unwrap();
    assert_eq!(result.decision_groups.len(), 3);
    let original = result
        .decision_groups
        .iter()
        .find(|g| {
            g.unresolved_consequence == original_consequence
                && g.actions[0] == result.decisions[0].action.summary()
        })
        .unwrap();
    assert_eq!(original.count, 3);
    assert_eq!(
        serde_json::to_value(&original.decision_ids).unwrap(),
        serde_json::json!([
            {"first_id":"d/0","count":1},
            {"first_id":"d/2","count":1},
            {"first_id":"d/5","count":1}
        ])
    );
    let different_consequence = result
        .decision_groups
        .iter()
        .find(|g| g.unresolved_consequence == "another consequence")
        .unwrap();
    assert_eq!(different_consequence.count, 2);
    assert_eq!(
        serde_json::to_value(&different_consequence.decision_ids).unwrap(),
        serde_json::json!([
            {"first_id":"d/1","count":1},
            {"first_id":"d/3","count":1}
        ])
    );
}

#[test]
fn impl_heavy_mixed_layout_drafts_members_without_duplicate_body_observations() {
    let repo = Fixture::new();
    fs::write(repo.0.join("lib.rs"), "mod flat;\n").unwrap();
    fs::write(repo.0.join("flat.rs"), "mod legacy;\n").unwrap();
    fs::create_dir_all(repo.0.join("flat/legacy")).unwrap();
    fs::write(repo.0.join("flat/legacy/mod.rs"), "mod heavy;\n").unwrap();
    fs::write(repo.0.join("flat/legacy/heavy.rs"),
        "struct Recorder;\nimpl Recorder {\nfn record_start() { missing(); self.record_stop(); }\nfn record_stop() {}\nfn other() {}\n#[cfg(unknown)] fn record_hidden() {}\n}\n").unwrap();
    let mut request = repo.request();
    request.source_path = "flat/legacy/heavy.rs".into();
    request.limits.diagnostic_count = 100_000;
    let result = run(&repo.0, request, &AtomicBool::new(false));
    assert_eq!(result.status, "complete");
    assert!(!result.drafts.is_empty(), "{result:?}");
    assert_eq!(result.counts.eligible_items, 4);
    let member = result
        .inventory
        .iter()
        .find(|i| i.name.as_deref() == Some("record_start"))
        .unwrap();
    assert_eq!(member.eligibility, "context_sensitive");
    let missing: Vec<_> = result
        .decisions
        .iter()
        .filter(|d| d.reason == DecisionReason::ExternalOrMissingBinding)
        .collect();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].item_ids, vec![member.id.clone()]);
    for draft in &result.drafts {
        let mut ids: Vec<_> = draft.groups.iter().flat_map(|g| &g.item_ids).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), result.inventory.len());
        for group in draft.groups.iter().skip(1) {
            assert_eq!(
                group.destination.as_ref().unwrap().parent_path,
                "flat/legacy/mod.rs"
            );
            assert!(
                group
                    .item_ids
                    .iter()
                    .all(|id| draftable(result.inventory.iter().find(|i| &i.id == id).unwrap()))
            );
        }
        assert!(
            draft
                .groups
                .iter()
                .skip(1)
                .any(|g| g.item_ids.contains(&member.id))
        );
        let group = draft
            .groups
            .iter()
            .find(|g| g.item_ids.contains(&member.id))
            .unwrap();
        assert_eq!(group.expected_to_block.counts.external_binding, 1);
        for item in result
            .inventory
            .iter()
            .filter(|i| partitioned_impl(i) || !draftable(i))
        {
            assert!(draft.groups[0].item_ids.contains(&item.id));
        }
    }
}

#[test]
fn excluded_giant_impl_has_no_draft_and_keeps_its_member_reasons() {
    let repo = Fixture::new();
    let source = format!(
        "#[cfg(unknown)] impl Missing {{ fn one() {{ {} }} fn two() {{}} }}\n",
        "call();".repeat(500)
    );
    fs::write(repo.0.join("lib.rs"), source).unwrap();
    let result = run(&repo.0, repo.request(), &AtomicBool::new(false));
    assert_eq!(result.status, "complete");
    assert!(result.drafts.is_empty());
    assert_eq!(result.counts.eligible_items, 0);
    assert!(
        result
            .inventory
            .iter()
            .filter(|i| i.enclosing_impl.is_some())
            .all(|i| !i.reasons.is_empty())
    );
    assert!(
        result
            .draft_eligibility
            .reasons
            .iter()
            .any(|r| r == "fewer_than_two_eligible_items")
    );
}

#[test]
fn zero_consequences_and_fitted_memberships_keep_unassessed_applicability() {
    let empty = ConsequenceSummary::default();
    assert!(
        empty
            .classes
            .values()
            .all(|m| m.count == 0 && m.decision_refs.is_empty())
    );
    assert_eq!(empty.unmapped.membership.count, 0);
    assert_eq!(empty.destination_and_batch_applicability, "not_assessed");
    let repo = Fixture::new();
    let mut result = run(&repo.0, repo.request(), &AtomicBool::new(false));
    let expected: Vec<_> = result
        .drafts
        .iter()
        .map(|d| {
            d.groups
                .iter()
                .map(|g| {
                    let summary = &g.consequence_summary;
                    assert_eq!(summary.destination_and_batch_applicability, "not_assessed");
                    serde_json::to_value(summary).unwrap()
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(!expected.is_empty());
    result.drafts[0].rationale = "x".repeat(100_000);
    result.limits.response_bytes = 65_536;
    result
        .fit(Controls {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &AtomicBool::new(false),
        })
        .unwrap();
    assert!(result.drafts.is_empty());
    let actual: Vec<_> = result
        .draft_summaries
        .iter()
        .map(|d| {
            d.groups
                .iter()
                .map(|g| serde_json::to_value(&g.consequence_summary).unwrap())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(actual, expected);
    assert!(result.wire_bytes() <= result.limits.response_bytes);
}

fn display_controls() -> Controls<'static> {
    static CANCELLED: AtomicBool = AtomicBool::new(false);
    Controls {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &CANCELLED,
    }
}

fn display_fixture() -> SuggestSplitEnvelope {
    let repo = Fixture::new();
    let mut result = run(&repo.0, repo.request(), &AtomicBool::new(false));
    assert_eq!(result.status, "complete");
    // Exercise descriptor text and JSON escaping, without first stripping text.
    for item in &mut result.inventory {
        item.span.text = Some("line\n\"\\λ\t".repeat(64));
        item.span.text_bytes = item.span.text.as_ref().unwrap().len();
        item.span.text_omitted = false;
    }
    let inventory: BTreeMap<_, _> = result
        .inventory
        .iter()
        .map(|item| (item.id.clone(), item.span.clone()))
        .collect();
    for signal in &mut result.signals {
        if matches!(signal.kind.as_str(), "item_size" | "name_prefix") {
            signal.evidence = signal
                .item_ids
                .iter()
                .map(|id| inventory[id].clone())
                .collect();
        }
    }
    result.drafts[0].rationale = "mandatory \"\\\nλ".repeat(6000);
    force_display_pressure(&mut result);
    result
}

fn force_display_pressure(result: &mut SuggestSplitEnvelope) {
    // Stabilize the budget's digit width before choosing the one-byte overflow.
    result.limits.response_bytes = result.wire_bytes();
    result.limits.response_bytes = result.wire_bytes() - 1;
    assert!(result.wire_bytes() > result.limits.response_bytes);
}

fn expand_declaration_displays(result: &mut SuggestSplitEnvelope) {
    let inventory: BTreeMap<_, _> = result
        .inventory
        .iter()
        .map(|item| (item.id.clone(), item.span.clone()))
        .collect();
    let mut expanded = 0;
    for signal in &mut result.signals {
        if matches!(signal.kind.as_str(), "item_size" | "name_prefix") && signal.evidence.is_empty()
        {
            signal.evidence = signal
                .item_ids
                .iter()
                .map(|id| inventory[id].clone())
                .collect();
            expanded += signal.evidence.len();
        }
    }
    assert_eq!(
        result
            .counts
            .omissions
            .remove("duplicate_declaration_display_spans"),
        Some(expanded)
    );
}

#[test]
fn duplicate_declaration_display_fit_reconstructs_exactly_and_honors_escaped_wire_boundary() {
    let before = display_fixture();
    let mut projected = before.clone();
    assert!(
        projected
            .fit_duplicate_declaration_displays(display_controls())
            .unwrap()
    );
    let exact = projected.wire_bytes();
    assert!(exact > 65_536);
    let value = serde_json::to_value(&projected).unwrap();
    let duplicated = serde_json::to_vec(&serde_json::json!({
        "content":[{"type":"text","text":value.to_string()}],
        "structuredContent":value,"isError":false
    }))
    .unwrap()
    .len()
        + 4096;
    assert_eq!(exact, duplicated);
    assert!(
        exact > 2 * value.to_string().len() + 4096,
        "escaping must be counted"
    );
    expand_declaration_displays(&mut projected);
    assert_eq!(
        serde_json::to_value(&projected).unwrap(),
        serde_json::to_value(&before).unwrap()
    );

    let mut boundary = before.clone();
    boundary.limits.response_bytes = exact;
    assert!(
        boundary
            .fit_duplicate_declaration_displays(display_controls())
            .unwrap()
    );
    assert_eq!(boundary.wire_bytes(), exact);
    let mut too_small = before;
    too_small.limits.response_bytes = exact - 1;
    let unchanged = serde_json::to_value(&too_small).unwrap();
    assert!(
        !too_small
            .fit_duplicate_declaration_displays(display_controls())
            .unwrap()
    );
    assert_eq!(serde_json::to_value(&too_small).unwrap(), unchanged);
}

#[test]
fn declaration_display_sharing_refuses_descriptor_or_inventory_mismatches_atomically() {
    for case in 0..22 {
        let mut result = display_fixture();
        let index = result
            .signals
            .iter()
            .position(|s| s.kind == "name_prefix")
            .unwrap();
        match case {
            0 => result.signals[index].evidence.reverse(),
            1 => result.signals[index].evidence[0].text = Some("different bytes".into()),
            2 => result.signals[index].evidence[0].text_omitted = true,
            3 => result.signals[index].evidence[0].text_bytes += 1,
            4 => result.signals[index].evidence[0].start.byte_column += 1,
            5 => result.signals[index].evidence[0].end.line += 1,
            6 => result.signals[index].evidence[0].range.end_byte += 1,
            7 => result.signals[index].item_ids[0] = "missing".into(),
            8 => {
                result.inventory.pop();
            }
            9 => result.status = "partial".into(),
            10 => result.draft_eligibility.membership_complete = false,
            11 => result.draft_eligibility.evidence_complete = false,
            12 => result.coverage.scope_exhaustive = false,
            13 => {
                result.counts.omissions.insert("inventory_items".into(), 1);
            }
            14 => result.inventory[1].id = result.inventory[0].id.clone(),
            15 => {
                result.signals[index].evidence.pop();
            }
            16 => {
                result.signals[index].item_ids.clear();
            }
            17 => result.coverage.scan_exhausted = false,
            18 => result.coverage.eligible_scan_complete = false,
            19 => result.counts.returned_items -= 1,
            20 => result.counts.inventory_items += 1,
            21 => result.error = Some(DomainError::new("SOURCE_CHANGED", "changed")),
            _ => unreachable!(),
        }
        force_display_pressure(&mut result);
        let before = serde_json::to_value(&result).unwrap();
        assert!(
            !result
                .fit_duplicate_declaration_displays(display_controls())
                .unwrap(),
            "case {case}"
        );
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            before,
            "case {case}"
        );
    }
}

#[test]
fn sharing_is_pressure_only_and_never_removes_unique_occurrences_or_other_signal_kinds() {
    let mut result = display_fixture();
    result.limits.response_bytes = result.wire_bytes();
    let before = serde_json::to_value(&result).unwrap();
    result.fit(display_controls()).unwrap();
    assert_eq!(serde_json::to_value(&result).unwrap(), before);

    result.limits.response_bytes -= 1;
    let other: Vec<_> = result
        .signals
        .iter()
        .filter(|s| !matches!(s.kind.as_str(), "item_size" | "name_prefix"))
        .map(|s| serde_json::to_value(s).unwrap())
        .collect();
    assert!(!other.is_empty());
    assert!(
        result
            .fit_duplicate_declaration_displays(display_controls())
            .unwrap()
    );
    let after: Vec<_> = result
        .signals
        .iter()
        .filter(|s| !matches!(s.kind.as_str(), "item_size" | "name_prefix"))
        .map(|s| serde_json::to_value(s).unwrap())
        .collect();
    assert_eq!(after, other);

    let mut unsupported = display_fixture();
    for signal in &mut unsupported.signals {
        if matches!(signal.kind.as_str(), "item_size" | "name_prefix") {
            signal.kind = "unique_declaration".into();
        }
    }
    let before = serde_json::to_value(&unsupported).unwrap();
    assert!(
        !unsupported
            .fit_duplicate_declaration_displays(display_controls())
            .unwrap()
    );
    assert_eq!(serde_json::to_value(&unsupported).unwrap(), before);
}

#[test]
fn descriptor_and_expired_work_checks_are_not_passing_evidence() {
    let mut result = SuggestSplitEnvelope::empty(Limits::default());
    assert!(result.account(128 * 1024 * 1024 + 1).is_err());
    let cancelled = AtomicBool::new(false);
    let error = result
        .fit(Controls {
            deadline: Instant::now(),
            cancelled: &cancelled,
        })
        .unwrap_err();
    assert_eq!(error.code, "planning_deadline");
}
