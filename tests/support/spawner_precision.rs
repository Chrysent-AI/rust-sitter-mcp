use super::fixture_gen::Fixture;

pub fn load(source: &str) -> Fixture {
    let repo = Fixture::generate();
    repo.write(
        "cases/precision/lib.rs",
        "mod subagent;\nmod utils;\nmod pi;\n",
    );
    repo.write("cases/precision/main.rs", "fn main() {}\n");
    repo.write("cases/precision/subagent/mod.rs", "mod spawner;\n");
    repo.write("cases/precision/subagent/spawner.rs", source);
    repo.write("cases/precision/utils/mod.rs", "pub(crate) mod display;\n");
    repo.write("cases/precision/utils/display.rs", "#[must_use]\npub(crate) fn sanitize_stop_reason(raw: &str, max: usize) -> String { raw.chars().take(max).collect() }\n#[must_use]\npub(crate) fn sanitize_error_diagnostic(raw: &str, max: usize) -> String { raw.chars().take(max).collect() }\n");
    repo.write("cases/precision/pi/mod.rs", "pub(crate) mod protocol;\n");
    repo.write(
        "cases/precision/pi/protocol.rs",
        "pub(crate) const INVALID_TERMINAL_METADATA: &str = \"<invalid>\";\n",
    );
    repo
}
