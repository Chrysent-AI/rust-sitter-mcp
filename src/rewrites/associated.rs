//! Written same-type header routes and explicit cross-module method visibility repairs.
use super::*;
use std::collections::BTreeSet;

impl Analyzer<'_> {
    fn associated_type(&self, path: &str, module: &[String], written: &str) -> Option<String> {
        if !written.contains("::") {
            let imports = self.imported(path, module, written);
            if imports.len() == 1 && !imports[0].conditioned {
                return self.imported_target(&imports[0]);
            }
            if !imports.is_empty() {
                return None;
            }
        }
        self.resolve_in(path, module, written, false)
    }
    pub(super) fn associated(&mut self) -> Result<Vec<Need>, DomainError> {
        let mut needs = Vec::new();
        let selected = self.selected;
        for (path, item, destination) in selected.iter().cloned() {
            items::check(self.controls.0, self.controls.1)?;
            let Some(implementation) = &item.enclosing_impl else {
                continue;
            };
            let Some(old) = self.contexts.get(&path) else {
                continue;
            };
            let Some(new) = self.final_contexts.get(&destination) else {
                continue;
            };
            let (old_module, new_module) =
                (old.module_segments.clone(), new.module_segments.clone());
            let mut need = Need {
                attribute_range: None,
                reason: DecisionReason::UnsupportedUnitKind,
                choice_target: None,
                lexical_uncertainty: None,
                refusal_basis: Vec::new(),
                category: "associated_context",
                path: path.clone(),
                range: item.span.range.clone(),
                message: "impl type has no unique admitted written nominal identity".into(),
                item_ids: vec![item.id.clone()],
            };
            if !item.reasons.is_empty() {
                continue;
            }
            let impl_node = self.parsed[&path]
                .tree
                .root_node()
                .named_descendant_for_byte_range(
                    implementation.range.start_byte,
                    implementation.range.end_byte,
                )
                .expect("impl");
            let body = impl_node.child_by_field_name("body").expect("body");
            let mut parameters = BTreeSet::new();
            if let Some(params) = impl_node.child_by_field_name("type_parameters") {
                for i in 0..params.named_child_count() {
                    if let Some(name) = params
                        .named_child(i as u32)
                        .and_then(|p| p.child_by_field_name("name"))
                    {
                        parameters.insert(self.files[&path].source[name.byte_range()].to_owned());
                    }
                }
            }
            let mut header_nodes = vec![impl_node];
            while let Some(node) = header_nodes.pop() {
                items::check(self.controls.0, self.controls.1)?;
                if node == body || node.kind() == "lifetime" {
                    continue;
                }
                if matches!(
                    node.kind(),
                    "type_identifier"
                        | "identifier"
                        | "scoped_type_identifier"
                        | "scoped_identifier"
                ) {
                    let written = &self.files[&path].source[node.byte_range()];
                    if parameters.contains(written)
                        || written == implementation.written_type
                        || matches!(
                            written,
                            "Self"
                                | "u8"
                                | "u16"
                                | "u32"
                                | "u64"
                                | "u128"
                                | "usize"
                                | "i8"
                                | "i16"
                                | "i32"
                                | "i64"
                                | "i128"
                                | "isize"
                                | "bool"
                                | "char"
                                | "str"
                                | "f32"
                                | "f64"
                        )
                    {
                        continue;
                    }
                    let route = self.associated_type(&path, &old_module, written);
                    let resolved = route
                        .as_deref()
                        .and_then(|r| self.written_target(&mut need, r, &new_module));
                    if let Some(resolved) = resolved
                        && let Some((p, declaration)) = self.declaration(&resolved.terminal)
                    {
                        let visible = self.visibility_need(
                            &mut need,
                            &p,
                            &declaration,
                            &new_module,
                            !resolved.evidence.fallback.is_empty(),
                        );
                        let carried = if written.contains("::") {
                            written.starts_with("crate::") && resolved.route == written
                        } else {
                            self.import(
                                &destination,
                                written,
                                &resolved.route,
                                &need.item_ids,
                                (
                                    anchor(self.files, &p, &declaration.span.range),
                                    resolved.evidence,
                                ),
                                vec![implementation.anchor.clone()],
                            )
                        };
                        if visible && carried {
                            continue;
                        }
                    }
                    let mut veto = need.clone();
                    veto.message = "impl header dependency cannot be preserved without transforming the written header".into();
                    needs.push(veto);
                    continue;
                }
                for i in 0..node.named_child_count() {
                    header_nodes.push(node.named_child(i as u32).expect("header child"));
                }
            }
            let route = self.associated_type(&path, &old_module, &implementation.written_type);
            let terminal = route
                .as_deref()
                .and_then(|r| self.follow_reexport(&mut need, r));
            let Some((terminal, route_anchors)) = terminal else {
                needs.push(need);
                continue;
            };
            let Some((type_path, ty)) = self.declaration(&terminal).filter(|(_, i)| {
                matches!(i.kind.as_str(), "struct_item" | "enum_item" | "union_item")
            }) else {
                needs.push(need);
                continue;
            };
            if let Some(crate::move_plan::Destination::ExistingImpl {
                implementation: target,
                ..
            }) = self
                .request
                .moves
                .iter()
                .find(|m| m.item.path == path && m.item.range == item.span.range)
                .map(|m| &m.destination)
            {
                let target_node = self.parsed[&destination]
                    .tree
                    .root_node()
                    .named_descendant_for_byte_range(target.range.start_byte, target.range.end_byte)
                    .expect("validated impl");
                let target_type = target_node.child_by_field_name("type").expect("impl type");
                let target_type = if target_type.kind() == "generic_type" {
                    target_type
                        .child_by_field_name("type")
                        .expect("nominal type")
                } else {
                    target_type
                };
                let target_route = self.associated_type(
                    &destination,
                    &new_module,
                    &self.files[&destination].source[target_type.byte_range()],
                );
                if target_route
                    .as_deref()
                    .and_then(|r| self.follow_reexport(&mut need, r))
                    .is_none_or(|(t, _)| t != terminal)
                {
                    need.message =
                        "identical written impl headers resolve to different or unproved types"
                            .into();
                    needs.push(need);
                    continue;
                }
            } else if !implementation.written_type.contains("::") {
                let evidence = anchor(self.files, &type_path, &ty.span.range);
                if !self.import(
                    &destination,
                    &implementation.written_type,
                    &terminal,
                    &need.item_ids,
                    (
                        evidence,
                        RouteEvidence {
                            anchors: route_anchors,
                            fallback: Vec::new(),
                        },
                    ),
                    vec![implementation.anchor.clone()],
                ) {
                    need.message = "impl header type import conflicts at destination".into();
                    needs.push(need);
                    continue;
                }
            } else if !implementation.written_type.starts_with("crate::") {
                need.message =
                    "relative impl type routes would require a header transformation".into();
                needs.push(need);
                continue;
            }
            if !self.visibility_need(&mut need, &type_path, &ty, &new_module, false) {
                needs.push(need);
                continue;
            }
            // Private moved declarations may have callers outside the admitted corpus.
            // Preserve their original defining scope's access explicitly, without inventing callers.
            if !self.widen(
                &path,
                Some(&item),
                &new_module,
                &old_module,
                &need.item_ids,
                None,
            ) {
                needs.push(need.clone());
            }
            let body = self.parsed[&path]
                .tree
                .root_node()
                .named_descendant_for_byte_range(
                    item.span.range.start_byte,
                    item.span.range.end_byte,
                )
                .expect("member");
            let mut stack = vec![body];
            while let Some(node) = stack.pop() {
                items::check(self.controls.0, self.controls.1)?;
                if matches!(
                    node.kind(),
                    "macro_invocation" | "macro_definition" | "token_tree"
                ) {
                    continue;
                }
                // Carry an evidenced written enum prefix provisionally; the existing
                // semantic need must still prove the variant at both overlays.
                if self.request.resolve_semantic
                    && node.kind() == "scoped_identifier"
                    && let (Some(prefix), Some(variant)) = (
                        node.child_by_field_name("path"),
                        node.child_by_field_name("name"),
                    )
                {
                    let name = &self.files[&path].source[prefix.byte_range()];
                    if !name.contains("::") {
                        let route = self.associated_type(&path, &old_module, name);
                        if let Some((target, anchors)) = route
                            .as_deref()
                            .and_then(|r| self.follow_reexport(&mut need, r))
                            && let Some((enum_path, declaration)) = self
                                .declaration(&target)
                                .filter(|(_, i)| i.kind == "enum_item")
                        {
                            let syntax = self.parsed[&enum_path]
                                .tree
                                .root_node()
                                .named_descendant_for_byte_range(
                                    declaration.span.range.start_byte,
                                    declaration.span.range.end_byte,
                                )
                                .expect("enum");
                            let variant_name = &self.files[&path].source[variant.byte_range()];
                            if syntax.child_by_field_name("body").is_some_and(|body| {
                                (0..body.named_child_count())
                                    .filter_map(|i| body.named_child(i as u32))
                                    .any(|v| {
                                        v.child_by_field_name("name").is_some_and(|n| {
                                            self.files[&enum_path].source[n.byte_range()]
                                                == *variant_name
                                        })
                                    })
                            }) {
                                let evidence =
                                    anchor(self.files, &enum_path, &declaration.span.range);
                                if !self.import(
                                    &destination,
                                    name,
                                    &target,
                                    &need.item_ids,
                                    (
                                        evidence,
                                        RouteEvidence {
                                            anchors,
                                            fallback: Vec::new(),
                                        },
                                    ),
                                    vec![anchor(
                                        self.files,
                                        &path,
                                        &span(prefix.start_byte(), prefix.end_byte()),
                                    )],
                                ) {
                                    let mut veto = need.clone();
                                    veto.reason = DecisionReason::DestinationBindingConflict;
                                    veto.message =
                                        "written enum prefix conflicts at destination".into();
                                    needs.push(veto);
                                }
                            }
                        }
                    }
                }
                if node.kind() == "field_expression"
                    && node
                        .child_by_field_name("value")
                        .is_some_and(|n| &self.files[&path].source[n.byte_range()] == "self")
                {
                    let name = node
                        .child_by_field_name("field")
                        .map(|n| self.files[&path].source[n.byte_range()].to_owned())
                        .unwrap_or_default();
                    let mut matches = Vec::new();
                    for (other_path, data) in self.parsed {
                        let Some(context) = self.contexts.get(other_path) else {
                            continue;
                        };
                        for member in &data.associated_items {
                            if member.name.as_deref() != Some(&name) {
                                continue;
                            }
                            let Some(parent) = &member.enclosing_impl else {
                                continue;
                            };
                            if self
                                .associated_type(
                                    other_path,
                                    &context.module_segments,
                                    &parent.written_type,
                                )
                                .as_deref()
                                .and_then(|r| self.follow_reexport(&mut need, r))
                                .is_some_and(|(t, _)| t == terminal)
                            {
                                matches.push((other_path.clone(), member.clone()));
                            }
                        }
                    }
                    if matches.len() == 1 {
                        let (p, member) = matches.remove(0);
                        let final_path = self.final_path(&p, &member).to_owned();
                        let defining = self.final_contexts[&final_path].module_segments.clone();
                        if !member.reasons.is_empty()
                            || !matches!(member.visibility_key, "private" | "pub(crate)" | "pub")
                            || !self.widen(
                                &p,
                                Some(&member),
                                &defining,
                                &new_module,
                                &need.item_ids,
                                None,
                            )
                        {
                            let mut veto = need.clone();
                            veto.reason = DecisionReason::VisibilityScopeUnproved;
                            veto.message = "cross-module inherent call requires an excluded or unsupported method visibility repair".into();
                            needs.push(veto);
                        }
                    }
                }
                for i in (0..node.named_child_count()).rev() {
                    stack.push(node.named_child(i as u32).expect("child"));
                }
            }
        }
        Ok(needs)
    }
}
