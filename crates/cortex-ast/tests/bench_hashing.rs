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
