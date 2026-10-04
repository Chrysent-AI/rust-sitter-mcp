use super::fixture_gen::{Fixture, tree};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

pub fn anchor(repo: &Fixture, path: &str, text: &str) -> Value {
    let source = fs::read_to_string(repo.0.join(path)).unwrap();
    let start = source.find(text).unwrap();
    json!({"path":path,"range":{"start_byte":start,"end_byte":start+text.len()},"expected_text":text})
}

pub fn apply(repo: &Fixture, envelope: &Value) -> Fixture {
    let plan = &envelope["plan"];
    assert_eq!(envelope["status"], "complete", "{envelope}");
    assert_eq!(plan["applicable"], true, "{envelope}");
    assert_eq!(plan["integrity"]["syntax"], "checked");
    assert_eq!(plan["integrity"]["semantic"], "not_performed");
    let patch = plan["patch"].as_str().unwrap();
    assert!(!patch.is_empty());
    for forbidden in ["deleted file mode", "old mode", "new mode", "rename from"] {
        assert!(!patch.contains(forbidden));
    }
    let copy = repo.copy();
    let edits = plan["edits"].as_array().unwrap();
    let mut expected = tree(&repo.0);
    for base in plan["base_files"].as_array().unwrap() {
        let path = base["path"].as_str().unwrap();
        let entry = expected.get_mut(Path::new(path)).unwrap();
        let original = String::from_utf8(entry.bytes.clone()).unwrap();
        assert_eq!(base["original_length"], original.len());
        assert_eq!(
            base["mode"],
            if entry.mode & 0o111 != 0 {
                "100755"
            } else {
                "100644"
            }
        );
        let relevant: Vec<_> = edits.iter().filter(|e| e["path"] == path).collect();
        let mut last = 0;
        for edit in &relevant {
            let start = edit["range"]["start_byte"].as_u64().unwrap() as usize;
            let end = edit["range"]["end_byte"].as_u64().unwrap() as usize;
            assert!(start >= last && end >= start);
            assert_eq!(
                &original[start..end],
                edit["original_text"].as_str().unwrap()
            );
            last = end;
        }
        let mut output = original;
        for edit in relevant.into_iter().rev() {
            output.replace_range(
                edit["range"]["start_byte"].as_u64().unwrap() as usize
                    ..edit["range"]["end_byte"].as_u64().unwrap() as usize,
                edit["replacement_text"].as_str().unwrap(),
            );
        }
        entry.bytes = output.into_bytes();
    }
    let mut new_paths = Vec::new();
    for file in plan["created_files"].as_array().unwrap() {
        let path = file["path"].as_str().unwrap();
        assert!(!repo.0.join(path).exists());
        assert_eq!(file["must_be_absent"], true);
        assert_eq!(file["mode"], "100644");
        assert!(!edits.iter().any(|e| e["path"] == path));
        new_paths.push(path.to_owned());
        // Build the independent JSON tree in a disposable copy, not the caller tree.
        fs::write(copy.0.join(path), file["content"].as_str().unwrap()).unwrap();
        expected.insert(
            Path::new(path).to_owned(),
            tree(&copy.0).remove(Path::new(path)).unwrap(),
        );
        fs::remove_file(copy.0.join(path)).unwrap();
        let link = &file["declaration_link"];
        if link["kind"] == "synthesized" {
            assert!(
                plan["rewrites"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["id"] == link["rewrite_id"] && r["kind"] == "module_declaration")
            );
        } else {
            assert_eq!(link["kind"], "reused");
            let source = fs::read_to_string(repo.0.join(link["path"].as_str().unwrap())).unwrap();
            let span = &link["span"]["range"];
            assert!(
                source[span["start_byte"].as_u64().unwrap() as usize
                    ..span["end_byte"].as_u64().unwrap() as usize]
                    .contains("mod ")
            );
        }
    }
    for args in [
        ["apply", "--check", "-"].as_slice(),
        ["apply", "-"].as_slice(),
    ] {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&copy.0)
            .args(args)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(patch.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{patch}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(tree(&copy.0), expected);
    for path in new_paths {
        let output = Command::new("git")
            .arg("-C")
            .arg(&copy.0)
            .args(["diff", "--no-index", "--", "/dev/null", &path])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&output.stdout).contains("new file mode 100644"));
    }
    for origin in plan["origins"].as_array().unwrap() {
        let source =
            fs::read_to_string(repo.0.join(origin["source_path"].as_str().unwrap())).unwrap();
        let output =
            fs::read_to_string(copy.0.join(origin["output_path"].as_str().unwrap())).unwrap();
        let a = &origin["source_range"];
        let b = &origin["output_range"];
        assert_eq!(
            &source[a["start_byte"].as_u64().unwrap() as usize
                ..a["end_byte"].as_u64().unwrap() as usize],
            &output[b["start_byte"].as_u64().unwrap() as usize
                ..b["end_byte"].as_u64().unwrap() as usize]
        );
    }
    for rewrite in plan["rewrites"].as_array().unwrap() {
        assert_eq!(rewrite["confidence"]["basis"], "syntactic_heuristic");
        assert_eq!(rewrite["artifact_links"].as_array().unwrap().len(), 1);
        let link = &rewrite["artifact_links"][0];
        let (text, span) = if link["kind"] == "edit" {
            (
                &edits[link["index"].as_u64().unwrap() as usize]["replacement_text"],
                &link["replacement_range"],
            )
        } else {
            let file = plan["created_files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["id"] == link["id"])
                .unwrap();
            (&file["content"], &link["content_range"])
        };
        assert_eq!(
            &text.as_str().unwrap()[span["start_byte"].as_u64().unwrap() as usize
                ..span["end_byte"].as_u64().unwrap() as usize],
            rewrite["after_text"].as_str().unwrap()
        );
    }
    copy
}
