//! Diagnostic provenance only; these records never participate in admission.
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn occurrence_fallback_classes_preserve_the_reason_and_attribute_anchor() {
        let flag = std::sync::atomic::AtomicBool::new(false);
        let tree = crate::trivia::parse(
            "fn f() {}",
            std::time::Instant::now() + std::time::Duration::from_secs(5),
            &flag,
        )
        .unwrap()
        .unwrap();
        for (reason, class) in [
            (DecisionReason::GlobBindingUnproved, "glob_import"),
            (
                DecisionReason::MacroContextUnexamined,
                "chain_macro_statement",
            ),
            (
                DecisionReason::ConditionalOrInheritedContext,
                "conditional_context",
            ),
            (DecisionReason::ModuleChainFailure, "unresolved_chain"),
            (
                DecisionReason::ExternalOrMissingBinding,
                "written_binding_unproved",
            ),
        ] {
            let mut need = crate::items::need(reason, "test", "probe.rs", tree.root_node(), "test");
            need.attribute_range = Some(ByteRange {
                start_byte: 0,
                end_byte: 2,
            });
            need.disclose_refusal();
            assert_eq!(need.reason, reason);
            assert_eq!(need.refusal_basis[0].class, class);
            assert_eq!(need.refusal_basis[0].anchor.range, need.attribute_range);
        }
    }
}
use super::{DecisionReason, Need};
use crate::result::ByteRange;
use rmcp::schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RefusalAnchor {
    pub path: String,
    /// Original coordinates; absent for an unresolved file or synthesized binding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<ByteRange>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RefusalBasis {
    pub class: String,
    pub anchor: RefusalAnchor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}
impl RefusalBasis {
    pub(crate) fn new(class: &str, path: &str, range: Option<ByteRange>) -> Self {
        Self {
            class: class.into(),
            anchor: RefusalAnchor {
                path: path.into(),
                range,
            },
            name: None,
        }
    }
    pub(crate) fn named(mut self, name: &str) -> Self {
        self.name = Some(name.trim_start_matches("r#").into());
        self
    }
}
impl Need {
    pub(crate) fn refuse_at_occurrence(&mut self, class: &str) {
        self.refusal_basis.push(RefusalBasis::new(
            class,
            &self.path,
            Some(self.range.clone()),
        ));
    }
    pub(crate) fn disclose_refusal(&mut self) {
        if let Some(witness) = &self.lexical_uncertainty {
            let location = witness.pattern.as_ref().unwrap_or(&witness.scope);
            let class = if witness.witness_relation.is_some() {
                "chain_macro_statement"
            } else {
                "lexical_uncertainty"
            };
            let basis = RefusalBasis::new(class, &location.path, Some(location.range.clone()))
                .named(&witness.spelling);
            if !self.refusal_basis.contains(&basis) {
                self.refusal_basis.push(basis);
            }
        }
        // More precise audit provenance wins over this occurrence-level fallback.
        if self.refusal_basis.is_empty() {
            let class = match self.reason {
                DecisionReason::GlobBindingUnproved => "glob_import",
                DecisionReason::MacroContextUnexamined => "chain_macro_statement",
                DecisionReason::ConditionalOrInheritedContext => "conditional_context",
                DecisionReason::ModuleChainFailure => "unresolved_chain",
                _ => "written_binding_unproved",
            };
            self.refusal_basis.push(RefusalBasis::new(
                class,
                &self.path,
                Some(
                    self.attribute_range
                        .clone()
                        .unwrap_or_else(|| self.range.clone()),
                ),
            ));
        }
    }
}
