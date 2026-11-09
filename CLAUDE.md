<!-- git-cortex:begin -->
## Epistemic Memory & Invariant Protocol (git-cortex)
- **Session Start:** Call `cortex_handoff_pack(mode="IMPORT")` before running broad repo searches to load verified call-graphs and active hypotheses.
- **Before Editing Symbols:** Call `cortex_inspect_symbol(filepath)` before modifying existing functions. Respect all ACTIVE Tombstones and Invariant Contracts.
- **On Failed Experiments:** Whenever a test fails and you revert or abandon an approach, call `cortex_record_tombstone` before trying an alternative.
- **Before Committing:** Call `cortex_verify_invariants(diff_scope="STAGED")` to verify cross-file causal contracts.
<!-- git-cortex:end -->
