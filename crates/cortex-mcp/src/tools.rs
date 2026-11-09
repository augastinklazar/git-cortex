use serde_json::{json, Value};

pub fn list_tools() -> Value {
    json!({
        "tools": [
            {
                "name": "cortex_inspect_symbol",
                "description": "Retrieve active negative-knowledge Tombstones (failed past approaches) and Causal Invariant Locks for a file or AST symbol BEFORE planning or applying edits.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "filepath": {
                            "type": "string",
                            "description": "Repository-relative path to the source file (e.g., 'src/auth/token_manager.ts')."
                        },
                        "symbol_name": {
                            "type": "string",
                            "description": "Optional canonical class/function identifier (e.g., 'TokenManager.refreshSession')."
                        }
                    },
                    "required": ["filepath"]
                }
            },
            {
                "name": "cortex_record_tombstone",
                "description": "Record a disproved hypothesis or failed code modification as an AST-bound Tombstone so neither this agent (post-compaction) nor downstream agents repeat the dead end.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "target_symbols": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "List of symbol URIs modified during the failed attempt."
                        },
                        "attempted_approach": {
                            "type": "string",
                            "description": "Concise technical summary of the change that was attempted."
                        },
                        "failure_reason": {
                            "type": "string",
                            "description": "Root cause explaining why the attempt failed (test failure, deadlock, type violation)."
                        },
                        "test_command": {
                            "type": "string",
                            "description": "Command that demonstrated the failure."
                        },
                        "error_signature": {
                            "type": "string",
                            "description": "Key assertion or error output line."
                        }
                    },
                    "required": ["target_symbols", "attempted_approach", "failure_reason"]
                }
            },
            {
                "name": "cortex_checkpoint_state",
                "description": "Checkpoint working epistemic memory (call graph edges and active hypotheses) and create a git working tree recovery snapshot before context compaction.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "objective": {
                            "type": "string",
                            "description": "Current goal or engineering objective being pursued."
                        },
                        "verified_edges": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "caller": { "type": "string" },
                                    "callee": { "type": "string" },
                                    "relationship": { "type": "string" },
                                    "verified_ast_hash": { "type": "string" }
                                },
                                "required": ["caller", "callee", "relationship", "verified_ast_hash"]
                            },
                            "description": "Call graph edges verified by this session."
                        },
                        "active_hypotheses": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "hypothesis_id": { "type": "string" },
                                    "statement": { "type": "string" },
                                    "confidence": { "type": "number" },
                                    "status": { "type": "string" },
                                    "next_verification_step": { "type": "string" }
                                },
                                "required": ["hypothesis_id", "statement", "confidence", "status", "next_verification_step"]
                            },
                            "description": "Active hypotheses currently under test."
                        }
                    },
                    "required": ["objective"]
                }
            },
            {
                "name": "cortex_handoff_pack",
                "description": "Export or import a compact (<=4 KB) EpistemicHandoff envelope when starting a session or handing off between sub-agents or harnesses.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "mode": {
                            "type": "string",
                            "enum": ["IMPORT", "EXPORT"],
                            "description": "IMPORT to load prior context, EXPORT to create a handoff envelope."
                        },
                        "branch": {
                            "type": "string",
                            "description": "Optional branch context."
                        },
                        "envelope": {
                            "type": "object",
                            "description": "EpistemicHandoff payload to ingest when mode is IMPORT."
                        }
                    },
                    "required": ["mode"]
                }
            },
            {
                "name": "cortex_verify_invariants",
                "description": "Verify causal invariant locks against staged, working tree, or branch diffs before committing or opening a pull request.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "diff_scope": {
                            "type": "string",
                            "enum": ["STAGED", "WORKING_TREE", "BRANCH"],
                            "default": "WORKING_TREE",
                            "description": "Scope of modifications to verify against invariant dependencies."
                        }
                    }
                }
            },
            {
                "name": "cortex_record_invariant",
                "description": "Create a new Causal Invariant Lock guarding a critical architectural symbol against upstream dependency drift.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "guarded_symbol": {
                            "type": "string",
                            "description": "Symbol URI of the function/method to protect (e.g. 'src/billing/ledger.ts#LedgerService.issueRefund')."
                        },
                        "assertion": {
                            "type": "string",
                            "description": "Architectural requirement or contract precondition."
                        },
                        "upstream_dependencies": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "List of upstream symbol URIs this contract depends upon."
                        },
                        "verification_test_cmd": {
                            "type": "string",
                            "description": "Command to run to verify this contract."
                        },
                        "severity": {
                            "type": "string",
                            "enum": ["BLOCKING", "WARNING"],
                            "default": "BLOCKING"
                        }
                    },
                    "required": ["guarded_symbol", "assertion", "verification_test_cmd"]
                }
            }
        ]
    })
}
