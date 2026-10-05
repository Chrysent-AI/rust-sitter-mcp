const TRANSCRIPT_MAX_RAW_LINE: usize = 16 * 1024 * 1024;

fn sanitize_subagent_transcript_line(line: &str) -> usize {
    if line.len() > TRANSCRIPT_MAX_RAW_LINE {
        return line.len();
    }
    0
}

async fn spawn_and_monitor() {
    let (assistant_done_tx, mut assistant_done_rx) =
        mpsc::unbounded_channel::<AssistantDoneSignal>();
    let settled_observed = Arc::new(AtomicBool::new(false));
    let stdout_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut stdout_buf = [0_u8; 8192];
        let mut stdout_carry: Vec<u8> = Vec::new();
        let mut assistant_done = false;
        let settled_observed = settled_observed;
        loop {
            let n = match reader.read(&mut stdout_buf).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => { break; }
            };
            stdout_carry.extend_from_slice(&stdout_buf[..n]);
            while let Some(newline_idx) = stdout_carry.iter().position(|b| *b == b'\n') {
                let mut line_bytes: Vec<u8> = stdout_carry.drain(..=newline_idx).collect();
                if line_bytes.ends_with(b"\n") { line_bytes.pop(); }
                let Ok(line) = std::str::from_utf8(&line_bytes) else { continue; };
                append_sanitized_transcript_line(&mut transcript, line).await;
                if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
                    if let Some(usage) = event.get("message").and_then(|m| m.get("usage")) {
                        let _ = usage;
                    }
                }
            }
            if stdout_carry.len() > TRANSCRIPT_MAX_RAW_LINE {
                let invalid = transcript_invalid_line(stdout_carry.len(), "raw_line_too_long");
                let _ = transcript.write_all(invalid.as_bytes()).await;
                let _ = transcript.write_all(b"\n").await;
                stdout_carry.clear();
            }
        }
    });
}
