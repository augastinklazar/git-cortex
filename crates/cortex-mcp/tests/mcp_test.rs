use cortex_mcp::McpServer;
use cortex_storage::{CortexDb, CortexRepo};
use git2::Repository;
use serde_json::json;
use std::fs;
use std::io::Cursor;
use tempfile::TempDir;

#[test]
fn test_mcp_server_full_stdio_lifecycle() {
    let temp = TempDir::new().unwrap();
    let repo = Repository::init(temp.path()).unwrap();
    let sig = git2::Signature::now("Test", "test@example.com").unwrap();

    let src_dir = temp.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let file = src_dir.join("service.rs");
    fs::write(
        &file,
        "pub struct Service;\nimpl Service {\n  pub fn run(&self) -> bool { true }\n}\n",
    )
    .unwrap();

    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Initial", &tree, &[])
        .unwrap();

    let cortex_repo = CortexRepo::discover(Some(temp.path())).unwrap();
    let db = CortexDb::open_in_memory().unwrap();

    let mut server = McpServer::new(cortex_repo, db);

    // Prepare JSON-RPC requests
    let requests = vec![
        // 1. Initialize
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "claude-code",
                    "version": "1.4.2"
                }
            }
        }),
        // 2. Tools list
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
        // 3. Inspect symbol
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "cortex_inspect_symbol",
                "arguments": {
                    "filepath": "src/service.rs",
                    "symbol_name": "Service.run"
                }
            }
        }),
        // 4. Record tombstone
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "cortex_record_tombstone",
                "arguments": {
                    "target_symbols": ["src/service.rs#Service::run"],
                    "attempted_approach": "Tried multithreading without Arc",
                    "failure_reason": "Lifetime borrowed data escapes function",
                    "test_command": "cargo test",
                    "error_signature": "E0373: closure may outlive the current function"
                }
            }
        }),
        // 5. Inspect again - tombstone should now appear!
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "cortex_inspect_symbol",
                "arguments": {
                    "filepath": "src/service.rs"
                }
            }
        }),
        // 6. Checkpoint
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "cortex_checkpoint_state",
                "arguments": {
                    "objective": "Verify concurrency refactor",
                    "verified_edges": [
                        {
                            "caller": "src/main.rs#main",
                            "callee": "src/service.rs#Service::run",
                            "relationship": "INVOKES",
                            "verified_ast_hash": "ast:sha256:1111222233334444"
                        }
                    ],
                    "active_hypotheses": []
                }
            }
        }),
        // 7. Handoff export
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "cortex_handoff_pack",
                "arguments": {
                    "mode": "EXPORT"
                }
            }
        }),
        // 8. Record invariant
        json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": {
                "name": "cortex_record_invariant",
                "arguments": {
                    "guarded_symbol": "src/service.rs#Service::run",
                    "assertion": "Must return boolean without panicking",
                    "upstream_dependencies": ["src/service.rs#Service"],
                    "verification_test_cmd": "cargo test"
                }
            }
        }),
        // 9. Verify invariants
        json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {
                "name": "cortex_verify_invariants",
                "arguments": {
                    "diff_scope": "WORKING_TREE"
                }
            }
        }),
    ];

    let mut input_bytes = Vec::new();
    for r in requests {
        input_bytes.extend_from_slice(serde_json::to_string(&r).unwrap().as_bytes());
        input_bytes.push(b'\n');
    }

    let input_cursor = Cursor::new(input_bytes);
    let mut output_bytes = Vec::new();

    server.run_stdio(input_cursor, &mut output_bytes);

    let output_str = String::from_utf8(output_bytes).unwrap();
    let lines: Vec<&str> = output_str
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();

    assert_eq!(lines.len(), 9);

    // Verify initialize response
    let init_resp: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(init_resp["id"], 1);
    assert_eq!(init_resp["result"]["serverInfo"]["name"], "git-cortex");

    // Verify tools list
    let list_resp: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(list_resp["id"], 2);
    let tools = list_resp["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 6);

    // Verify tombstone record response
    let tmb_resp: serde_json::Value = serde_json::from_str(lines[3]).unwrap();
    assert_eq!(tmb_resp["id"], 4);
    assert_eq!(tmb_resp["result"]["isError"], false);
    assert!(tmb_resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Recorded Tombstone"));

    // Verify inspect after tombstone
    let inspect_after: serde_json::Value = serde_json::from_str(lines[4]).unwrap();
    assert_eq!(inspect_after["id"], 5);
    let text = inspect_after["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("Tried multithreading without Arc"));

    // Verify handoff export
    let handoff_resp: serde_json::Value = serde_json::from_str(lines[6]).unwrap();
    let handoff_text = handoff_resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(handoff_text.contains("handoff_id"));
    assert!(handoff_text.contains("hnd_"));

    // Verify invariant check
    let inv_resp: serde_json::Value = serde_json::from_str(lines[8]).unwrap();
    assert_eq!(inv_resp["result"]["isError"], false);
    assert!(inv_resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("verified successfully"));
}
