mod doctor;
mod hook;
mod init;

use clap::{Parser, Subcommand};
use cortex_ast::extract_symbols;
use cortex_ast::uri::normalize_rel_path;
use cortex_capture::execute_command;
use cortex_core::schema::{CaptureMode, InvariantSeverity};
use cortex_engine::checkpoint::checkpoint_state;
use cortex_engine::handoff::{
    create_handoff_envelope, format_a2a_task_envelope, import_handoff_envelope,
};
use cortex_engine::inspect::inspect_symbol;
use cortex_engine::invariant::{record_invariant, verify_invariants, DiffScope};
use cortex_engine::promote::promote_local_records;
use cortex_engine::rewind::rewind_to_checkpoint;
use cortex_engine::sync::sync_ledger;
use cortex_engine::tombstone::{record_tombstone, RecordTombstoneParams};
use cortex_mcp::McpServer;
use cortex_storage::{CortexDb, CortexLedger, CortexRepo};
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(
    name = "git-cortex",
    version,
    about = "Git-native epistemic context protocol: Working memory, negative knowledge tombstones, and causal invariant locks"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Initialize git-cortex in the current repository
    Init {
        #[arg(long, default_value_t = true)]
        hooks: bool,
        #[arg(long, default_value_t = true)]
        agents_md: bool,
        #[arg(long, default_value_t = true)]
        mcp: bool,
    },

    /// Show working memory, active tombstones, and invariants
    Status {
        #[arg(long)]
        json: bool,
    },

    /// Inspect a file or symbol for active Tombstones and Invariant Locks
    Inspect {
        filepath: String,
        symbol_name: Option<String>,
        #[arg(long)]
        json: bool,
    },

    /// Extract and list AST symbols and hashes for a file
    Symbols {
        filepath: String,
        #[arg(long)]
        json: bool,
    },

    /// Execute a test or build command with passive capture (Edit -> Fail -> Revert detector)
    Exec {
        #[arg(short, long, default_value_t = 0)]
        timeout: u64,
        #[arg(trailing_var_arg = true, required = true)]
        command: Vec<String>,
    },

    /// Manage negative-knowledge Tombstone records
    Tombstone {
        #[command(subcommand)]
        action: TombstoneCommands,
    },

    /// Manage Causal Invariant Lock contracts
    Invariant {
        #[command(subcommand)]
        action: InvariantCommands,
    },

    /// Checkpoint epistemic state and create working tree snapshot
    Checkpoint {
        #[arg(short, long, default_value = "Manual checkpoint")]
        message: String,
    },

    /// Split-brain safe rollback to a checkpoint with immunity tombstone
    Rewind {
        checkpoint_id: String,
        #[arg(short, long)]
        reason: Option<String>,
    },

    /// Cross-harness and sub-agent EpistemicHandoff envelope export/import
    Handoff {
        #[command(subcommand)]
        action: HandoffCommands,
    },

    /// Verify causal invariant locks against staged, working tree, or branch diffs
    VerifyInvariants {
        #[arg(long)]
        staged: bool,
        #[arg(long)]
        working_tree: bool,
        #[arg(long)]
        branch: bool,
    },

    /// Promote local session records to the distributed refs/cortex/ledger
    Promote,

    /// Sync distributed ledger with remote git repository
    Sync { remote: Option<String> },

    /// Re-index distributed ledger records into local SQLite cache
    Reindex,

    /// Internal hook handler
    Hook { name: String },

    /// Run the zero-latency stdio Model Context Protocol (MCP) server
    McpServe,

    /// Run environment and configuration diagnostics
    Doctor,
}

#[derive(Subcommand, Debug)]
enum TombstoneCommands {
    /// Add a new negative-knowledge Tombstone manually
    Add {
        #[arg(short, long, value_delimiter = ',')]
        symbols: Vec<String>,
        #[arg(short = 'm', long)]
        approach: String,
        #[arg(short = 'r', long)]
        reason: String,
        #[arg(long, default_value = "")]
        cmd: String,
        #[arg(long, default_value = "")]
        sig: String,
    },
    /// List all Tombstones
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show details of a Tombstone
    Show { id: String },
    /// Revoke a Tombstone with an append-only lifecycle event
    Revoke {
        id: String,
        #[arg(short = 'r', long, default_value = "Manual revocation via cortex cli")]
        reason: String,
    },
}

#[derive(Subcommand, Debug)]
enum InvariantCommands {
    /// Create a new Invariant Contract
    Add {
        #[arg(long)]
        guarded: String,
        #[arg(long)]
        assertion: String,
        #[arg(long, value_delimiter = ',')]
        upstream: Vec<String>,
        #[arg(long)]
        test: String,
        #[arg(long, default_value = "BLOCKING")]
        severity: String,
    },
    /// List all Invariant Contracts
    List {
        #[arg(long)]
        json: bool,
    },
    /// Reaffirm an Invariant Contract with updated hashes and commit SHA
    Reaffirm {
        id: String,
        #[arg(
            short = 'r',
            long,
            default_value = "Manual reaffirmation via cortex cli"
        )]
        reason: String,
    },
    /// Revoke an Invariant Contract with an append-only lifecycle event
    Revoke {
        id: String,
        #[arg(short = 'r', long, default_value = "Manual revocation via cortex cli")]
        reason: String,
    },
}

#[derive(Subcommand, Debug)]
enum HandoffCommands {
    /// Export an EpistemicHandoff envelope (<= 4 KB)
    Export {
        #[arg(long)]
        a2a: bool,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Import an EpistemicHandoff envelope
    Import {
        #[arg(long)]
        file: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    let mut raw_args: Vec<String> = std::env::args().collect();
    if raw_args.len() > 1 && raw_args[1] == "cortex" {
        raw_args.remove(1);
    }

    let cli = Cli::parse_from(raw_args);

    let repo = CortexRepo::discover(None)?;
    let db_path = repo.state_db_path();
    let mut db = CortexDb::open(&db_path)?;

    match cli.command {
        Commands::Init {
            hooks,
            agents_md,
            mcp,
        } => {
            let actions = init::run_init(
                &repo,
                &init::InitOptions {
                    install_hooks: hooks,
                    setup_agents_md: agents_md,
                    setup_mcp_config: mcp,
                },
            )?;
            println!("âœ¨ git-cortex initialized successfully!");
            for a in actions {
                println!("  â€¢ {}", a);
            }
        }
        Commands::Status { json } => {
            let branch = repo.current_branch()?;
            let head = repo.head_commit_sha()?;
            let wt_hash = repo.working_tree_dirty_hash()?;
            let tombstones = db.list_records_by_kind("TOMBSTONE")?;
            let invariants = db.list_records_by_kind("INVARIANT")?;
            let edges = db.get_call_graph_edges(&branch)?;
            let hyps = db.get_hypotheses(&branch)?;

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "branch": branch,
                        "head_commit": head,
                        "working_tree_hash": wt_hash,
                        "tombstones_count": tombstones.len(),
                        "invariants_count": invariants.len(),
                        "call_graph_edges": edges.len(),
                        "active_hypotheses": hyps.len()
                    }))?
                );
            } else {
                println!("=== git-cortex Epistemic State ===");
                println!(
                    "Branch:            {} (HEAD: {})",
                    branch,
                    &head[..8.min(head.len())]
                );
                println!("Working Tree:      {}", wt_hash);
                println!("Tombstones:        {} recorded", tombstones.len());
                println!("Invariants:        {} active contracts", invariants.len());
                println!("Call Graph Edges:  {} verified", edges.len());
                println!("Hypotheses:        {} tracked", hyps.len());
            }
        }
        Commands::Inspect {
            filepath,
            symbol_name,
            json,
        } => {
            let res = inspect_symbol(&repo, &db, &filepath, symbol_name.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&res)?);
            } else {
                println!("{}", res.formatted_markdown);
            }
        }
        Commands::Symbols { filepath, json } => {
            let norm_path = normalize_rel_path(Path::new(&filepath));
            let full = repo.workdir.join(&norm_path);
            let src = std::fs::read(&full)?;
            let symbols = extract_symbols(Path::new(&norm_path), &src)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&symbols)?);
            } else {
                println!("Symbols in {}:", norm_path);
                for s in symbols {
                    println!(
                        "  â€¢ {:<12} {:<40} {}",
                        format!("{:?}", s.kind),
                        s.scope_chain,
                        s.ast_hash
                    );
                }
            }
        }
        Commands::Exec { timeout, command } => {
            let timeout_opt = if timeout > 0 { Some(timeout) } else { None };
            let exit_code = execute_command(&repo, &mut db, &command, timeout_opt)?;
            std::process::exit(exit_code);
        }
        Commands::Tombstone { action } => match action {
            TombstoneCommands::Add {
                symbols,
                approach,
                reason,
                cmd,
                sig,
            } => {
                let rec = record_tombstone(
                    &repo,
                    &mut db,
                    RecordTombstoneParams {
                        target_symbols: &symbols,
                        attempted_approach: &approach,
                        failure_reason: &reason,
                        test_command: &cmd,
                        error_signature: &sig,
                        author_harness: "cortex-cli",
                        capture_mode: CaptureMode::Manual,
                    },
                )?;
                println!(
                    "Recorded Tombstone '{}' bound to {} symbol(s).",
                    rec.id,
                    rec.target_symbols.len()
                );
            }
            TombstoneCommands::List { json } => {
                let tombstones = db.list_records_by_kind("TOMBSTONE")?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&tombstones)?);
                } else {
                    println!("Tombstones ({} total):", tombstones.len());
                    for t in tombstones {
                        println!("  â€¢ {} | {} | {}", t.id, t.created_at, t.branch);
                    }
                }
            }
            TombstoneCommands::Show { id } => {
                if let Some(r) = db.get_record(&id)? {
                    println!("{}", r.json);
                } else {
                    eprintln!("Tombstone '{}' not found", id);
                }
            }
            TombstoneCommands::Revoke { id, reason } => {
                let evt = cortex_engine::revoke_record(&repo, &mut db, &id, &reason, "cortex-cli")?;
                println!(
                    "Revoked Tombstone '{}' via LifecycleEvent '{}'.",
                    id, evt.event_id
                );
            }
        },
        Commands::Invariant { action } => match action {
            InvariantCommands::Add {
                guarded,
                assertion,
                upstream,
                test,
                severity,
            } => {
                let sev = if severity.eq_ignore_ascii_case("WARNING") {
                    InvariantSeverity::Warning
                } else {
                    InvariantSeverity::Blocking
                };
                let contract =
                    record_invariant(&repo, &mut db, &guarded, &assertion, &upstream, &test, sev)?;
                println!(
                    "Created InvariantContract '{}' guarding '{}'.",
                    contract.invariant_id, contract.guarded_symbol.symbol_uri
                );
            }
            InvariantCommands::List { json } => {
                let invariants = db.list_records_by_kind("INVARIANT")?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&invariants)?);
                } else {
                    println!("Invariant Contracts ({} total):", invariants.len());
                    for inv in invariants {
                        println!("  • {} | {} | {}", inv.id, inv.created_at, inv.branch);
                    }
                }
            }
            InvariantCommands::Reaffirm { id, reason } => {
                let contract =
                    cortex_engine::reaffirm_invariant(&repo, &mut db, &id, &reason, "cortex-cli")?;
                println!(
                    "Reaffirmed InvariantContract '{}'. Updated guarded hash: {}",
                    contract.invariant_id, contract.guarded_symbol.ast_hash
                );
            }
            InvariantCommands::Revoke { id, reason } => {
                let evt = cortex_engine::revoke_record(&repo, &mut db, &id, &reason, "cortex-cli")?;
                println!(
                    "Revoked InvariantContract '{}' via LifecycleEvent '{}'.",
                    id, evt.event_id
                );
            }
        },
        Commands::Checkpoint { message } => {
            let res = checkpoint_state(&repo, &mut db, &message, &[], &[])?;
            println!(
                "Created Checkpoint '{}' on branch '{}'.",
                res.checkpoint_id, res.branch
            );
            println!("Snapshot ref: {}", res.snapshot_ref);
        }
        Commands::Rewind {
            checkpoint_id,
            reason,
        } => {
            let res = rewind_to_checkpoint(&repo, &mut db, &checkpoint_id, reason.as_deref())?;
            println!("{}", res.status_message);
            println!("Backup ref created at: {}", res.backup_ref);
            println!(
                "Immunity Tombstone recorded: {}",
                res.synthesized_tombstone_id
            );
        }
        Commands::Handoff { action } => match action {
            HandoffCommands::Export { a2a, out } => {
                let envelope = create_handoff_envelope(&repo, &db, None, None)?;
                let output_text = if a2a {
                    let task = format_a2a_task_envelope(
                        &envelope,
                        "task_handoff",
                        "Continue development",
                    )?;
                    serde_json::to_string_pretty(&task)?
                } else {
                    serde_json::to_string_pretty(&envelope)?
                };

                if let Some(out_path) = out {
                    std::fs::write(&out_path, &output_text)?;
                    println!("Exported EpistemicHandoff to {}", out_path.display());
                } else {
                    println!("{}", output_text);
                }
            }
            HandoffCommands::Import { file } => {
                let content = if let Some(in_path) = file {
                    std::fs::read_to_string(&in_path)?
                } else {
                    use std::io::Read;
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s)?;
                    s
                };
                let envelope = serde_json::from_str(&content)?;
                let branch = repo.current_branch()?;
                import_handoff_envelope(&repo, &mut db, &envelope, &branch)?;
                println!("Imported EpistemicHandoff into branch '{}'.", branch);
            }
        },
        Commands::VerifyInvariants {
            staged,
            working_tree: _,
            branch,
        } => {
            let scope = if staged {
                DiffScope::Staged
            } else if branch {
                DiffScope::Branch
            } else {
                DiffScope::WorkingTree
            };
            let report = verify_invariants(&repo, &db, scope)?;
            if report.passed {
                println!(
                    "âœ“ All {} invariant contracts verified successfully against {:?}.",
                    report.checked_count, scope
                );
            } else {
                eprintln!(
                    "âŒ {} Invariant violation(s) detected in {:?}:",
                    report.violations.len(),
                    scope
                );
                for v in &report.violations {
                    eprintln!("  â€¢ Invariant: {}", v.invariant_id);
                    eprintln!("    Guarded:   {}", v.guarded_symbol);
                    eprintln!("    Upstream:  {}", v.upstream_symbol);
                    eprintln!("    Assertion: {}", v.assertion);
                    eprintln!("    Run Test:  {}\n", v.verification_test_cmd);
                }
                std::process::exit(1);
            }
        }
        Commands::Promote => {
            let count = promote_local_records(&repo, &mut db)?;
            println!("Promoted {} record(s) to refs/cortex/ledger.", count);
        }
        Commands::Sync { remote } => {
            let msg = sync_ledger(&repo, &mut db, remote.as_deref())?;
            println!("{}", msg);
        }
        Commands::Reindex => {
            let count = CortexLedger::reindex_into_db(&repo.repo, &mut db)?;
            println!("Re-indexed {} record(s) from refs/cortex/ledger.", count);
        }
        Commands::Hook { name } => {
            let exit_code = hook::handle_hook(&repo, &mut db, &name)?;
            std::process::exit(exit_code);
        }
        Commands::McpServe => {
            let mut server = McpServer::new(repo, db);
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            server.run_stdio(stdin.lock(), stdout.lock());
        }
        Commands::Doctor => {
            doctor::run_doctor(&repo, &db)?;
        }
    }

    Ok(())
}
