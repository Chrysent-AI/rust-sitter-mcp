#[path = "support/fixture_gen.rs"]
mod fixture_gen;
#[path = "support/move_artifacts.rs"]
mod move_artifacts;
use fixture_gen::{Fixture, observe};
use rust_sitter_mcp::engine::Engine;
use serde_json::{Value, json};
use std::{fs, sync::atomic::AtomicBool};

const ROOT: &str = "cases/associated/lib.rs";
const SERVE: &str = "cases/associated/serve.rs";
const ROUTING: &str = "cases/associated/routing.rs";

fn fixture() -> Fixture {
    let repo = Fixture::generate();
    repo.write(ROOT, "mod serve; mod routing;\npub struct ConnectionLifetimeLimits { pub age: u8 }\npub struct MaxConnectionAge { pub age: u8 }\npub struct Serve<'a, L>(&'a L);\npub struct MethodRouter<A>(A);\npub struct AllowHeader;\npub trait Route { fn route(&self) -> u8; }\n");
    repo.write(
        SERVE,
        include_str!("fixtures/associated-inventory/serve.rs"),
    );
    repo.write(
        ROUTING,
        include_str!("fixtures/associated-inventory/routing.rs"),
    );
    repo
}

fn advice(repo: &Fixture, source: &str) -> Value {
    let result = Engine::new(repo.0.clone()).unwrap().suggest_split(
        serde_json::from_value(json!({"repo_path":repo.0,"crate_root":ROOT,
            "source_path":source,"paths":["cases/associated"],
            "limits":{"text_bytes":0,"diagnostic_count":100000}}))
        .unwrap(),
        &AtomicBool::new(false),
    );
    serde_json::to_value(result).unwrap()
}

fn selected<'a>(advice: &'a Value, name: &str) -> &'a Value {
    advice["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == name)
        .unwrap()
}

fn movement(repo: &Fixture, item: &Value) -> Value {
    let path = item["path"].as_str().unwrap();
    let text = fs::read_to_string(repo.0.join(path)).unwrap();
    let range = &item["span"]["range"];
    let start = range["start_byte"].as_u64().unwrap() as usize;
    let end = range["end_byte"].as_u64().unwrap() as usize;
    let mut movement = json!({"item":{"path":path,"range":range,
        "expected_text":&text[start..end]},
        "destination":{"kind":"new_sibling","path":"cases/associated/moved.rs","parent_path":ROOT}});
    if let Some(implementation) = item.get("enclosing_impl") {
        movement["enclosing_impl"] = implementation["anchor"].clone();
    }
    movement
}

fn plan(repo: &Fixture, moves: Vec<Value>) -> Value {
    serde_json::to_value(
        Engine::new(repo.0.clone()).unwrap().move_item(
            serde_json::from_value(json!({"repo_path":repo.0,"crate_root":ROOT,
            "paths":["cases/associated"],"moves":moves}))
            .unwrap(),
            &AtomicBool::new(false),
        ),
    )
    .unwrap()
}

#[test]
fn every_written_impl_member_has_context_and_excluded_members_are_retained() {
    let repo = fixture();
    let before = observe(&repo.0);
    for (path, expected) in [
        (
            SERVE,
            vec![
                "max_connection_age",
                "identity",
                "UNLIMITED",
                "age",
                "local_addr",
                "debug_addr",
                "state",
            ],
        ),
        (
            ROUTING,
            vec![
                "merge",
                "identity",
                "on",
                "debug_route",
                "count",
                "route",
                "conditional_route",
            ],
        ),
    ] {
        let result = advice(&repo, path);
        assert_eq!(result["status"], "complete", "{result}");
        assert_eq!(result["integrity"]["syntax"], "input_checked");
        let inventory = result["inventory"].as_array().unwrap();
        let members: Vec<_> = inventory
            .iter()
            .filter(|item| item.get("enclosing_impl").is_some())
            .collect();
        assert_eq!(
            members
                .iter()
                .map(|item| item["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            expected
        );
        let source = fs::read_to_string(repo.0.join(path)).unwrap();
        let interpretation = json!({
            "basis":"original_descriptor_ranges",
            "additivity":"non_additive",
            "generated_module_size":"not_estimated"
        });
        let overlaps = result["overlaps"].as_array().unwrap();
        assert_eq!(overlaps.len(), members.len());
        for member in members {
            assert_eq!(member["eligibility"], "context_sensitive", "{member}");
            let implementation = &member["enclosing_impl"];
            let anchor = &implementation["anchor"];
            assert_eq!(anchor["path"], path);
            let start = anchor["range"]["start_byte"].as_u64().unwrap() as usize;
            let end = anchor["range"]["end_byte"].as_u64().unwrap() as usize;
            assert_eq!(anchor["expected_text"], &source[start..end]);
            assert_eq!(implementation["header"], anchor["expected_text"]);
            assert_eq!(&source[end..end + 1], "{");
            assert_eq!(implementation["range"]["start_byte"], start);
            assert!(end < member["span"]["range"]["start_byte"].as_u64().unwrap() as usize);
            assert!(
                member["span"]["range"]["end_byte"].as_u64().unwrap()
                    < implementation["range"]["end_byte"].as_u64().unwrap()
            );
            let owner = inventory
                .iter()
                .find(|item| {
                    item["kind"] == "impl_item" && item["span"]["range"] == implementation["range"]
                })
                .unwrap();
            assert_eq!(member["enclosing_impl_id"], owner["id"]);
            let links = member["overlap_ids"].as_array().unwrap();
            assert_eq!(links.len(), 1);
            let overlap = overlaps.iter().find(|o| o["id"] == links[0]).unwrap();
            assert_eq!(overlap["relation"], "member_contained_in_impl");
            assert_eq!(overlap["path"], path);
            assert_eq!(overlap["item_ids"], json!([owner["id"], member["id"]]));
            assert_eq!(
                overlap["ranges"],
                json!([owner["span"]["range"], member["span"]["range"]])
            );
            assert!(
                owner["overlap_ids"]
                    .as_array()
                    .unwrap()
                    .contains(&overlap["id"])
            );
            assert_eq!(member["size_interpretation"], interpretation);
            assert_eq!(owner["size_interpretation"], interpretation);
            assert_eq!(
                overlap["draft_groups"].as_array().unwrap().len(),
                result["drafts"].as_array().unwrap().len()
            );
            for draft in result["drafts"].as_array().unwrap() {
                if !member["reasons"].as_array().unwrap().is_empty() {
                    assert!(
                        draft["groups"][0]["item_ids"]
                            .as_array()
                            .unwrap()
                            .contains(&member["id"])
                    );
                }
                let owner = inventory
                    .iter()
                    .find(|item| {
                        item["kind"] == "impl_item"
                            && item["span"]["range"] == implementation["range"]
                    })
                    .unwrap();
                assert!(
                    draft["groups"][0]["item_ids"]
                        .as_array()
                        .unwrap()
                        .contains(&owner["id"])
                );
            }
        }
        assert!(!result["drafts"].as_array().unwrap().is_empty());
        for item in inventory {
            let start = item["span"]["range"]["start_byte"].as_u64().unwrap() as usize;
            let end = item["span"]["range"]["end_byte"].as_u64().unwrap() as usize;
            assert_eq!(item["bytes"], end - start);
            let start_line = source[..start].bytes().filter(|b| *b == b'\n').count();
            let end_line = source[..end].bytes().filter(|b| *b == b'\n').count();
            assert_eq!(item["lines"], end_line - start_line + 1);
        }
        let mut same_group = false;
        let mut cross_group = false;
        for draft in result["drafts"].as_array().unwrap() {
            let groups = draft["groups"].as_array().unwrap();
            let mut seen = std::collections::BTreeSet::new();
            for group in groups {
                let mut bytes = 0;
                let mut lines = 0;
                for id in group["item_ids"].as_array().unwrap() {
                    assert!(seen.insert(id.as_str().unwrap()));
                    let item = inventory.iter().find(|item| item["id"] == *id).unwrap();
                    bytes += item["bytes"].as_u64().unwrap();
                    lines += item["lines"].as_u64().unwrap();
                }
                assert_eq!(
                    group["sizes"]["items"],
                    group["item_ids"].as_array().unwrap().len()
                );
                assert_eq!(group["sizes"]["bytes"], bytes);
                assert_eq!(group["sizes"]["lines"], lines);
                let expected: Vec<_> = overlaps
                    .iter()
                    .filter(|o| {
                        o["item_ids"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|id| group["item_ids"].as_array().unwrap().contains(id))
                    })
                    .map(|o| o["id"].clone())
                    .collect();
                assert_eq!(group["overlap_ids"], json!(expected));
                if !expected.is_empty() {
                    assert_eq!(group["size_interpretation"], interpretation);
                }
            }
            assert_eq!(seen.len(), inventory.len());
            for overlap in overlaps {
                let link = overlap["draft_groups"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|link| link["draft_id"] == draft["id"])
                    .unwrap();
                let owner_group = link["impl_group_index"].as_u64().unwrap() as usize;
                let member_group = link["member_group_index"].as_u64().unwrap() as usize;
                assert!(
                    groups[owner_group]["item_ids"]
                        .as_array()
                        .unwrap()
                        .contains(&overlap["item_ids"][0])
                );
                assert!(
                    groups[member_group]["item_ids"]
                        .as_array()
                        .unwrap()
                        .contains(&overlap["item_ids"][1])
                );
                same_group |= owner_group == member_group;
                cross_group |= owner_group != member_group;
            }
        }
        assert!(
            same_group && cross_group,
            "both overlap shapes must be disclosed"
        );
        let admitted = inventory
            .iter()
            .filter(|item| {
                item["eligibility"] == "supported_unit"
                    || (item.get("enclosing_impl").is_some() && item["reasons"] == json!([]))
            })
            .count();
        assert_eq!(result["counts"]["eligible_items"], admitted);
        // Containment does not turn an otherwise selectable member into an exclusion.
        assert_eq!(selected(&result, "identity")["reasons"], json!([]));
        let conditional = if path == SERVE { "debug_addr" } else { "merge" };
        assert!(
            selected(&result, conditional)["reasons"]
                .as_array()
                .unwrap()
                .contains(&json!("conditional_or_unexamined_member_attribute"))
        );
        if path == ROUTING {
            assert!(
                selected(&result, "on")["reasons"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("generic_member"))
            );
            assert!(
                selected(&result, "route")["enclosing_impl"]["exclusions"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("trait_impl"))
            );
            assert!(
                selected(&result, "conditional_route")["enclosing_impl"]["exclusions"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("attributed_impl"))
            );
        }
    }
    assert_eq!(observe(&repo.0), before);
}

#[test]
fn serve_advice_builds_top_level_and_associated_batches_without_invalid_selection() {
    let repo = fixture();
    let before = observe(&repo.0);
    let result = advice(&repo, SERVE);
    let top_level: Vec<_> = result["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["eligibility"] == "supported_unit")
        .map(|item| movement(&repo, item))
        .collect();
    assert_eq!(top_level.len(), 2);
    assert!(
        top_level
            .iter()
            .all(|item| item.get("enclosing_impl").is_none())
    );
    let top_level_plan = plan(&repo, top_level.clone());
    assert_eq!(top_level_plan["status"], "complete", "{top_level_plan}");
    assert_eq!(
        top_level_plan["plan"]["applicable"], true,
        "{top_level_plan}"
    );

    let missing_context = plan(
        &repo,
        vec![json!({
            "item":move_artifacts::anchor(&repo, SERVE, "fn identity(age: u8) -> u8 { age }"),
            "destination":{"kind":"new_sibling","path":"cases/associated/moved.rs","parent_path":ROOT}
        })],
    );
    assert_eq!(missing_context["plan"]["applicable"], false);
    assert_eq!(missing_context["status"], "failed");
    assert_eq!(missing_context["error"]["code"], "INVALID_ITEM_SELECTION");

    let mut combined = top_level;
    for name in ["identity", "UNLIMITED"] {
        let member = movement(&repo, selected(&result, name));
        assert!(member["enclosing_impl"].is_object());
        combined.push(member);
    }
    let replay = plan(&repo, combined);
    assert_eq!(replay["status"], "complete", "{replay}");
    assert_eq!(replay["plan"]["applicable"], true, "{replay}");
    assert_eq!(replay["plan"]["integrity"]["semantic"], "not_performed");
    let copy = move_artifacts::apply(&repo, &replay);
    let moved = fs::read_to_string(copy.0.join("cases/associated/moved.rs")).unwrap();
    assert!(moved.contains("impl ConnectionLifetimeLimits {"));
    assert!(moved.contains("fn identity(age: u8) -> u8 { age }"));
    assert!(moved.contains("const UNLIMITED: u8 = 0;"));
    assert_eq!(observe(&repo.0), before);
}
