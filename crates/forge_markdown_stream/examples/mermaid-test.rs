//! Mermaid streaming test - renders diagrams to terminal using tmux.
//! Run with: cargo run --example mermaid-test

use std::io;

use forge_markdown_stream::StreamdownRenderer;

fn main() -> io::Result<()> {
    // Test 1: Simple flowchart TD
    println!("{}", "─".repeat(60));
    println!("  Flowchart TD: A --> B");
    println!("{}", "─".repeat(60));
    let mut r1 = StreamdownRenderer::new(io::stdout(), 80);
    r1.push("```mermaid\n")?;
    r1.push("graph TD\n")?;
    r1.push("  A[Start] --> B[End]\n")?;
    r1.push("```\n")?;
    r1.finish()?;
    println!();

    // Test 2: Flowchart LR
    println!("{}", "─".repeat(60));
    println!("  Flowchart LR: A --> B --> C");
    println!("{}", "─".repeat(60));
    let mut r2 = StreamdownRenderer::new(io::stdout(), 80);
    r2.push("```mermaid\n")?;
    r2.push("graph LR\n")?;
    r2.push("  A[Alpha] --> B[Beta]\n")?;
    r2.push("  B[Beta] --> C[Gamma]\n")?;
    r2.push("```\n")?;
    r2.finish()?;
    println!();

    // Test 3: Sequence diagram
    println!("{}", "─".repeat(60));
    println!("  Sequence Diagram");
    println!("{}", "─".repeat(60));
    let mut r3 = StreamdownRenderer::new(io::stdout(), 80);
    r3.push("```mermaid\n")?;
    r3.push("sequenceDiagram\n")?;
    r3.push("  Alice->>Bob: Hello Bob\n")?;
    r3.push("  Bob-->>Alice: Hi Alice\n")?;
    r3.push("```\n")?;
    r3.finish()?;
    println!();

    // Test 4: Fallback to raw code for unsupported diagram types
    println!("{}", "─".repeat(60));
    println!("  Unsupported type (falls back to code):");
    println!("{}", "─".repeat(60));
    let mut r4 = StreamdownRenderer::new(io::stdout(), 80);
    r4.push("```mermaid\n")?;
    r4.push("classDiagram\n")?;
    r4.push("  class Animal\n")?;
    r4.push("```\n")?;
    r4.finish()?;
    println!();

    // Test 5: Complex flowchart
    println!("{}", "─".repeat(60));
    println!("  Complex Flowchart TD");
    println!("{}", "─".repeat(60));
    let mut r5 = StreamdownRenderer::new(io::stdout(), 80);
    r5.push("```mermaid\n")?;
    r5.push("graph TD\n")?;
    r5.push("  A[Start] --> B{Is it working?}\n")?;
    r5.push("  B -->|Yes| C[Great!]\n")?;
    r5.push("  B -->|No| D[Fix it]\n")?;
    r5.push("  D --> B\n")?;
    r5.push("```\n")?;
    r5.finish()?;
    println!();

    // Test 6: Streaming chunk by chunk (simulating real LLM output)
    println!("{}", "─".repeat(60));
    println!("  Streaming chunk by chunk:");
    println!("{}", "─".repeat(60));
    let chunks = [
        "```mermaid\n",
        "graph LR\n",
        "  A[Input] --> B",
        "[Process]\n",
        "  B --> C",
        "[Output]\n",
        "```\n",
    ];
    let mut r6 = StreamdownRenderer::new(io::stdout(), 80);
    for chunk in &chunks {
        r6.push(chunk)?;
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    r6.finish()?;
    println!();

    Ok(())
}
