//! Explicit parent re-exports share the written declaration/route index.
//! The ledger records only unchanged old bindings; it is not a new resolver.
use super::*;

#[derive(Clone)]
pub(super) struct Facade {
    pub anchor: SourceAnchor,
    // None is the original public region; Some([]) is crate-local.
    pub region: Option<Vec<String>>,
    target: RewriteTarget,
    text: String,
    visibility: Option<(RewriteTarget, String)>,
}
impl Analyzer<'_> {
    pub(super) fn facade_for(&self, path: &str, item: &Item) -> Option<&Facade> {
        let name = item.name.as_deref()?;
        self.facades.get(&canonical(self.contexts.get(path)?, name))
    }
    fn facade_failure(
        &self,
        path: &str,
        item: &Item,
        reason: DecisionReason,
        message: &str,
    ) -> Need {
        Need {
            attribute_range: None,
            reason,
            choice_target: None,
            lexical_uncertainty: None,
            refusal_basis: vec![items::RefusalBasis::new(
                "parent_facade_unproved",
                path,
                Some(item.span.range.clone()),
            )],
            category: "reexport_dependency",
            path: path.into(),
            range: item.span.range.clone(),
            message: message.into(),
            item_ids: vec![item.id.clone()],
        }
    }
    pub(super) fn parent_facades(&mut self) -> Result<Vec<Need>, DomainError> {
        let mut failures = Vec::new();
        if !self.request.parent_facade {
            return Ok(failures);
        }
        for (path, item, destination) in self.selected {
            items::check(self.controls.0, self.controls.1)?;
            if !self.request.moves.iter().any(|m| {
                matches!(&m.destination,
                crate::move_plan::Destination::NewChild { path: child, parent_path }
                    if child == destination && parent_path == path)
            }) {
                continue;
            }
            let reject = |reason, message| self.facade_failure(path, item, reason, message);
            let Some(name) = item
                .name
                .as_deref()
                .filter(|n| simple_path(n) && !n.contains("::"))
            else {
                failures.push(reject(
                    DecisionReason::UnsupportedUnitKind,
                    "parent facade requires a supported named declaration",
                ));
                continue;
            };
            if item.enclosing_impl.is_some()
                || !matches!(
                    item.kind.as_str(),
                    "function_item"
                        | "const_item"
                        | "static_item"
                        | "type_item"
                        | "struct_item"
                        | "enum_item"
                )
            {
                failures.push(reject(
                    DecisionReason::UnsupportedUnitKind,
                    "parent facade does not support this declaration namespace",
                ));
                continue;
            }
            let Some(old) = self.contexts.get(path).filter(|c| c.unresolved.is_empty()) else {
                failures.push(reject(
                    DecisionReason::ConditionalOrInheritedContext,
                    "parent facade requires a complete original module route",
                ));
                continue;
            };
            let Some(new) = self.final_contexts.get(destination).filter(|c| {
                c.unresolved.is_empty()
                    && c.module_segments.len() == old.module_segments.len() + 1
                    && c.module_segments.starts_with(&old.module_segments)
            }) else {
                failures.push(reject(
                    DecisionReason::ConditionalOrInheritedContext,
                    "parent facade requires the evidenced private direct-child route",
                ));
                continue;
            };
            let source = &self.files[path].source;
            let data = &self.parsed[path];
            let node = data
                .tree
                .root_node()
                .named_descendant_for_byte_range(
                    item.span.range.start_byte,
                    item.span.range.end_byte,
                )
                .expect("selected item");
            // A configuration does not turn a conditional facade into an unconditional one.
            let mut conditioned = data.tree.root_node().has_error()
                || (0..data.tree.root_node().named_child_count())
                    .filter_map(|i| data.tree.root_node().named_child(i as u32))
                    .any(|n| {
                        matches!(n.kind(), "macro_invocation" | "macro_definition")
                            || (n.kind() == "inner_attribute_item"
                                && !items::context_independent_attribute(&source[n.byte_range()]))
                    });
            for route_path in &old.filesystem_paths {
                let Some(route_data) = self.parsed.get(route_path) else {
                    conditioned = true;
                    continue;
                };
                let route_source = &self.files[route_path].source;
                conditioned |= (0..route_data.tree.root_node().named_child_count())
                    .filter_map(|i| route_data.tree.root_node().named_child(i as u32))
                    .any(|n| {
                        matches!(n.kind(), "macro_invocation" | "macro_definition")
                            || (n.kind() == "inner_attribute_item"
                                && !items::context_independent_attribute(
                                    &route_source[n.byte_range()],
                                ))
                    });
            }
            for a in &old.declaration_anchors {
                let declaration = self.parsed[&a.path]
                    .items
                    .iter()
                    .find(|i| i.span.range == a.range);
                conditioned |= declaration.is_none_or(|i| {
                    i.attributes.iter().any(|attribute| {
                        let text = &self.files[&a.path].source
                            [attribute.range.start_byte..attribute.range.end_byte];
                        !items::context_independent_attribute(text)
                    })
                });
            }
            // Attributes on retained siblings may produce competing namespace
            // bindings. A plain cfg-gated module has a fixed written name and
            // does not inject bindings into this parent; provider attributes do.
            for sibling in &data.items {
                if sibling.span.range == item.span.range {
                    continue;
                }
                for attribute in &sibling.attributes {
                    let text = &source[attribute.range.start_byte..attribute.range.end_byte];
                    conditioned |= !items::context_independent_attribute(text)
                        && !(sibling.kind == "mod_item" && text.trim_start().starts_with("#[cfg("));
                }
            }
            conditioned |= item.attributes.iter().any(|a| {
                let text = &source[a.range.start_byte..a.range.end_byte];
                !items::context_independent_attribute(text)
            });
            if conditioned {
                failures.push(reject(
                    DecisionReason::ConditionalOrInheritedContext,
                    "parent facade has cfg-sensitive, generated or unexamined module-route context",
                ));
                continue;
            }
            // A type re-export also exports a tuple/unit constructor. Do not silently
            // change a constructor's private-field region by moving it into a child.
            if item.kind == "struct_item"
                && let Some(body) = node.child_by_field_name("body")
            {
                let mut stack = vec![body];
                let mut private = false;
                while let Some(field) = stack.pop() {
                    items::check(self.controls.0, self.controls.1)?;
                    if matches!(
                        field.kind(),
                        "field_declaration" | "ordered_field_declaration"
                    ) {
                        private |= !(0..field.named_child_count())
                            .filter_map(|i| field.named_child(i as u32))
                            .any(|n| {
                                n.kind() == "visibility_modifier"
                                    && &source[n.byte_range()] == "pub"
                            });
                    } else {
                        for i in 0..field.named_child_count() {
                            stack.push(field.named_child(i as u32).expect("field child"));
                        }
                    }
                }
                // Ordered fields are type nodes in some grammar forms, so use the
                // written modifier count as an additional conservative guard.
                if body.kind() == "ordered_field_declaration_list" {
                    let fields = (0..body.named_child_count())
                        .filter_map(|i| body.named_child(i as u32))
                        .filter(|n| {
                            !matches!(
                                n.kind(),
                                "visibility_modifier"
                                    | "attribute_item"
                                    | "line_comment"
                                    | "block_comment"
                            )
                        })
                        .count();
                    let public = (0..body.named_child_count())
                        .filter_map(|i| body.named_child(i as u32))
                        .filter(|n| {
                            n.kind() == "visibility_modifier" && &source[n.byte_range()] == "pub"
                        })
                        .count();
                    private |= fields != public;
                }
                if private {
                    failures.push(reject(DecisionReason::VisibilityScopeUnproved, "parent facade cannot preserve private or restricted field/constructor access"));
                    continue;
                }
            }
            let competing_item = data
                .items
                .iter()
                .filter(|i| i.name.as_deref() == Some(name))
                .count()
                != 1
                || self
                    .selected
                    .iter()
                    .any(|(_, i, d)| d == path && i.name.as_deref() == Some(name));
            let competing_import = self.imports.iter().any(|b| {
                b.path == *path
                    && b.scope_range.is_none()
                    && b.module == old.module_segments
                    && b.leaf.binding == name
            });
            let glob = self
                .imports
                .iter()
                .any(|b| b.path == *path && b.leaf.path.contains('*'))
                || (0..data.tree.root_node().named_child_count())
                    .filter_map(|i| data.tree.root_node().named_child(i as u32))
                    .any(|n| {
                        n.kind() == "use_declaration"
                            && items::use_facts(n, source, self.controls).is_ok_and(|f| f.0)
                    });
            if competing_item || competing_import || glob {
                failures.push(reject(DecisionReason::FinalAliasConflict, "parent facade name conflicts with a written declaration, alias or unproved wildcard binding"));
                continue;
            }
            let modifier = (0..node.named_child_count())
                .filter_map(|i| node.named_child(i as u32))
                .find(|n| n.kind() == "visibility_modifier");
            let region = match item.visibility_key {
                "pub" => None,
                "pub(crate)" => Some(Vec::new()),
                "private" => Some(old.module_segments.clone()),
                "restricted" => {
                    let Some(region) = modifier
                        .and_then(|m| {
                            restricted_region(m, source, &old.module_segments, self.controls)
                        })
                        .filter(|r| old.module_segments.starts_with(r))
                    else {
                        failures.push(reject(
                            DecisionReason::VisibilityScopeUnproved,
                            "parent facade cannot prove the original restricted access region",
                        ));
                        continue;
                    };
                    Some(region)
                }
                _ => {
                    failures.push(reject(
                        DecisionReason::VisibilityScopeUnproved,
                        "parent facade has unsupported visibility",
                    ));
                    continue;
                }
            };
            let facade_visibility = match &region {
                None => "pub ".into(),
                Some(r) if r == &old.module_segments => String::new(),
                Some(r) => format!(
                    "{} ",
                    VisibilityRegion {
                        defining: old.module_segments.clone(),
                        required: r.clone()
                    }
                    .text()
                ),
            };
            let binding = canonical(old, name);
            let a = anchor(self.files, path, &item.span.range);
            let (at, boundary) = crate::move_plan::ergonomics::import_boundary(data, source);
            let target = RewriteTarget::Synthesis {
                path: path.clone(),
                slot: "parent_facade".into(),
                items: self.contributors(std::slice::from_ref(&item.id)),
                boundary_role: None,
                parent_path: Some(path.clone()),
                binding: Some(binding.clone()),
            };
            let text = format!(
                "{facade_visibility}use self::{}::{name};",
                new.module_segments.last().expect("child")
            );
            let mut anchors = vec![a.clone()];
            anchors.extend(old.declaration_anchors.clone());
            if let Some(boundary) = boundary {
                anchors.push(anchor(self.files, path, &boundary.span.range));
            }
            self.add(Repair {
                path: path.clone(), range: span(at, at), after: text.clone(), kind: "parent_facade", written_reexport: true,
                target: target.clone(), item_ids: vec![item.id.clone()], anchors,
                rationale: format!("parent_facade: preserve original named binding {binding} and its original visibility region; private child declaration stays private; no semantic or expansion proof"),
                visibility: None, declaration_for: None, references: Vec::new(), caller_override: false,
                import_module: Some(old.module_segments.clone()), import_scope: None,
            });
            let mut visibility = None;
            if let Some(region) = &region {
                let desired = format!(
                    "{} ",
                    VisibilityRegion {
                        defining: new.module_segments.clone(),
                        required: region.clone()
                    }
                    .text()
                );
                let range = modifier
                    .map(|m| span(m.start_byte(), m.end_byte()))
                    .unwrap_or_else(|| span(node.start_byte(), node.start_byte()));
                let after = if modifier.is_some() {
                    desired.trim_end().to_owned()
                } else {
                    desired
                };
                if source[range.start_byte..range.end_byte] != after {
                    let target = if modifier.is_some() {
                        RewriteTarget::Source {
                            anchor: anchor(self.files, path, &range),
                        }
                    } else {
                        RewriteTarget::Synthesis {
                            path: path.clone(),
                            slot: "visibility_insert".into(),
                            items: self.contributors(std::slice::from_ref(&item.id)),
                            boundary_role: None,
                            parent_path: None,
                            binding: Some(canonical(new, name)),
                        }
                    };
                    self.visibility_observations
                        .entry((path.clone(), range.start_byte, None))
                        .or_default()
                        .preserved_original_access_regions
                        .push(region.clone());
                    self.add_visibility(Repair {
                        path: path.clone(), range, after: String::new(), kind: "visibility", written_reexport: false, target: target.clone(),
                        item_ids: vec![item.id.clone()], anchors: vec![a.clone()],
                        rationale: "parent_facade: express the exact original access region in the child, never a broader region".into(),
                        visibility: Some(VisibilityExplanation {
                            consumers: Vec::new(), original_defining_region: Some(old.module_segments.clone()),
                            final_defining_region: new.module_segments.clone(), preserved_access_region: region.clone(),
                            preserved_original_access_regions: vec![region.clone()], narrowest_covering_region: region.clone(),
                            basis: "preserve exact original region for explicit parent facade".into(),
                        }), declaration_for: None, references: Vec::new(), caller_override: false, import_module: None, import_scope: None,
                    }, &new.module_segments, region);
                    // add_visibility renders the existing region vocabulary (including
                    // the trailing insertion space) used by the ordinary move engine.
                    let repair = self
                        .repairs
                        .iter()
                        .find(|r| r.target == target)
                        .expect("visibility repair");
                    visibility = Some((target, repair.after.clone()));
                }
            }
            self.facades.insert(
                binding,
                Facade {
                    anchor: a,
                    region,
                    target,
                    text,
                    visibility,
                },
            );
        }
        Ok(failures)
    }
    fn facade_items(&self, need: &Need) -> bool {
        !need.item_ids.is_empty()
            && need.item_ids.iter().all(|id| {
                self.selected
                    .iter()
                    .any(|(p, i, _)| &i.id == id && self.facade_for(p, i).is_some())
            })
    }
    pub(super) fn preserve_facade_need(&mut self, need: &mut Need) -> bool {
        if !self.facade_need(need) {
            return false;
        }
        let witness = anchor(self.files, &need.path, &need.range);
        for repair in &mut self.repairs {
            if repair.kind == "parent_facade"
                && repair.item_ids.iter().any(|id| need.item_ids.contains(id))
                && !repair.anchors.contains(&witness)
            {
                self.descriptor_bytes += witness.path.len() + witness.expected_text.len() + 128;
                repair.anchors.push(witness.clone());
            }
        }
        self.exceeded |= self.descriptor_bytes > 128 * 1024 * 1024;
        true
    }
    fn facade_need(&mut self, need: &mut Need) -> bool {
        if !self.facade_items(need) {
            return false;
        }
        let data = &self.parsed[&need.path];
        let source = &self.files[&need.path].source;
        let Some(node) = data
            .tree
            .root_node()
            .named_descendant_for_byte_range(need.range.start_byte, need.range.end_byte)
        else {
            return false;
        };
        if need.reason == DecisionReason::PublicPathChange
            && self
                .selected
                .iter()
                .any(|(p, i, _)| p == &need.path && i.span.range == need.range)
        {
            return true;
        }
        if need.category == "visibility_context"
            && self
                .selected
                .iter()
                .any(|(p, i, _)| p == &need.path && i.span.range == need.range)
        {
            return true;
        }
        if self.unchanged_direct_glob(need, node) {
            return true;
        }
        if node.kind() == "use_declaration"
            && matches!(
                need.category,
                "reexport_dependency" | "unsupported_dependency_form"
            )
        {
            // Do not use declared configuration to bless conditional exported routes.
            if attributed_use(node) {
                return false;
            }
            let Some(module) = self.lexical_module(&need.path, node, false) else {
                return false;
            };
            let Some(using) = self.lexical_module(&need.path, node, true) else {
                return false;
            };
            let Ok(leaves) = items::use_leaves(node, source, self.controls) else {
                return false;
            };
            if leaves.is_empty() {
                return false;
            }
            let scope = node.parent().expect("import scope");
            for leaf in leaves {
                let Some(old) = self.resolve_use(&need.path, &module, node, &leaf.path) else {
                    return false;
                };
                let Some(resolved) = self.written_target(need, &old, &using) else {
                    return false;
                };
                if !self.facades.contains_key(&resolved.terminal) || resolved.route != old {
                    return false;
                }
                if self
                    .imports
                    .iter()
                    .filter(|b| {
                        b.path == need.path
                            && b.module == module
                            && b.leaf.binding == leaf.binding
                            && b.scope_range
                                == (scope.kind() == "block")
                                    .then(|| span(scope.start_byte(), scope.end_byte()))
                    })
                    .count()
                    != 1
                    || (0..scope.named_child_count())
                        .filter_map(|i| scope.named_child(i as u32))
                        .any(|n| {
                            n.child_by_field_name("name")
                                .is_some_and(|n| source[n.byte_range()] == leaf.binding)
                        })
                {
                    return false;
                }
            }
            return true;
        }
        // A retained parent occurrence still uses its own unchanged declaration
        // binding. Moved occurrences continue through ordinary binding repair.
        if need.category == "unsupported_dependency_form"
            && matches!(node.kind(), "identifier" | "type_identifier")
            && !node
                .parent()
                .is_some_and(|p| matches!(p.kind(), "scoped_identifier" | "scoped_type_identifier"))
            && !self.selected.iter().any(|(p, i, _)| {
                p == &need.path
                    && i.span.range.start_byte <= node.start_byte()
                    && node.end_byte() <= i.span.range.end_byte
            })
            && self.lexical_binding(&need.path, node, source, &source[node.byte_range()], false)
                == items::LexicalBinding::Absent
        {
            let Some(module) = self.lexical_module(&need.path, node, false) else {
                return false;
            };
            return self
                .resolve_in(&need.path, &module, &source[node.byte_range()], false)
                .is_some_and(|target| self.facades.contains_key(&target));
        }
        false
    }
    // Only a direct, unique wildcard to the unchanged parent namespace is
    // admitted. Forwarding, competing globs and conditional/unknown scopes keep
    // their existing vetoes; the ordinary route index supplies all path identity.
    fn unchanged_direct_glob(&self, need: &mut Need, node: Node<'_>) -> bool {
        if !matches!(
            need.reason,
            DecisionReason::GlobConsumerUnrepaired
                | DecisionReason::GlobBindingUnproved
                | DecisionReason::PublicPathChange
        ) || !matches!(need.category, "glob_dependency" | "reexport_dependency")
        {
            return false;
        }
        let source = &self.files[&need.path].source;
        let Some(using) = self.lexical_module(&need.path, node, true) else {
            return false;
        };
        let mut globs = Vec::new();
        let mut parent = Some(node);
        while let Some(scope) = parent {
            let module_body = scope.kind() == "declaration_list"
                && scope.parent().is_some_and(|p| p.kind() == "mod_item");
            if scope.kind() == "block" || scope.kind() == "source_file" || module_body {
                if scope.has_error() {
                    return false;
                }
                for i in 0..scope.named_child_count() {
                    if items::check(self.controls.0, self.controls.1).is_err() {
                        return false;
                    }
                    let child = scope.named_child(i as u32).expect("scope child");
                    if matches!(
                        child.kind(),
                        "macro_invocation"
                            | "macro_definition"
                            | "let_declaration"
                            | "inner_attribute_item"
                    ) || (child.kind() == "attribute_item"
                        && !items::context_independent_attribute(&source[child.byte_range()]))
                        || (child.kind() == "expression_statement"
                            && child
                                .named_child(0)
                                .is_some_and(|n| n.kind() == "macro_invocation"))
                    {
                        return false;
                    }
                    if child.kind() == "use_declaration" {
                        if items::use_facts(child, source, self.controls).is_ok_and(|f| f.0) {
                            globs.push(child);
                        } else if items::use_leaves(child, source, self.controls).is_ok_and(
                            |leaves| {
                                leaves.iter().any(|l| {
                                    self.selected.iter().any(|(_, i, _)| {
                                        need.item_ids.contains(&i.id)
                                            && i.name.as_deref() == Some(&l.binding)
                                    })
                                })
                            },
                        ) {
                            return false;
                        }
                    } else if child.child_by_field_name("name").is_some_and(|name| {
                        self.selected.iter().any(|(_, item, _)| {
                            need.item_ids.contains(&item.id)
                                && item.name.as_deref() == Some(&source[name.byte_range()])
                        })
                    }) {
                        return false;
                    }
                }
                if scope.kind() == "source_file" || module_body {
                    break;
                }
            }
            parent = scope.parent();
        }
        if globs.len() != 1 {
            return false;
        }
        let glob = globs[0];
        if attributed_use(glob) {
            return false;
        }
        let Some(wildcard) = (0..glob.named_child_count())
            .filter_map(|i| glob.named_child(i as u32))
            .find(|n| n.kind() == "use_wildcard")
        else {
            return false;
        };
        let Some(route) = source[wildcard.byte_range()].trim().strip_suffix("::*") else {
            return false;
        };
        if !simple_path(route) {
            return false;
        }
        let Some(module) = self.lexical_module(&need.path, glob, false) else {
            return false;
        };
        let Some(target) = self.resolve_use(&need.path, &module, glob, route) else {
            return false;
        };
        if need.lexical_uncertainty.as_ref().is_some_and(|w| {
            w.reason != items::LexicalReason::RelevantLocalImport || w.pattern.is_some()
        }) {
            return false;
        }
        for id in need.item_ids.clone() {
            let Some((path, item, _)) = self.selected.iter().find(|(_, i, _)| i.id == id) else {
                return false;
            };
            let Some(context) = self.contexts.get(path) else {
                return false;
            };
            if canonical(context, "").trim_end_matches("::") != target
                || !self
                    .pattern_routes
                    .safe_scope(
                        self.parsed[path].tree.root_node(),
                        &self.files[path].source,
                        true,
                    )
                    .unwrap_or(false)
            {
                return false;
            }
            let Some(facade) = self.facade_for(path, item) else {
                return false;
            };
            if !facade.region.as_ref().is_none_or(|r| using.starts_with(r))
                || !self.accessible_route(need, path, item, &using)
            {
                return false;
            }
        }
        true
    }
    pub(super) fn audit_facades(&self) -> Result<Vec<Need>, DomainError> {
        let mut failures = Vec::new();
        for facade in self.facades.values() {
            items::check(self.controls.0, self.controls.1)?;
            if let Some(facade_repair) = self.repairs.iter().find(|r| r.target == facade.target) {
                for repair in &self.repairs {
                    if repair.kind == "import_insert" && repair.path == facade_repair.path
                        && repair.import_scope.is_none()
                        && parsed_import(&repair.after, self.controls).is_ok_and(|(_, binding)| matches!(&facade.target, RewriteTarget::Synthesis { binding: Some(old), .. } if old.rsplit("::").next() == Some(binding.as_str()))) {
                        failures.push(self.repair_need(repair, DecisionReason::FinalAliasConflict, "binding_collision", "selected import alias conflicts with the preserved parent facade name"));
                    }
                }
            }
            for (target, text) in std::iter::once((&facade.target, &facade.text))
                .chain(facade.visibility.iter().map(|(t, s)| (t, s)))
            {
                if let Some(repair) = self.repairs.iter().find(|r| &r.target == target)
                    && &repair.after != text
                {
                    failures.push(self.repair_need(repair, DecisionReason::VisibilityScopeUnproved, "reexport_dependency", "parent facade and its exact access region must remain unchanged; retain/replace choices cannot waive route preservation"));
                }
            }
        }
        Ok(failures)
    }
}
