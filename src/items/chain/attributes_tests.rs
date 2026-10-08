use super::attributes::{DeclaredCfg, admits};

#[test]
fn inert_chain_metadata_needs_no_configuration_or_file_reads() {
    for text in [
        "#![doc = include_str!(\"missing.md\")]",
        "#![doc = \"plain documentation\"]",
        "#[doc(hidden)]",
        "#![ forbid (unsafe_code) ]",
        "#[allow(dead_code)]",
        "#[allow(cfg, path)]",
        "#![warn(missing_docs)]",
        "#[deny(warnings)]",
        "#[cfg_attr(unknown, doc(cfg(feature = \"enabled\")))]",
        "#![cfg_attr(docsrs, feature(doc_cfg))]",
        "#![cfg_attr(unknown, cfg_attr(other, allow(dead_code)))]",
    ] {
        assert!(admits(text, None), "{text}");
    }
    for text in [
        "#![no_implicit_prelude]",
        "#![no_std]",
        "#![no_core]",
        "#![feature(arbitrary_gate)]",
        "#![recursion_limit = \"256\"]",
        "#![macro_use]",
        "#![provider(doc = \"not builtin\")]",
        "#![custom::doc(include_str!(\"missing.md\"))]",
        "#[path = \"other.rs\"]",
        "#[custom::doc]",
        "#[unexamined(cfg, path)]",
        "#[derive(Clone)]",
        "#[cfg_attr(unknown, path = \"other.rs\")]",
        "#![cfg_attr(unknown, feature(other_feature))]",
        "#[cfg_attr(not(a, b), doc(hidden))]",
        "#[cfg_attr(unknown,)]",
        "#[cfg_attr(unknown, doc(hidden)]",
    ] {
        assert!(!admits(text, None), "{text}");
    }
}

#[test]
fn chain_cfg_uses_positive_atoms_and_requires_active_declarations() {
    let cfg = DeclaredCfg::from([
        ("feature".into(), Some("enabled".into())),
        ("unix".into(), None),
    ]);
    for text in [
        "#[cfg(feature = \"enabled\")]",
        "#[cfg(all(unix, feature = \"enabled\"))]",
        "#[cfg_attr(unix, cfg(feature = \"enabled\"))]",
        "#[cfg_attr(not(unix), path = \"other.rs\")]",
        "#[cfg(all())]",
    ] {
        assert!(admits(text, Some(&cfg)), "{text}");
        assert!(!admits(text, None), "no opt-in: {text}");
    }
    for text in [
        "#[cfg(any())]",
        "#[cfg(not(unix))]",
        "#[cfg(missing)]",
        "#[cfg(any(unix, missing))]",
        "#[cfg_attr(unix, path = \"other.rs\")]",
        "#[cfg_attr(unix, cfg(any()))]",
        "#[cfg_attr(missing, path = \"other.rs\")]",
        "#[cfg(not(unix, unix))]",
        "#[cfg()]",
    ] {
        assert!(!admits(text, Some(&cfg)), "{text}");
    }
}
