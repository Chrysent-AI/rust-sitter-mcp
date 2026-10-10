//! Revalidate moved pattern binders against immutable assembled bytes.
use super::*;
use tree_sitter::Node;

fn mapped(
    location: &items::LexicalLocation,
    destination: &str,
    files: &BTreeMap<String, FileSnapshot>,
    final_files: &BTreeMap<String, FileSnapshot>,
    outputs: &BTreeMap<String, (String, Vec<trivia::Trivia>)>,
    origins: &[MoveOrigin],
    controls: (Instant, &AtomicBool),
) -> Result<Option<ByteRange>, DomainError> {
    items::check(controls.0, controls.1)?;
    let original = &location.range;
    if location.path == destination && !outputs.contains_key(destination) {
        return Ok(Some(original.clone()));
    }
    let mut found = None;
    for origin in origins {
        items::check(controls.0, controls.1)?;
        if origin.output_path != destination {
            continue;
        }
        if let Some((_, range)) =
            origin.mapped(&location.path, &(original.start_byte..original.end_byte))
        {
            if found.is_some()
                || origin.source_range.end_byte - origin.source_range.start_byte
                    != origin.output_range.end_byte - origin.output_range.start_byte
            {
                return Ok(None);
            }
            let before = files
                .get(&location.path)
                .and_then(|f| f.source.get(original.start_byte..original.end_byte));
            let after = final_files
                .get(destination)
                .and_then(|f| f.source.get(range.clone()));
            if before.is_none() || before != after {
                return Ok(None);
            }
            found = Some(super::range(range.start, range.end));
        }
    }
    Ok(found)
}
fn exact<'a>(tree: &'a tree_sitter::Tree, range: &ByteRange, kind: &str) -> Option<Node<'a>> {
    let mut node = tree
        .root_node()
        .named_descendant_for_byte_range(range.start_byte, range.end_byte)?;
    // Pattern wrappers can have exactly the same range as their child. Select
    // the witnessed CST kind, not whichever equal-range descendant is deepest.
    while node.start_byte() == range.start_byte && node.end_byte() == range.end_byte {
        if node.kind() == kind {
            return (!node.has_error() && !node.is_missing()).then_some(node);
        }
        node = node.parent()?;
    }
    None
}
fn authored_range(
    id: &str,
    edits: &[Edit],
    result: &MoveEnvelope,
    controls: (Instant, &AtomicBool),
) -> Result<Option<(String, ByteRange, String)>, DomainError> {
    let Some(audit) = result.plan.rewrites.iter().find(|r| r.id == id) else {
        return Ok(None);
    };
    let [
        ArtifactLink::Edit {
            index,
            replacement_range,
        },
    ] = audit.artifact_links.as_slice()
    else {
        return Ok(None);
    };
    let Some(edit) = edits.get(*index) else {
        return Ok(None);
    };
    let mut at = edit.range.start_byte;
    for prior in &edits[..*index] {
        items::check(controls.0, controls.1)?;
        if prior.path == edit.path {
            at =
                at + prior.replacement_text.len() - (prior.range.end_byte - prior.range.start_byte);
        }
    }
    if edit
        .replacement_text
        .get(replacement_range.start_byte..replacement_range.end_byte)
        != Some(audit.after_text.as_str())
    {
        return Ok(None);
    }
    Ok(Some((
        edit.path.clone(),
        range(
            at + replacement_range.start_byte,
            at + replacement_range.end_byte,
        ),
        audit.after_text.clone(),
    )))
}
#[allow(clippy::too_many_arguments)]
pub(super) fn audit(
    request: &MoveRequest,
    files: &BTreeMap<String, FileSnapshot>,
    parsed: &BTreeMap<String, items::ParsedFile>,
    outputs: &BTreeMap<String, (String, Vec<trivia::Trivia>)>,
    edits: &[Edit],
    creations: &BTreeMap<String, Creation>,
    proof: (&[items::PatternBinding], &BTreeMap<String, ModuleEvidence>),
    controls: (Instant, &AtomicBool),
    result: &mut MoveEnvelope,
) -> Result<(), DomainError> {
    let (ledger, contexts) = proof;
    if ledger.is_empty() {
        return Ok(());
    }
    let mut final_files = BTreeMap::new();
    let mut bytes = 0;
    for (path, file) in files {
        items::check(controls.0, controls.1)?;
        let text = outputs.get(path).map_or(&file.source, |o| &o.0);
        bytes += path.len() + text.len();
        if bytes > 128 * 1024 * 1024 {
            return Err(DomainError::new(
                "analysis_descriptor_bytes",
                "final glob corpus guard reached",
            ));
        }
        final_files.insert(
            path.clone(),
            FileSnapshot {
                path: path.clone(),
                source: text.clone(),
                mode: file.mode,
            },
        );
    }
    for (path, (text, _)) in outputs {
        items::check(controls.0, controls.1)?;
        if !final_files.contains_key(path) {
            bytes += path.len() + text.len();
            if bytes > 128 * 1024 * 1024 {
                return Err(DomainError::new(
                    "analysis_descriptor_bytes",
                    "final glob corpus guard reached",
                ));
            }
            final_files.insert(
                path.clone(),
                FileSnapshot {
                    path: path.clone(),
                    source: text.clone(),
                    mode: 0o100644,
                },
            );
        }
    }
    let mut final_parsed = BTreeMap::new();
    let mut observed = 0;
    for (path, file) in &final_files {
        items::check(controls.0, controls.1)?;
        let data = if outputs.contains_key(path) {
            items::parse(file, 0, controls.0, controls.1, &mut observed)?
        } else {
            // Unchanged bytes retain the captured CST and inventory. Tree clones
            // share immutable storage; no filesystem discovery or reparse.
            parsed[path].clone()
        };
        final_parsed.insert(path.clone(), data);
    }
    // Preserve captured physical-layout admission; validate edges in final CSTs,
    // never ask the filesystem to resolve virtual files.
    let mut final_contexts = contexts.clone();
    for evidence in final_contexts.values_mut() {
        for anchor in &mut evidence.declaration_anchors {
            items::check(controls.0, controls.1)?;
            let location = items::LexicalLocation {
                path: anchor.path.clone(),
                range: anchor.range.clone(),
                kind: "mod_item".into(),
            };
            let mapped = mapped(
                &location,
                &anchor.path,
                files,
                &final_files,
                outputs,
                &result.plan.origins,
                controls,
            )?;
            if let Some(mapped) = mapped
                && exact(&final_parsed[&anchor.path].tree, &mapped, "mod_item").is_some()
            {
                anchor.range = mapped;
            } else {
                evidence
                    .unresolved
                    .push("final captured edge is not exact".into());
            }
        }
    }
    for (path, creation) in creations {
        items::check(controls.0, controls.1)?;
        let Some(DeclarationLink::Synthesized { rewrite_id }) = &creation.link else {
            continue;
        };
        let Some(context) = final_contexts.get_mut(path) else {
            continue;
        };
        let mut admitted = false;
        if let Some((parent, mut range, mut expected)) =
            authored_range(rewrite_id, edits, result, controls)?
            && parent == creation.parent
        {
            if let Some(id) = &creation.visibility_id {
                if let Some((vp, vr, text)) = authored_range(id, edits, result, controls)?
                    && vp == parent
                    && vr.end_byte == range.start_byte
                {
                    range.start_byte = vr.start_byte;
                    expected = format!("{text}{expected}");
                } else {
                    context
                        .unresolved
                        .push("final creation access link is not exact".into());
                }
            }
            if let Some(node) = exact(&final_parsed[&parent].tree, &range, "mod_item") {
                let source = &final_files[&parent].source;
                let name = items::module_name(path)?;
                let matches = final_parsed[&parent]
                    .items
                    .iter()
                    .filter(|i| i.name.as_deref().map(|s| s.trim_start_matches("r#")) == Some(name))
                    .count();
                let ordinary = items::child_path(&parent, &request.crate_root, name);
                if matches == 1
                    && node.child_by_field_name("body").is_none()
                    && source.get(node.byte_range()) == Some(expected.as_str())
                    && (path == &ordinary
                        || path == &format!("{}/mod.rs", ordinary.trim_end_matches(".rs")))
                {
                    context.declaration_anchors.push(SourceAnchor {
                        path: parent,
                        range,
                        expected_text: expected,
                    });
                    admitted = true;
                }
            }
        }
        if !admitted {
            context
                .unresolved
                .push("final creation declaration link is not exact".into());
        }
    }
    let routes = items::GlobRoutes::strict(
        &final_files,
        &final_parsed,
        &final_contexts,
        &request.crate_root,
        controls,
    )?;
    for entry in ledger {
        items::check(controls.0, controls.1)?;
        let destination = &entry.destination;
        let valid = if let Some(data) = final_parsed.get(destination) {
            let mut nodes = Vec::new();
            for witness in [&entry.identifier, &entry.pattern] {
                items::check(controls.0, controls.1)?;
                let range = mapped(
                    witness,
                    destination,
                    files,
                    &final_files,
                    outputs,
                    &result.plan.origins,
                    controls,
                )?;
                nodes.push(range.and_then(|r| exact(&data.tree, &r, &witness.kind)));
            }
            if let [Some(identifier), Some(pattern)] = nodes.as_slice() {
                let source = &final_files[destination].source;
                let mut valid = items::final_pattern_binding(
                    destination,
                    *identifier,
                    *pattern,
                    &entry.scope.kind,
                    source,
                    &routes,
                    !entry.routes.is_empty(),
                    controls,
                )?;
                for witness in &entry.references {
                    items::check(controls.0, controls.1)?;
                    let node = mapped(
                        witness,
                        destination,
                        files,
                        &final_files,
                        outputs,
                        &result.plan.origins,
                        controls,
                    )?
                    .and_then(|r| exact(&data.tree, &r, &witness.kind));
                    if let Some(node) = node {
                        let assessment = items::lexical_assessment_with_globs(
                            destination,
                            node,
                            source,
                            &entry.spelling,
                            controls,
                            false,
                            None,
                            Some(&routes),
                        )?;
                        valid &= assessment.binding == items::LexicalBinding::Independent
                            && assessment.definite_binding.is_some_and(|b| {
                                b.range.start_byte == identifier.start_byte()
                                    && b.range.end_byte == identifier.end_byte()
                            });
                    } else {
                        valid = false;
                    }
                }
                valid
            } else {
                false
            }
        } else {
            false
        };
        if !valid {
            let original = &entry.identifier;
            let message = "moved pattern binding is not proved in the complete final written context; artifacts withheld";
            result.blocker(
                "BINDING_COLLISION",
                message,
                Some(&original.path),
                Some(original.range.clone()),
            );
            result.structural_decision(
                "binding_collision",
                DecisionReason::LexicalContextUnproved,
                vec![anchor(
                    &original.path,
                    &files[&original.path].source,
                    &original.range,
                )],
                vec![entry.item_id.clone()],
                message,
            )?;
            result
                .plan
                .decisions
                .last_mut()
                .expect("final pattern refusal")
                .lexical_uncertainty = Some(items::LexicalUncertainty {
                spelling: entry.spelling.clone(),
                reason: items::LexicalReason::IdentifierPatternBindingOrConstant,
                scope: entry.scope.clone(),
                pattern: Some(entry.identifier.clone()),
                witness_relation: None,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "pattern_tests.rs"]
mod pattern_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapping_requires_one_exact_copy_and_one_exact_cst_node() {
        let cancelled = AtomicBool::new(false);
        let controls = (
            Instant::now() + std::time::Duration::from_secs(10),
            &cancelled,
        );
        let mut original = BTreeMap::new();
        original.insert(
            "source.rs".into(),
            FileSnapshot {
                path: "source.rs".into(),
                source: "binding".into(),
                mode: 0o100644,
            },
        );
        let mut final_files = BTreeMap::new();
        final_files.insert(
            "target.rs".into(),
            FileSnapshot {
                path: "target.rs".into(),
                source: "binding".into(),
                mode: 0o100644,
            },
        );
        let location = items::LexicalLocation {
            path: "source.rs".into(),
            range: range(0, 7),
            kind: "identifier".into(),
        };
        let outputs = BTreeMap::from([("target.rs".into(), ("binding".into(), Vec::new()))]);
        let origin = MoveOrigin {
            id: String::new(),
            source_path: "source.rs".into(),
            source_range: range(0, 7),
            output_path: "target.rs".into(),
            output_range: range(0, 7),
            role: "item".into(),
        };
        let query = |origins: &[MoveOrigin]| {
            mapped(
                &location,
                "target.rs",
                &original,
                &final_files,
                &outputs,
                origins,
                controls,
            )
            .unwrap()
        };
        assert_eq!(query(std::slice::from_ref(&origin)), Some(range(0, 7)));
        assert!(query(&[]).is_none());
        assert!(query(&[origin.clone(), origin.clone()]).is_none());
        let mut wrong = origin.clone();
        wrong.output_range = range(0, 6);
        assert!(query(&[wrong]).is_none());
        let mut partial = origin.clone();
        partial.source_range = range(0, 6);
        assert!(query(&[partial]).is_none());
        final_files.get_mut("target.rs").unwrap().source = "changed".into();
        assert!(
            mapped(
                &location,
                "target.rs",
                &original,
                &final_files,
                &outputs,
                std::slice::from_ref(&origin),
                controls
            )
            .unwrap()
            .is_none()
        );
        let text = "fn selected() { let [binding] = [0]; binding; }";
        let tree = trivia::parse(text, controls.0, controls.1)
            .unwrap()
            .unwrap();
        let at = text.find("binding").unwrap();
        assert!(exact(&tree, &range(at, at + 7), "identifier").is_some());
        assert!(exact(&tree, &range(at, at + 6), "identifier").is_none());
        assert!(exact(&tree, &range(at, at + 7), "slice_pattern").is_none());
        cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            mapped(
                &location,
                "target.rs",
                &original,
                &final_files,
                &outputs,
                &[origin],
                controls
            )
            .unwrap_err()
            .code,
            "CANCELLED"
        );
    }
}
