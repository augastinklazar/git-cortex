use crate::protocol::*;
use crate::tools::list_tools;
use cortex_capture::reconcile_candidates;
use cortex_core::schema::{ActiveHypothesis, CallGraphEdge, CaptureMode, InvariantSeverity};
use cortex_engine::checkpoint::checkpoint_state;
use cortex_engine::handoff::{create_handoff_envelope, import_handoff_envelope};
use cortex_engine::inspect::inspect_symbol;
use cortex_engine::invariant::{record_invariant, verify_invariants, DiffScope};
use cortex_engine::tombstone::{record_tombstone, RecordTombstoneParams};
use cortex_storage::{CortexDb, CortexRepo};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

pub struct McpServer {
    repo: CortexRepo,
    db: CortexDb,
    client_harness: String,
}

impl McpServer {
    pub fn new(repo: CortexRepo, db: CortexDb) -> Self {
        Self {
            repo,
            db,
            client_harness: "generic-mcp-client/1.0".to_string(),
        }
    }

    pub fn run_stdio<R: BufRead, W: Write>(&mut self, mut reader: R, mut writer: W) {
        loop {
            let (payload, is_header_framed) = match read_next_rpc_message(&mut reader) {
                Ok(Some(msg)) => msg,
                Ok(None) => break,
                Err(e) => {
                    eprintln!("cortex-mcp: stdin error: {}", e);
                    break;
                }
            };

            let req: JsonRpcRequest = match serde_json::from_slice(&payload) {
                Ok(r) => r,
                Err(e) => {
                    let err_resp =
                        JsonRpcResponse::error(None, -32700, format!("Parse error: {}", e));
                    if let Ok(err_str) = serde_json::to_string(&err_resp) {
                        if is_header_framed {
                            let _ = write!(
                                writer,
                                "Content-Length: {}\r\n\r\n{}",
                                err_str.len(),
                                err_str
                            );
                        } else {
                            let _ = writeln!(writer, "{}", err_str);
                        }
                        let _ = writer.flush();
                    }
                    continue;
                }
            };

            let resp = self.handle_request(&req);
            if let Some(r) = resp {
                if let Ok(json_str) = serde_json::to_string(&r) {
                    if is_header_framed {
                        let _ = write!(
                            writer,
                            "Content-Length: {}\r\n\r\n{}",
                            json_str.len(),
                            json_str
                        );
                    } else {
                        let _ = writeln!(writer, "{}", json_str);
                    }
                    let _ = writer.flush();
                }
            }
        }
    }

    pub fn handle_request(&mut self, req: &JsonRpcRequest) -> Option<JsonRpcResponse> {
        // Per JSON-RPC 2.0 (section 4.1), notifications MUST NOT produce any response
        req.id.as_ref()?;

        match req.method.as_str() {
            "initialize" => {
                if let Some(params) = &req.params {
                    if let Some(client_info) = params.get("clientInfo") {
                        let name = client_info
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("agent");
                        let version = client_info
                            .get("version")
                            .and_then(|v| v.as_str())
                            .unwrap_or("1.0");
                        self.client_harness = format!("{}/{}", name, version);
                    }
                }

                Some(JsonRpcResponse::success(
                    req.id.clone(),
                    json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": {
                            "tools": {}
                        },
                        "serverInfo": {
                            "name": "git-cortex",
                            "version": "0.1.0"
                        }
                    }),
                ))
            }
            "notifications/initialized" => None,
            "ping" => Some(JsonRpcResponse::success(req.id.clone(), json!({}))),
            "tools/list" => Some(JsonRpcResponse::success(req.id.clone(), list_tools())),
            "tools/call" => {
                // Passive capture reconcile pass on every tool call!
                let _ = reconcile_candidates(&self.repo, &mut self.db);

                let params = req.params.as_ref().cloned().unwrap_or(Value::Null);
                let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));

                let tool_result = self.execute_tool(tool_name, &args);
                Some(JsonRpcResponse::success(
                    req.id.clone(),
                    serde_json::to_value(tool_result).unwrap_or(Value::Null),
                ))
            }
            _ => Some(JsonRpcResponse::error(
                req.id.clone(),
                -32601,
                format!("Method not found: {}", req.method),
            )),
        }
    }

    fn execute_tool(&mut self, name: &str, args: &Value) -> ToolResult {
        match name {
            "cortex_inspect_symbol" => {
                let filepath = match args.get("filepath").and_then(|v| v.as_str()) {
                    Some(f) => f,
                    None => {
                        return ToolResult::error(
                            "Missing required parameter 'filepath'".to_string(),
                        )
                    }
                };
                let symbol_name = args.get("symbol_name").and_then(|v| v.as_str());

                match inspect_symbol(&self.repo, &self.db, filepath, symbol_name) {
                    Ok(res) => ToolResult::ok(res.formatted_markdown),
                    Err(e) => ToolResult::error(format!("Inspection failed: {}", e)),
                }
            }
            "cortex_record_tombstone" => {
                let target_symbols: Vec<String> = match args.get("target_symbols") {
                    Some(Value::Array(arr)) => arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect(),
                    _ => {
                        return ToolResult::error("Missing or invalid 'target_symbols'".to_string())
                    }
                };

                let approach = match args.get("attempted_approach").and_then(|v| v.as_str()) {
                    Some(a) => a,
                    None => {
                        return ToolResult::error(
                            "Missing required 'attempted_approach'".to_string(),
                        )
                    }
                };

                let reason = match args.get("failure_reason").and_then(|v| v.as_str()) {
                    Some(r) => r,
                    None => {
                        return ToolResult::error("Missing required 'failure_reason'".to_string())
                    }
                };

                let test_cmd = args
                    .get("test_command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let error_sig = args
                    .get("error_signature")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                match record_tombstone(
                    &self.repo,
                    &mut self.db,
                    RecordTombstoneParams {
                        target_symbols: &target_symbols,
                        attempted_approach: approach,
                        failure_reason: reason,
                        test_command: test_cmd,
                        error_signature: error_sig,
                        author_harness: &self.client_harness,
                        capture_mode: CaptureMode::Manual,
                    },
                ) {
                    Ok(rec) => ToolResult::ok(format!(
                        "Recorded Tombstone '{}' bound to {} symbol(s).",
                        rec.id,
                        rec.target_symbols.len()
                    )),
                    Err(e) => ToolResult::error(format!("Failed to record tombstone: {}", e)),
                }
            }
            "cortex_checkpoint_state" => {
                let objective = match args.get("objective").and_then(|v| v.as_str()) {
                    Some(o) => o,
                    None => return ToolResult::error("Missing required 'objective'".to_string()),
                };

                let edges: Vec<CallGraphEdge> = match args.get("verified_edges") {
                    Some(val) if !val.is_null() => match serde_json::from_value(val.clone()) {
                        Ok(edges) => edges,
                        Err(e) => {
                            return ToolResult::error(format!("Invalid 'verified_edges': {}", e))
                        }
                    },
                    _ => Vec::new(),
                };

                let hyps: Vec<ActiveHypothesis> = match args.get("active_hypotheses") {
                    Some(val) if !val.is_null() => match serde_json::from_value(val.clone()) {
                        Ok(hyps) => hyps,
                        Err(e) => {
                            return ToolResult::error(format!("Invalid 'active_hypotheses': {}", e))
                        }
                    },
                    _ => Vec::new(),
                };

                match checkpoint_state(&self.repo, &mut self.db, objective, &edges, &hyps) {
                    Ok(res) => ToolResult::ok(format!(
                        "Checkpoint '{}' saved on branch '{}' (snapshot: {}). Epistemic graph: {} edges, {} hypotheses.",
                        res.checkpoint_id, res.branch, res.snapshot_ref, res.edge_count, res.hypothesis_count
                    )),
                    Err(e) => ToolResult::error(format!("Checkpoint failed: {}", e)),
                }
            }
            "cortex_handoff_pack" => {
                let mode = match args.get("mode").and_then(|v| v.as_str()) {
                    Some(m) => m,
                    None => {
                        return ToolResult::error(
                            "Missing required 'mode' (IMPORT or EXPORT)".to_string(),
                        )
                    }
                };

                if mode.eq_ignore_ascii_case("EXPORT") {
                    let branch = args.get("branch").and_then(|v| v.as_str());
                    match create_handoff_envelope(&self.repo, &self.db, branch, None) {
                        Ok(envelope) => {
                            let json_str =
                                serde_json::to_string_pretty(&envelope).unwrap_or_default();
                            ToolResult::ok(json_str)
                        }
                        Err(e) => ToolResult::error(format!("Export failed: {}", e)),
                    }
                } else if mode.eq_ignore_ascii_case("IMPORT") {
                    let envelope_val = match args.get("envelope") {
                        Some(val) => val,
                        None => {
                            return ToolResult::error(
                                "Missing 'envelope' payload for IMPORT mode".to_string(),
                            )
                        }
                    };

                    match serde_json::from_value(envelope_val.clone()) {
                        Ok(envelope) => {
                            let branch = self
                                .repo
                                .current_branch()
                                .unwrap_or_else(|_| "main".to_string());
                            match import_handoff_envelope(
                                &self.repo,
                                &mut self.db,
                                &envelope,
                                &branch,
                            ) {
                                Ok(_) => ToolResult::ok(
                                    "Epistemic handoff imported successfully.".to_string(),
                                ),
                                Err(e) => ToolResult::error(format!("Import failed: {}", e)),
                            }
                        }
                        Err(e) => {
                            ToolResult::error(format!("Invalid handoff envelope JSON: {}", e))
                        }
                    }
                } else {
                    ToolResult::error("Invalid mode: expected 'IMPORT' or 'EXPORT'".to_string())
                }
            }
            "cortex_verify_invariants" => {
                let diff_scope = match args.get("diff_scope").and_then(|v| v.as_str()) {
                    Some("STAGED") => DiffScope::Staged,
                    Some("BRANCH") => DiffScope::Branch,
                    _ => DiffScope::WorkingTree,
                };

                match verify_invariants(&self.repo, &self.db, diff_scope) {
                    Ok(rep) => {
                        if rep.passed {
                            ToolResult::ok(format!(
                                "All {} invariant contracts verified successfully against {:?}.",
                                rep.checked_count, rep.scope
                            ))
                        } else {
                            let mut msg = format!(
                                "INVARIANT VIOLATION: {} blocking failure(s) found in {:?}:\n",
                                rep.violations.len(),
                                rep.scope
                            );
                            for v in &rep.violations {
                                msg.push_str(&format!(
                                    "- Invariant '{}': guarded symbol '{}' broken by upstream '{}' drift!\n  Assertion: {}\n  Run: {}\n",
                                    v.invariant_id, v.guarded_symbol, v.upstream_symbol, v.assertion, v.verification_test_cmd
                                ));
                            }
                            ToolResult::error(msg)
                        }
                    }
                    Err(e) => ToolResult::error(format!("Invariant verification failed: {}", e)),
                }
            }
            "cortex_record_invariant" => {
                let guarded = match args.get("guarded_symbol").and_then(|v| v.as_str()) {
                    Some(g) => g,
                    None => return ToolResult::error("Missing 'guarded_symbol'".to_string()),
                };
                let assertion = match args.get("assertion").and_then(|v| v.as_str()) {
                    Some(a) => a,
                    None => return ToolResult::error("Missing 'assertion'".to_string()),
                };
                let test_cmd = match args.get("verification_test_cmd").and_then(|v| v.as_str()) {
                    Some(t) => t,
                    None => {
                        return ToolResult::error("Missing 'verification_test_cmd'".to_string())
                    }
                };
                let upstream: Vec<String> = args
                    .get("upstream_dependencies")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();

                let severity = match args.get("severity").and_then(|v| v.as_str()) {
                    Some("WARNING") => InvariantSeverity::Warning,
                    _ => InvariantSeverity::Blocking,
                };

                match record_invariant(
                    &self.repo,
                    &mut self.db,
                    guarded,
                    assertion,
                    &upstream,
                    test_cmd,
                    severity,
                ) {
                    Ok(contract) => ToolResult::ok(format!(
                        "Created InvariantContract '{}' guarding '{}' against {} upstream dependencies.",
                        contract.invariant_id, contract.guarded_symbol.symbol_uri, contract.upstream_dependencies.len()
                    )),
                    Err(e) => ToolResult::error(format!("Failed to record invariant: {}", e)),
                }
            }
            _ => ToolResult::error(format!("Unknown tool: {}", name)),
        }
    }
}

pub fn read_next_rpc_message<R: BufRead>(
    reader: &mut R,
) -> std::io::Result<Option<(Vec<u8>, bool)>> {
    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(len_str) = trimmed
            .strip_prefix("Content-Length:")
            .or_else(|| trimmed.strip_prefix("content-length:"))
        {
            let byte_len: usize = len_str
                .trim()
                .parse()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            loop {
                line.clear();
                reader.read_line(&mut line)?;
                if line.trim().is_empty() {
                    break;
                }
            }
            let mut buf = vec![0u8; byte_len];
            reader.read_exact(&mut buf)?;
            return Ok(Some((buf, true)));
        }
        return Ok(Some((trimmed.as_bytes().to_vec(), false)));
    }
}
