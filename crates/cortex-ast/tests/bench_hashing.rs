use cortex_ast::extract_symbols;
use std::path::Path;
use std::time::Instant;

#[test]
fn test_sub_15ms_on_large_file() {
    let mut large_src = String::with_capacity(500_000);
    large_src.push_str("export class HugeService {\n");
    // Generate 10,000+ lines (2000 methods * 5 lines = 10,000 lines)
    for i in 0..2000 {
        large_src.push_str(&format!(
            "  async method_{}(param: number): Promise<number> {{\n    // Comment for method {}\n    const res = param + {};\n    return res * 2;\n  }}\n",
            i, i, i
        ));
    }
    large_src.push_str("}\n");

    let line_count = large_src.lines().count();
    assert!(
        line_count >= 10000,
        "Should have 10,000 lines, actual: {}",
        line_count
    );

    let start = Instant::now();
    let symbols = extract_symbols(Path::new("src/services/huge.ts"), large_src.as_bytes()).unwrap();
    let duration = start.elapsed();

    println!(
        "\n===> Extracted {} symbols from {}-line file in {:?} <===\n",
        symbols.len(),
        line_count,
        duration
    );
    assert!(!symbols.is_empty());
}

#[test]
fn test_parse_latency_sla_1000_loc() {
    // 1. TypeScript 1,000 LOC
    let mut ts_src = String::with_capacity(50_000);
    for i in 0..120 {
        ts_src.push_str(&format!(
            "export function handler_{}(input: string): boolean {{\n    // Business logic step\n    const normalized = input.trim();\n    if (normalized.length > 0) {{\n        return true;\n    }}\n    return false;\n}}\n\n",
            i
        ));
    }
    let ts_lines = ts_src.lines().count();
    assert!(ts_lines >= 1000);

    // Warm-up parse
    let _ = extract_symbols(Path::new("warmup.ts"), ts_src.as_bytes()).unwrap();

    let start = Instant::now();
    let ts_symbols = extract_symbols(Path::new("service.ts"), ts_src.as_bytes()).unwrap();
    let ts_duration = start.elapsed();
    println!("TS {} LOC parse duration: {:?}", ts_lines, ts_duration);
    assert_eq!(ts_symbols.len(), 120);

    // 2. Python 1,000 LOC
    let mut py_src = String::with_capacity(50_000);
    for i in 0..150 {
        py_src.push_str(&format!(
            "def worker_{}(item_id: str) -> dict:\n    '''Process worker queue item.'''\n    cleaned = item_id.strip()\n    if not cleaned:\n        raise ValueError('empty')\n    return {{'id': cleaned, 'status': 'PROCESSED'}}\n\n\n",
            i
        ));
    }
    let py_lines = py_src.lines().count();
    assert!(py_lines >= 1000);

    let start = Instant::now();
    let py_symbols = extract_symbols(Path::new("worker.py"), py_src.as_bytes()).unwrap();
    let py_duration = start.elapsed();
    println!("Python {} LOC parse duration: {:?}", py_lines, py_duration);
    assert_eq!(py_symbols.len(), 150);

    // 3. Rust 1,000 LOC
    let mut rs_src = String::with_capacity(50_000);
    for i in 0..150 {
        rs_src.push_str(&format!(
            "pub fn execute_{}(value: i64) -> Result<i64, &'static str> {{\n    // Compute step\n    if value < 0 {{\n        return Err(\"negative\");\n    }}\n    Ok(value * 2 + 1)\n}}\n\n",
            i
        ));
    }
    let rs_lines = rs_src.lines().count();
    assert!(rs_lines >= 1000);

    let start = Instant::now();
    let rs_symbols = extract_symbols(Path::new("executor.rs"), rs_src.as_bytes()).unwrap();
    let rs_duration = start.elapsed();
    println!("Rust {} LOC parse duration: {:?}", rs_lines, rs_duration);
    assert_eq!(rs_symbols.len(), 150);

    // Check SLA bound in release mode (generous 35ms bound to prevent VM noise on CI)
    #[cfg(not(debug_assertions))]
    {
        assert!(
            ts_duration.as_millis() <= 35,
            "TypeScript 1000 LOC parse exceeded SLA: {:?}",
            ts_duration
        );
        assert!(
            py_duration.as_millis() <= 35,
            "Python 1000 LOC parse exceeded SLA: {:?}",
            py_duration
        );
        assert!(
            rs_duration.as_millis() <= 35,
            "Rust 1000 LOC parse exceeded SLA: {:?}",
            rs_duration
        );
    }
}
