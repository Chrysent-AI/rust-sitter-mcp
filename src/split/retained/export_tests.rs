use super::*;
use crate::{move_plan::MoveRequest, split::retained::export::ExportRequest};
use std::os::unix::fs::{PermissionsExt, symlink};

fn export_request(retention: &Retention, ids: &[&str]) -> ExportRequest {
    serde_json::from_value(json!({"analysis_handle":retention.analysis_handle,"snapshot_id":retention.snapshot_id,
        "analysis_id":retention.analysis_id,"scope_input_digest":retention.scope_input_digest,
        "selection":ids.iter().map(|id| json!({"unit_ref":{"analysis_id":retention.analysis_id,"item_id":id},
            "destination":{"kind":"new_sibling","parent_path":"src/lib.rs","path":"src/helpers.rs"}})).collect::<Vec<_>>() })).unwrap()
}
fn inventory(store: &Store, retention: &Retention) -> Vec<Value> {
    store
        .detail(
            identity(retention, page(Collection::Inventory, 1000)),
            &AtomicBool::new(false),
        )
        .records
}
fn failure(store: &Store, request: ExportRequest, code: &str) {
    let result = store.export(request, &AtomicBool::new(false));
    assert_eq!(
        result.error.as_ref().map(|e| e.code.as_str()),
        Some(code),
        "{result:?}"
    );
    let value = serde_json::to_value(result).unwrap();
    assert!(value["request"].is_null());
    assert!(value["review"].is_null());
    assert_eq!(value["submitted"], false);
    assert_eq!(value["applicability"], "not_assessed");
}
const GENERIC: &str = "struct Worker<T>(T);\r\nimpl<T> Worker<T>\r\nwhere T: Copy\r\n{\r\n fn first() { Self::second(); }\r\n fn second() {}\r\n}\r\nfn alpha() {}\r\n";
#[test]
fn export_preserves_exact_generic_where_header_and_strict_defaults() {
    let repo = Fixture::new(GENERIC);
    let engine = Engine::new(repo.0.clone()).unwrap();
    let full = engine.suggest_split(repo.request(), &AtomicBool::new(false));
    let full = serde_json::to_value(full).unwrap();
    assert!(
        full["inventory"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["span"]["text"].is_null())
    );
    {
        // Retained metadata is the only identity; omitted display text is never transcribed.
        let store = &engine;
        let first = full["inventory"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["name"] == "first")
            .unwrap();
        let request: ExportRequest = serde_json::from_value(json!({"analysis_handle":full["retention"]["analysis_handle"],"snapshot_id":full["snapshot_id"],
            "selection":[{"unit_ref":{"analysis_id":full["retention"]["analysis_id"],"item_id":first["id"]},
                "destination":{"kind":"new_sibling","parent_path":"src/lib.rs","path":"src/helpers.rs"}}]})).unwrap();
        let result = store.export_move_request(request, &AtomicBool::new(false));
        assert!(result.error.is_none(), "{:?}", result.error);
        let value = serde_json::to_value(&result).unwrap();
        let strict = &value["request"];
        assert_eq!(
            strict["moves"][0]["item"]["expected_text"],
            "fn first() { Self::second(); }"
        );
        assert_eq!(
            strict["moves"][0]["enclosing_impl"]["expected_text"],
            "impl<T> Worker<T>\r\nwhere T: Copy\r\n"
        );
        assert_eq!(strict["moves"].as_array().unwrap().len(), 1);
        for field in [
            "assume_standard_prelude",
            "assume_declared_helpers",
            "acknowledge_test_consumers",
            "resolve_semantic",
            "semantic_configuration",
            "trivia_overrides",
            "rewrite_overrides",
            "draft_provenance",
        ] {
            assert!(strict.get(field).is_none(), "silently copied {field}");
        }
        assert_eq!(value["source_freshness"], "checked_at_export");
        assert_eq!(value["integrity"]["semantic"], "not_performed");
        assert!(result.counts.response_bytes >= wire_bytes(&result));
        let start = GENERIC.find("fn first()").unwrap();
        let header_start = GENERIC.find("impl<T>").unwrap();
        let header_end = GENERIC.find("{\r\n fn first").unwrap();
        let expected: MoveRequest = serde_json::from_value(json!({"repo_path":fs::canonicalize(&repo.0).unwrap(),"crate_root":"src/lib.rs","paths":["src"],"moves":[{
            "item":{"path":"src/worker.rs","range":{"start_byte":start,"end_byte":start+"fn first() { Self::second(); }".len()},"expected_text":"fn first() { Self::second(); }"},
            "enclosing_impl":{"path":"src/worker.rs","range":{"start_byte":header_start,"end_byte":header_end},"expected_text":&GENERIC[header_start..header_end]},
            "destination":{"kind":"new_sibling","parent_path":"src/lib.rs","path":"src/helpers.rs"}}]})).unwrap();
        assert_eq!(
            serde_json::to_value(result.request.unwrap()).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}
#[test]
fn export_selection_refusals_and_fixed_lifecycle() {
    let repo = Fixture::new(
        "struct Worker;\nimpl Worker { fn first() {} }\nuse core::mem;\nimpl Clone for Worker { fn clone(&self) -> Self { Worker } }\n",
    );
    let store = Store::default();
    let now = Instant::now();
    let retained = retain(&store, &repo, &repo.request(), now);
    let inv = inventory(&store, &retained);
    let member = inv.iter().find(|i| i["name"] == "first").unwrap()["id"]
        .as_str()
        .unwrap();
    let implementation = inv.iter().find(|i| i["kind"] == "impl_item").unwrap()["id"]
        .as_str()
        .unwrap();
    failure(
        &store,
        export_request(&retained, &[]),
        "INVALID_SCAFFOLD_SELECTION",
    );
    failure(
        &store,
        export_request(&retained, &[member, member]),
        "INVALID_SCAFFOLD_SELECTION",
    );
    failure(
        &store,
        export_request(&retained, &[member, implementation]),
        "INVALID_SCAFFOLD_SELECTION",
    );
    failure(
        &store,
        export_request(&retained, &["missing"]),
        "UNKNOWN_ADVICE_ID",
    );
    let excluded = inv.iter().find(|i| i["name"] == "clone").unwrap()["id"]
        .as_str()
        .unwrap();
    failure(
        &store,
        export_request(&retained, &[excluded]),
        "UNSUPPORTED_SCAFFOLD_SELECTION",
    );
    let import = inv.iter().find(|i| i["kind"] == "use_declaration").unwrap()["id"]
        .as_str()
        .unwrap();
    failure(
        &store,
        export_request(&retained, &[import]),
        "UNSUPPORTED_SCAFFOLD_SELECTION",
    );
    let mut wrong = export_request(&retained, &[member]);
    wrong.selection[0].unit_ref.analysis_id = "another-analysis".into();
    failure(&store, wrong, "INVALID_SCAFFOLD_SELECTION");
    let mut wrong = export_request(&retained, &[member]);
    wrong.snapshot_id = "wrong".into();
    failure(&store, wrong, "ADVICE_SNAPSHOT_MISMATCH");
    let mut wrong = export_request(&retained, &[member]);
    wrong.scope_input_digest = Some("wrong".into());
    failure(&store, wrong, "ADVICE_SNAPSHOT_MISMATCH");
    let mut wrong = export_request(&retained, &[member]);
    wrong.analysis_handle = "unknown".into();
    failure(&store, wrong, "ADVICE_SNAPSHOT_UNKNOWN");
    let result = store.export_at(
        export_request(&retained, &[member]),
        &AtomicBool::new(false),
        now + Duration::from_secs(901),
    );
    assert_eq!(result.error.unwrap().code, "ADVICE_SNAPSHOT_EXPIRED");
    assert!(result.request.is_none());
}
#[test]
fn export_reobserves_bytes_modes_ignore_admission_and_identity_without_substitution() {
    for mutation in 0..13 {
        let repo = Fixture::new("fn alpha() {}\n");
        if mutation >= 10 {
            fs::write(repo.0.join(".gitignore"), "# initial policy\n").unwrap();
        }
        let store = Store::default();
        let retained = retain(&store, &repo, &repo.request(), Instant::now());
        let inv = inventory(&store, &retained);
        let id = inv[0]["id"].as_str().unwrap();
        match mutation {
            0 => fs::write(
                repo.0.join("src/worker.rs"),
                "fn alpha() { /* changed */ }\n",
            )
            .unwrap(),
            1 => fs::set_permissions(
                repo.0.join("src/worker.rs"),
                fs::Permissions::from_mode(0o755),
            )
            .unwrap(),
            2 => fs::write(repo.0.join(".gitignore"), "# same admitted set\n").unwrap(),
            3 => fs::write(repo.0.join("src/.gitignore"), "worker.rs\n").unwrap(),
            4 => fs::write(repo.0.join("src/added.rs"), "fn added() {}\n").unwrap(),
            5 => fs::remove_file(repo.0.join("src/outside.rs")).unwrap(),
            6 => {
                fs::rename(repo.0.join("src/worker.rs"), repo.0.join("saved.rs")).unwrap();
                fs::write(repo.0.join("src/worker.rs"), "fn alpha() {}\n").unwrap();
            }
            7 => {
                fs::rename(repo.0.join("src/worker.rs"), repo.0.join("saved.rs")).unwrap();
                symlink("../saved.rs", repo.0.join("src/worker.rs")).unwrap();
            }
            8 => fs::write(repo.0.join("src/outside.rs"), "fn other() {}\n").unwrap(),
            9 => {
                fs::rename(repo.0.join("src"), repo.0.join("previous")).unwrap();
                fs::create_dir(repo.0.join("src")).unwrap();
                for file in ["worker.rs", "lib.rs", "outside.rs"] {
                    fs::copy(
                        repo.0.join("previous").join(file),
                        repo.0.join("src").join(file),
                    )
                    .unwrap();
                }
            }
            10 => fs::write(
                repo.0.join(".gitignore"),
                "# changed policy, same admission\n",
            )
            .unwrap(),
            11 => fs::remove_file(repo.0.join(".gitignore")).unwrap(),
            12 => fs::set_permissions(
                repo.0.join("src/worker.rs"),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap(),
            _ => unreachable!(),
        }
        failure(&store, export_request(&retained, &[id]), "SOURCE_CHANGED");
        let historical = store.detail(
            identity(
                &retained,
                Selector::Units {
                    item_ids: vec![id.into()],
                },
            ),
            &AtomicBool::new(false),
        );
        assert_eq!(
            historical.records[0]["item"]["expected_text"],
            "fn alpha() {}"
        );
    }
}
#[test]
fn export_honors_only_explicit_options_and_refuses_invalid_destinations_and_cancel() {
    let repo = Fixture::new("fn alpha() {}\n");
    let store = Store::default();
    let retained = retain(&store, &repo, &repo.request(), Instant::now());
    let inv = inventory(&store, &retained);
    let mut request = export_request(&retained, &[inv[0]["id"].as_str().unwrap()]);
    request.move_options = serde_json::from_value(json!({"context":{"before_lines":0,"after_lines":20},"limits":{"response_bytes":65536,"diagnostic_count":100000,"text_bytes":0},"max_moves":1})).unwrap();
    let result = store.export(request.clone(), &AtomicBool::new(false));
    assert!(result.error.is_none(), "{:?}", result.error);
    let strict = result.request.unwrap();
    assert_eq!(strict.context.before_lines, 0);
    assert_eq!(strict.context.after_lines, 20);
    assert_eq!(strict.limits.diagnostic_count, 100000);
    assert_eq!(strict.max_moves, 1);
    assert!(!strict.assume_standard_prelude && !strict.acknowledge_test_consumers);
    request.selection[0].destination = crate::move_plan::Destination::Existing {
        path: "../outside.rs".into(),
        before_item: None,
    };
    failure(&store, request.clone(), "INVALID_DESTINATION");
    let result = store.export(request, &AtomicBool::new(true));
    assert_eq!(result.error.unwrap().code, "CANCELLED");
    assert!(result.request.is_none());
}
#[test]
fn export_preserves_caller_order_destinations_and_outside_review_links() {
    let repo = Fixture::new(SOURCE);
    let store = Store::default();
    let retained = retain(&store, &repo, &repo.request(), Instant::now());
    let inv = inventory(&store, &retained);
    let ids: Vec<_> = ["Worker", "first"]
        .into_iter()
        .map(|name| {
            inv.iter().find(|i| i["name"] == name).unwrap()["id"]
                .as_str()
                .unwrap()
        })
        .collect();
    let mut request = export_request(&retained, &[ids[1], ids[0]]);
    let destination: crate::move_plan::Destination = serde_json::from_value(json!({"kind":"existing_impl","path":"src/outside.rs",
        "implementation":{"path":"src/outside.rs","range":{"start_byte":0,"end_byte":14},"expected_text":"impl Worker {}"}})).unwrap();
    request.selection[0].destination = destination.clone();
    let result = store.export(request, &AtomicBool::new(false));
    assert!(result.error.is_none(), "{:?}", result.error);
    let strict = result.request.as_ref().unwrap();
    assert_eq!(strict.moves.len(), 2);
    assert_eq!(
        strict.moves[0].item.expected_text,
        "fn first() { Self::second(); }"
    );
    assert_eq!(strict.moves[1].item.expected_text, "struct Worker;");
    assert_eq!(strict.moves[0].destination, destination);
    assert_eq!(result.review["selected_item_ids"], json!([ids[1], ids[0]]));
    assert!(!array(&result.review["companions_not_selected"]).is_empty());
    assert!(!array(&result.review["unresolved_decision_refs"]).is_empty());
    let value = serde_json::to_value(&result).unwrap();
    assert!(value["request"].get("review").is_none());
    let wire = rmcp::model::CallToolResult::structured(value);
    assert!(serde_json::to_vec(&wire).unwrap().len() + 4096 <= result.counts.response_bytes);
}
#[test]
fn export_missing_frozen_provenance_refuses_instead_of_reading_current_substitutes() {
    let repo = Fixture::new("fn alpha() {}\n");
    let store = Store::default();
    let request = repo.request();
    let (full, evidence) = repo.analyze(&request);
    let id = full.inventory[0].id.clone();
    let mut record = store
        .prepare(
            &full,
            evidence,
            &request,
            Instant::now(),
            controls(&AtomicBool::new(false)),
        )
        .unwrap();
    record.evidence.buffers.remove("src/worker.rs");
    let retained = store
        .publish(record, controls(&AtomicBool::new(false)), Instant::now())
        .unwrap();
    failure(
        &store,
        export_request(&retained, &[&id]),
        "UNSUPPORTED_SCAFFOLD_SELECTION",
    );
    assert_eq!(
        fs::read_to_string(repo.0.join("src/worker.rs")).unwrap(),
        "fn alpha() {}\n"
    );
}
#[test]
fn export_refuses_indivisible_wire_and_decoded_request_overflow_without_partial_json() {
    for (payload, response) in [
        ("x".repeat(40_000), 65536),
        ("\t".repeat(20_000), 65536),
        ("\t".repeat(4_200_000), 16 * 1024 * 1024),
    ] {
        let repo = Fixture::new(&format!("fn alpha() {{ /*{payload}*/ }}\n"));
        let store = Store::default();
        let mut analysis = repo.request();
        analysis.limits.max_file_bytes = 16 * 1024 * 1024;
        let retained = retain(&store, &repo, &analysis, Instant::now());
        // A page containing the complete giant unit is intentionally unavailable at this budget.
        let record = store
            .obtain(&identity(&retained, Selector::Release {}), Instant::now())
            .unwrap();
        let id = record.canonical["inventory"][0]["id"].as_str().unwrap();
        let mut request = export_request(&retained, &[id]);
        request.limits.response_bytes = response;
        let result = store.export(request, &AtomicBool::new(false));
        assert_eq!(result.error.unwrap().code, "SCAFFOLD_TOO_LARGE");
        assert!(result.request.is_none());
        if response == 16 * 1024 * 1024 {
            assert!(result.counts.request_bytes > 8 * 1024 * 1024);
        }
    }
}
