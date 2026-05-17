//! Mermaid streaming demo - shows top-down and left-right diagrams while
//! streaming. Run with: cargo run --example mermaid-streaming-demo

use std::time::Duration;
use std::{io, thread};

use forge_markdown_stream::StreamdownRenderer;

fn stream_chunks(renderer: &mut StreamdownRenderer<io::Stdout>, chunks: &[&str]) -> io::Result<()> {
    for chunk in chunks {
        renderer.push(chunk)?;
        thread::sleep(Duration::from_millis(150));
    }
    Ok(())
}

fn main() -> io::Result<()> {
    println!("\n{}", "═".repeat(70));
    println!("  MERMAID STREAMING DEMO: Top-Down & Left-Right");
    println!("{}", "═".repeat(70));

    // ── Demo 1: Simple Top-Down (chunked streaming) ──
    println!("\n{}", "─".repeat(70));
    println!("  1. Simple Top-Down Flowchart (streamed chunk by chunk)");
    println!("{}", "─".repeat(70));
    let chunks_td = [
        "```mermaid\n",
        "graph TD\n",
        "  A[Start] --> B{Is it working?}\n",
        "  B -->|Yes| C[Great!]\n",
        "  B -->|No| D[Fix it]\n",
        "  D --> B\n",
        "```\n",
    ];
    let mut r1 = StreamdownRenderer::new(io::stdout(), 80);
    stream_chunks(&mut r1, &chunks_td)?;
    r1.finish()?;
    println!();

    // ── Demo 2: Left-to-Right (chunked streaming) ──
    println!("\n{}", "─".repeat(70));
    println!("  2. Left-to-Right Flowchart (streamed chunk by chunk)");
    println!("{}", "─".repeat(70));
    let chunks_lr = [
        "```mermaid\n",
        "graph LR\n",
        "  A[Input Data] --> B{Valid?}\n",
        "  B -->|Yes| C[Process]\n",
        "  B -->|No| D[Reject]\n",
        "  C --> E[Output]\n",
        "```\n",
    ];
    let mut r2 = StreamdownRenderer::new(io::stdout(), 80);
    stream_chunks(&mut r2, &chunks_lr)?;
    r2.finish()?;
    println!();

    // ── Demo 3: Sequence Diagram (chunked streaming) ──
    println!("\n{}", "─".repeat(70));
    println!("  3. Sequence Diagram (streamed chunk by chunk)");
    println!("{}", "─".repeat(70));
    let chunks_seq = [
        "```mermaid\n",
        "sequenceDiagram\n",
        "  Alice->>Bob: Hello Bob\n",
        "  Bob-->>Alice: Hi Alice\n",
        "  Alice->>Bob: How are you?\n",
        "  Bob-->>Alice: Doing great!\n",
        "```\n",
    ];
    let mut r3 = StreamdownRenderer::new(io::stdout(), 80);
    stream_chunks(&mut r3, &chunks_seq)?;
    r3.finish()?;
    println!();

    // ── Demo 4: Both orientations side by side concept ──
    println!("\n{}", "─".repeat(70));
    println!("  4. Nested decision tree (Top-Down)");
    println!("{}", "─".repeat(70));
    let chunks_nested = [
        "```mermaid\n",
        "graph TD\n",
        "  A[User Request] --> B{Authenticated?}\n",
        "  B -->|Yes| C{Authorized?}\n",
        "  B -->|No| D[Login Page]\n",
        "  C -->|Yes| E[Process Request]\n",
        "  C -->|No| F[Forbidden]\n",
        "  E --> G[Return Result]\n",
        "```\n",
    ];
    let mut r4 = StreamdownRenderer::new(io::stdout(), 80);
    stream_chunks(&mut r4, &chunks_nested)?;
    r4.finish()?;
    println!();

    // ── Demo 5: LR with shapes ──
    println!("\n{}", "─".repeat(70));
    println!("  5. Left-to-Right with different node shapes");
    println!("{}", "─".repeat(70));
    let chunks_shapes = [
        "```mermaid\n",
        "graph LR\n",
        "  A[Rectangle] --> B(Round)\n",
        "  B --> C{Diamond}\n",
        "  C --> D([Stadium])\n",
        "```\n",
    ];
    let mut r5 = StreamdownRenderer::new(io::stdout(), 80);
    stream_chunks(&mut r5, &chunks_shapes)?;
    r5.finish()?;
    println!();

    println!("\n{}", "═".repeat(70));
    println!("  Demo complete!");
    println!("{}\n", "═".repeat(70));

    Ok(())
}
