use super::*;

/// Read-only measurement of compact wire bytes versus conservative owned retention charges.
#[test]
#[ignore = "set SPLIT_MEASURE_REPO to a caller repository for read-only measurement"]
fn measure_coupled_manifest_and_retention() {
    let root = PathBuf::from(std::env::var("SPLIT_MEASURE_REPO").expect("measurement repository"));
    for source in [
        "src/move_plan.rs",
        "src/rewrites.rs",
        "src/items.rs",
        "src/split.rs",
        "src/split/retained.rs",
        "src/split/test_observations.rs",
        "src/items/globs.rs",
        "src/scope.rs",
        "src/move_plan/semantic.rs",
        "src/move_plan/prelude.rs",
        "src/items/lexical.rs",
        "src/split/signals.rs",
    ] {
        let request: SuggestSplitRequest = serde_json::from_value(json!({
            "repo_path":root,"crate_root":"src/lib.rs","source_path":source,"paths":["src"],
            "retain_snapshot":true,"response_mode":"compact",
            "limits":{"diagnostic_count":0,"text_bytes":0,"response_bytes":16777216,"time_budget_ms":300000}
        })).unwrap();
        let cancelled = AtomicBool::new(false);
        let controls = Controls {
            deadline: Instant::now() + Duration::from_secs(300),
            cancelled: &cancelled,
        };
        let (full, evidence) = analyze_with_recheck(&root, &request, &cancelled, || {});
        assert_eq!(full.status, "complete", "{source}: {:?}", full.error);
        let evidence = evidence.unwrap();
        let reserve =
            preparation_allocation(&full, &evidence, &request, usize::MAX, controls).unwrap();
        let evidence_bytes = evidence_allocation(&evidence);
        let store = Store::new(RetentionLimits {
            aggregate_bytes: reserve,
            ..RetentionLimits::default()
        });
        let record = store
            .prepare(&full, evidence, &request, Instant::now(), controls)
            .unwrap();
        let manifest = Manifest::from_full(&full, record.retention.clone());
        let value = serde_json::to_value(&manifest).unwrap();
        // A counterfactual only: removing required inline fields is not an additive
        // product change. Quantify the payoff without changing any response contract.
        let mut referenced = value.clone();
        let mut summaries = BTreeMap::new();
        let mut table = Vec::new();
        fn intern(
            value: &mut Value,
            summaries: &mut BTreeMap<String, usize>,
            table: &mut Vec<Value>,
        ) {
            match value {
                Value::Array(values) => {
                    for value in values {
                        intern(value, summaries, table);
                    }
                }
                Value::Object(object) => {
                    if let Some(summary) = object.remove("consequence_summary") {
                        let key = summary.to_string();
                        let next = table.len();
                        let index = *summaries.entry(key).or_insert_with(|| {
                            table.push(summary);
                            next
                        });
                        object.insert("consequence_summary_ref".into(), json!(index));
                    }
                    for value in object.values_mut() {
                        intern(value, summaries, table);
                    }
                }
                _ => {}
            }
        }
        // Candidate summary entries repeat the same objects with a different key.
        for entry in referenced["consequence_summaries"].as_array_mut().unwrap() {
            let summary = entry.as_object_mut().unwrap().remove("summary").unwrap();
            entry["consequence_summary"] = summary;
        }
        intern(&mut referenced, &mut summaries, &mut table);
        referenced["consequence_summary_table"] = json!(table);
        let fields: BTreeMap<_, _> = value
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::to_vec(v).unwrap().len()))
            .collect();
        let allocation_fields: BTreeMap<_, _> = record
            .canonical
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), allocation(v)))
            .collect();
        println!(
            "MEASUREMENT {}",
            json!({"source":source,"compact_json_bytes":serde_json::to_vec(&value).unwrap().len(),"compact_wire_bytes":wire_bytes(&manifest),"field_json_bytes":fields,"preparation_reservation":reserve,"retained_accounted_bytes":record.retention.accounted_bytes,"canonical_allocation":allocation(&record.canonical),"evidence_allocation":evidence_bytes,"metadata_allocation":retention_allocation(&record.retention),"counterfactual_reference_wire_bytes":wire_bytes(&referenced),"counterfactual_reference_json_bytes":serde_json::to_vec(&referenced).unwrap().len(),"unique_summary_count":table.len(),"summary_table_json_bytes":serde_json::to_vec(&table).unwrap().len(),"canonical_field_allocation":allocation_fields,"default_retention_fits":reserve <= DEFAULT_AGGREGATE_BYTES})
        );
    }
}
