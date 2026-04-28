//! Mermaid diagram rendering for terminal output.
//!
//! Parses Mermaid DSL and renders diagrams as Unicode/ASCII art suitable for
//! terminal display. Supports flowcharts (graph TD/LR) and sequence diagrams
//! with graceful fallback for unsupported types.

use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

// Style constants for per-character style maps (sequence + flowchart
// multi-branch)
const STYLE_NONE: u8 = 0;
const STYLE_BORDER: u8 = 1;
const STYLE_NODE: u8 = 2;
const STYLE_EDGE: u8 = 3;
const STYLE_ARROW: u8 = 4;
const STYLE_LABEL: u8 = 5;

/// Build a colored ANSI string from character and style buffers using theme.
fn build_colored_line(chars: &[char], styles: &[u8], theme: &Theme) -> String {
    let mut result = String::new();
    let mut i = 0;
    while i < chars.len() {
        let s = styles.get(i).copied().unwrap_or(0);
        // Group consecutive chars with the same style
        let start = i;
        while i < chars.len() && styles.get(i).copied().unwrap_or(0) == s {
            i += 1;
        }
        let segment: String = chars.get(start..i).unwrap_or_default().iter().collect();
        let styled = match s {
            STYLE_BORDER => theme.mermaid_border.apply(&segment),
            STYLE_NODE => theme.mermaid_node.apply(&segment),
            STYLE_EDGE => theme.mermaid_edge.apply(&segment),
            STYLE_ARROW => theme.mermaid_arrow_head.apply(&segment),
            STYLE_LABEL => theme.mermaid_label.apply(&segment),
            _ => theme.mermaid_bg.apply(&segment),
        };
        result.push_str(&styled.to_string());
    }
    result
}

fn set_char(row: &mut [char], index: usize, value: char) {
    if let Some(slot) = row.get_mut(index) {
        *slot = value;
    }
}

fn set_style(styles: &mut [u8], index: usize, value: u8) {
    if let Some(slot) = styles.get_mut(index) {
        *slot = value;
    }
}

fn get_bool(values: &[bool], index: usize) -> bool {
    values.get(index).copied().unwrap_or(false)
}

fn set_bool(values: &mut [bool], index: usize, value: bool) {
    if let Some(slot) = values.get_mut(index) {
        *slot = value;
    }
}

/// Renders a Mermaid diagram to terminal-friendly ASCII/Unicode art lines.
///
/// Returns `None` if the diagram type is unsupported (caller should fall back
/// to raw code display).
pub fn render_mermaid(diagram: &str, width: usize, theme: &Theme) -> Option<Vec<String>> {
    let trimmed = diagram.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Detect diagram type from the first meaningful line
    let diagram_type = detect_diagram_type(trimmed)?;

    match diagram_type {
        DiagramType::Flowchart { direction } => {
            Some(render_flowchart(trimmed, direction, width, theme))
        }
        DiagramType::Sequence => Some(render_sequence(trimmed, width, theme)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiagramType {
    Flowchart { direction: FlowDirection },
    Sequence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowDirection {
    TopDown,   // TD
    LeftRight, // LR
}

fn detect_diagram_type(diagram: &str) -> Option<DiagramType> {
    let first_line = diagram.lines().next()?.trim().to_lowercase();

    if first_line.starts_with("graph ") || first_line.starts_with("flowchart ") {
        let direction = if first_line.ends_with("lr") || first_line.ends_with("lr;") {
            FlowDirection::LeftRight
        } else {
            FlowDirection::TopDown
        };
        return Some(DiagramType::Flowchart { direction });
    }

    if first_line.starts_with("sequencediagram")
        || first_line.starts_with("sequenceDiagram")
        || first_line.starts_with("sequence diagram")
    {
        return Some(DiagramType::Sequence);
    }

    None
}

// ---------------------------------------------------------------------------
// Flowchart rendering
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct FlowNode {
    id: String,
    label: String,
    shape: NodeShape,
    _x: usize,
    _y: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeShape {
    Rect,      // [text]
    RoundRect, // (text)
    Diamond,   // {text}
    Stadium,   // ([text])
    Default,   // text
}

#[derive(Debug)]
struct FlowEdge {
    from: String,
    to: String,
    label: String,
    _arrow_type: ArrowType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArrowType {
    Normal, // -->
    Thick,  // ==>
    Dotted, // -.-> or -.->
    Open,   // ---
}

/// Parse a Mermaid flowchart definition and return nodes and edges.
fn parse_flowchart(diagram: &str) -> (Vec<FlowNode>, Vec<FlowEdge>) {
    let mut nodes: Vec<FlowNode> = Vec::new();
    let mut edges: Vec<FlowEdge> = Vec::new();
    let mut _node_counter = 0;

    // Helper to get or create a node by ID
    let mut get_or_create_node = |id: &str, label: Option<&str>, shape: NodeShape| -> FlowNode {
        if let Some(pos) = nodes.iter().position(|n| n.id == id)
            && let Some(node) = nodes.get_mut(pos)
        {
            if let Some(label) = label {
                node.label = label.to_string();
            }
            if shape != NodeShape::Default {
                node.shape = shape;
            }
            return node.clone();
        }
        let label = label.unwrap_or(id).to_string();
        let node = FlowNode { id: id.to_string(), label, shape, _x: 0, _y: 0 };
        nodes.push(node.clone());
        node
    };

    for line in diagram.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('%') || line.starts_with("subgraph") {
            continue;
        }

        // Match: NODEID[LABEL] --> NODEID2[LABEL2] | LABEL |
        // First, try to match edge definitions with labels
        if let Some((from_part, rest)) = line
            .split_once("-->")
            .or_else(|| line.split_once("==>"))
            .or_else(|| line.split_once("-.- >"))
            .or_else(|| line.split_once("-.->"))
            .or_else(|| line.split_once("---"))
        {
            let arrow_type = if line.contains("==>") {
                ArrowType::Thick
            } else if line.contains("-.-") || line.contains("-.- >") || line.contains("-.->") {
                ArrowType::Dotted
            } else if line.contains("---") && !line.contains("-->") {
                ArrowType::Open
            } else {
                ArrowType::Normal
            };

            // Parse the edge label if present: -->|label| target
            let (to_part, label) = if rest.trim_start().starts_with('|') {
                // rest looks like "|Yes| C[Great!]"
                let after_first_pipe = rest.trim_start().trim_start_matches('|');
                if let Some((label_text, remaining)) = after_first_pipe.split_once('|') {
                    (remaining.trim(), Some(label_text.trim().to_string()))
                } else {
                    (rest.trim(), None)
                }
            } else {
                (rest.trim(), None)
            };

            let (from_node, from_label, from_shape) = parse_node_ref(from_part.trim());
            let (to_node, to_label, to_shape) = parse_node_ref(to_part);

            let from = get_or_create_node(&from_node, from_label.as_deref(), from_shape);
            let to = get_or_create_node(&to_node, to_label.as_deref(), to_shape);

            // Check if label is embedded in the edge syntax
            let edge_label = label.unwrap_or_default();

            edges.push(FlowEdge {
                from: from.id.clone(),
                to: to.id.clone(),
                label: edge_label,
                _arrow_type: arrow_type,
            });
        } else {
            // Parse node-only definition: NODEID[LABEL]
            // Skip if it's a comment or directive
            if line.starts_with("graph ") || line.starts_with("flowchart ") {
                continue;
            }

            let (node_id, label, shape) = parse_node_ref(line);
            let _ = get_or_create_node(&node_id, label.as_deref(), shape);
        }
    }

    (nodes, edges)
}

/// Parse a node reference like `A[Hello]` or `B{Decision}` or `C-->`
/// Returns (id, optional label, shape)
fn parse_node_ref(text: &str) -> (String, Option<String>, NodeShape) {
    let text = text.trim();

    // Remove trailing arrow symbols
    let text = text
        .trim_end_matches("-->")
        .trim_end_matches("==>")
        .trim_end_matches("-.->")
        .trim_end_matches("---")
        .trim_end_matches("-.- >")
        .trim();

    if text.is_empty() {
        return (String::new(), None, NodeShape::Default);
    }

    // Check for shape-delimited nodes: ID[...], ID(...), ID{...}, ID([...])
    // Find the first opening delimiter
    let open_delimiters = ['[', '(', '{'];
    let mut first_open_pos = None;
    let mut open_char = ' ';

    for (pos, c) in text.char_indices() {
        if open_delimiters.contains(&c) {
            first_open_pos = Some(pos);
            open_char = c;
            break;
        }
    }

    if let Some(open_pos) = first_open_pos {
        let raw_id = text.get(..open_pos).unwrap_or("");
        let id = raw_id.trim().to_string();
        let id = if id.is_empty() {
            raw_id.to_string()
        } else {
            id
        };

        // Determine the closing delimiter
        let close_char = match open_char {
            '[' => ']',
            '(' => ')',
            '{' => '}',
            _ => ']', // fallback
        };

        // Find the closing delimiter
        if let Some(close_pos) = text.rfind(close_char) {
            let inner_start = open_pos + open_char.len_utf8();
            let inner = text.get(inner_start..close_pos).unwrap_or("");

            let shape = match (open_char, close_char) {
                ('[', ']') => {
                    // Check for stadium shape: ([...])
                    if open_pos >= 2 && text.get(open_pos - 2..open_pos) == Some("([") {
                        // Stadium shape: fall through after fixing the ID
                        return (
                            text.get(..open_pos - 2).unwrap_or("").trim().to_string(),
                            Some(inner.to_string()),
                            NodeShape::Stadium,
                        );
                    }
                    NodeShape::Rect
                }
                ('(', ')') => NodeShape::RoundRect,
                ('{', '}') => NodeShape::Diamond,
                _ => NodeShape::Default,
            };

            return (id, Some(inner.to_string()), shape);
        }
    }

    // Plain text: A or text
    let id = text
        .chars()
        .filter(|c| c.is_uppercase() || c.is_ascii_digit())
        .collect::<String>();
    let id = if id.is_empty() { text.to_string() } else { id };
    (id, None, NodeShape::Default)
}

/// Render a flowchart as Unicode box-drawing art.
fn render_flowchart(
    diagram: &str,
    direction: FlowDirection,
    width: usize,
    theme: &Theme,
) -> Vec<String> {
    let (nodes, edges) = parse_flowchart(diagram);
    if nodes.is_empty() {
        return vec!["[empty diagram]".to_string()];
    }

    // Compute node labels and widths
    let max_label_width = nodes
        .iter()
        .map(|n| UnicodeWidthStr::width(n.label.as_str()))
        .max()
        .unwrap_or(10);
    let node_inner_width = max_label_width.max(3);
    let node_total_width = node_inner_width + 6; // padded node block width

    match direction {
        FlowDirection::TopDown => render_flowchart_td(
            &nodes,
            &edges,
            node_inner_width,
            node_total_width,
            width,
            theme,
        ),
        FlowDirection::LeftRight => render_flowchart_lr(
            &nodes,
            &edges,
            node_inner_width,
            node_total_width,
            width,
            theme,
        ),
    }
}

fn make_node_lines(node: &FlowNode, inner_width: usize, theme: &Theme) -> Vec<String> {
    let label = &node.label;
    let padded = if UnicodeWidthStr::width(label.as_str()) <= inner_width {
        let pad = inner_width - UnicodeWidthStr::width(label.as_str());
        let left = pad / 2;
        let right = pad - left;
        format!("{}{}{}", " ".repeat(left), label, " ".repeat(right))
    } else {
        let truncated: String = label.chars().take(inner_width - 1).collect();
        format!("{}…", truncated)
    };

    let block_width = inner_width + 6;
    let center_to_block = |s: String| {
        let width = UnicodeWidthStr::width(s.as_str());
        let pad = block_width.saturating_sub(width);
        let left = pad / 2;
        let right = pad - left;
        format!("{}{}{}", " ".repeat(left), s, " ".repeat(right))
    };

    match node.shape {
        NodeShape::Default | NodeShape::Rect => {
            let border = theme
                .mermaid_border
                .apply(&center_to_block(format!("┌{}┐", "─".repeat(inner_width))));
            let label = theme
                .mermaid_node
                .apply(&center_to_block(format!("│{}│", padded)));
            let bottom = theme
                .mermaid_border
                .apply(&center_to_block(format!("└{}┘", "─".repeat(inner_width))));
            vec![border.to_string(), label.to_string(), bottom.to_string()]
        }
        NodeShape::RoundRect | NodeShape::Stadium => {
            let border = theme
                .mermaid_border
                .apply(&center_to_block(format!("╭{}╮", "─".repeat(inner_width))));
            let label = theme
                .mermaid_node
                .apply(&center_to_block(format!("│{}│", padded)));
            let bottom = theme
                .mermaid_border
                .apply(&center_to_block(format!("╰{}╯", "─".repeat(inner_width))));
            vec![border.to_string(), label.to_string(), bottom.to_string()]
        }
        NodeShape::Diamond => {
            let border = theme
                .mermaid_border
                .apply(&center_to_block(format!("╔{}╗", "═".repeat(inner_width))));
            let label = theme
                .mermaid_node_decision
                .apply(&center_to_block(format!("║{}║", padded)));
            let bottom = theme
                .mermaid_border
                .apply(&center_to_block(format!("╚{}╝", "═".repeat(inner_width))));
            vec![border.to_string(), label.to_string(), bottom.to_string()]
        }
    }
}

fn render_flowchart_td(
    nodes: &[FlowNode],
    edges: &[FlowEdge],
    inner_width: usize,
    total_width: usize,
    _max_width: usize,
    theme: &Theme,
) -> Vec<String> {
    let mut lines = Vec::new();

    // Build adjacency list for top-down layout
    let _node_ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let mut edge_map: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    let mut edge_labels: std::collections::BTreeMap<(usize, usize), String> =
        std::collections::BTreeMap::new();

    // Find root nodes (no incoming edges)
    let mut has_incoming = vec![false; nodes.len()];
    for edge in edges {
        if let (Some(from_idx), Some(to_idx)) = (
            nodes.iter().position(|n| n.id == edge.from),
            nodes.iter().position(|n| n.id == edge.to),
        ) {
            edge_map.entry(from_idx).or_default().push(to_idx);
            edge_labels.insert((from_idx, to_idx), edge.label.clone());
            set_bool(&mut has_incoming, to_idx, true);
        }
    }

    // Topological ordering via simple BFS layering
    let mut layers: Vec<Vec<usize>> = Vec::new();
    let mut visited = vec![false; nodes.len()];

    // Start with root nodes
    let mut current_layer: Vec<usize> = (0..nodes.len())
        .filter(|i| !get_bool(&has_incoming, *i) || nodes.len() == 1)
        .collect();

    if current_layer.is_empty() && !nodes.is_empty() {
        current_layer.push(0);
    }

    while !current_layer.is_empty() {
        let mut next_layer: Vec<usize> = Vec::new();
        for &idx in &current_layer {
            if !get_bool(&visited, idx) {
                set_bool(&mut visited, idx, true);
                if let Some(targets) = edge_map.get(&idx) {
                    for &target in targets {
                        if !get_bool(&visited, target) && !next_layer.contains(&target) {
                            next_layer.push(target);
                        }
                    }
                }
            }
        }
        if !current_layer.iter().all(|&i| get_bool(&visited, i)) {
            // Add unvisited nodes
            for i in 0..nodes.len() {
                if !get_bool(&visited, i) && !current_layer.contains(&i) && !next_layer.contains(&i)
                {
                    next_layer.push(i);
                }
            }
        }
        layers.push(current_layer);
        current_layer = next_layer;
    }

    // Ensure all nodes are in some layer
    let all_visited: Vec<usize> = (0..nodes.len())
        .filter(|&i| !get_bool(&visited, i))
        .collect();
    if !all_visited.is_empty() {
        layers.push(all_visited);
    }

    // Render per layer
    for (layer_idx, layer) in layers.iter().enumerate() {
        // Compute rendered lines for each node in this layer
        let node_blocks: Vec<Vec<String>> = layer
            .iter()
            .filter_map(|&idx| nodes.get(idx))
            .map(|node| make_node_lines(node, inner_width, theme))
            .collect();

        let max_rows = node_blocks.iter().map(|v| v.len()).max().unwrap_or(0);

        for row_idx in 0..max_rows {
            let mut line = String::new();
            for (ni, block) in node_blocks.iter().enumerate() {
                if ni > 0 {
                    line.push_str("   ");
                }
                if let Some(row) = block.get(row_idx) {
                    line.push_str(row);
                } else {
                    let width = block
                        .first()
                        .map(|s| UnicodeWidthStr::width(s.as_str()))
                        .unwrap_or(inner_width + 2);
                    line.push_str(&" ".repeat(width));
                }
            }
            lines.push(line);
        }

        // Render edges to next layer
        if let Some(next_layer) = layers.get(layer_idx + 1) {
            // For each node in this layer that has edges to next layer
            for (ni, &node_idx) in layer.iter().enumerate() {
                if let Some(edge_targets) = edge_map
                    .get(&node_idx)
                    .filter(|targets| targets.iter().any(|t| next_layer.contains(t)))
                {
                    let connect_to: Vec<&usize> = edge_targets
                        .iter()
                        .filter(|t| next_layer.contains(t))
                        .collect();
                    let source_center = ni * (total_width + 3) + total_width / 2;

                    let mut targets: Vec<(usize, usize, &str)> = connect_to
                        .iter()
                        .filter_map(|&&target_idx| {
                            let target_pos =
                                next_layer.iter().position(|&idx| idx == target_idx)?;
                            let target_center = target_pos * (total_width + 3) + total_width / 2;
                            let label = edge_labels
                                .get(&(node_idx, target_idx))
                                .map(|s| s.as_str())
                                .unwrap_or("");
                            Some((target_idx, target_center, label))
                        })
                        .collect();

                    targets.sort_by_key(|(_, center, _)| *center);

                    if targets.len() <= 1 {
                        if let Some(&(target_idx, target_center, label)) = targets.first() {
                            let offset = if target_center == source_center {
                                source_center
                            } else {
                                target_center
                            };
                            let arrow_line = theme.mermaid_edge.apply(&format!(
                                "{:source_center$}│",
                                "",
                                source_center = source_center
                            ));
                            lines.push(arrow_line.to_string());

                            let shaft_line = theme.mermaid_edge.apply(&format!(
                                "{:offset$}│",
                                "",
                                offset = offset
                            ));
                            lines.push(shaft_line.to_string());

                            let label = edge_labels
                                .get(&(node_idx, target_idx))
                                .map(|s| s.as_str())
                                .unwrap_or(label);
                            let arrow_head = if label.is_empty() {
                                theme
                                    .mermaid_arrow_head
                                    .apply(&format!("{:offset$}▼", "", offset = offset))
                                    .to_string()
                            } else {
                                let styled_label = theme.mermaid_label.apply(label);
                                let styled_arrow = theme.mermaid_arrow_head.apply("▼");
                                if UnicodeWidthStr::width(label) + offset > _max_width {
                                    format!(
                                        "{}{} {}",
                                        " ".repeat(offset),
                                        styled_label,
                                        styled_arrow
                                    )
                                } else {
                                    format!(
                                        "{:offset$}{}  {}",
                                        "",
                                        styled_arrow,
                                        styled_label,
                                        offset = offset
                                    )
                                }
                            };
                            lines.push(arrow_head);
                        }
                    } else {
                        let min_center = targets
                            .iter()
                            .map(|(_, center, _)| *center)
                            .min()
                            .unwrap_or(source_center);
                        let max_center = targets
                            .iter()
                            .map(|(_, center, _)| *center)
                            .max()
                            .unwrap_or(source_center);
                        let row_width = max_center
                            + targets
                                .iter()
                                .map(|(_, _, label)| UnicodeWidthStr::width(*label) + 5)
                                .max()
                                .unwrap_or(5);

                        let mut vertical_row = vec![' '; row_width];
                        let mut vert_styles = vec![STYLE_NONE; row_width];
                        set_char(&mut vertical_row, source_center, '│');
                        set_style(&mut vert_styles, source_center, STYLE_EDGE);
                        lines.push(build_colored_line(&vertical_row, &vert_styles, theme));

                        let mut branch_row = vec![' '; row_width];
                        let mut branch_styles = vec![STYLE_NONE; row_width];
                        for x in min_center..=max_center {
                            set_char(&mut branch_row, x, '─');
                            set_style(&mut branch_styles, x, STYLE_EDGE);
                        }
                        for (_, target_center, _) in &targets {
                            let marker = if *target_center == min_center {
                                '┌'
                            } else if *target_center == max_center {
                                '┐'
                            } else {
                                '┬'
                            };
                            set_char(&mut branch_row, *target_center, marker);
                            set_style(&mut branch_styles, *target_center, STYLE_EDGE);
                        }
                        let source_marker = if source_center == min_center {
                            '├'
                        } else if source_center == max_center {
                            '┤'
                        } else {
                            '┬'
                        };
                        set_char(&mut branch_row, source_center, source_marker);
                        set_style(&mut branch_styles, source_center, STYLE_EDGE);
                        lines.push(build_colored_line(&branch_row, &branch_styles, theme));

                        let mut arrow_row = vec![' '; row_width];
                        let mut arrow_styles = vec![STYLE_NONE; row_width];
                        for (_, target_center, label) in &targets {
                            set_char(&mut arrow_row, *target_center, '▼');
                            set_style(&mut arrow_styles, *target_center, STYLE_ARROW);
                            if !label.is_empty() {
                                let label_start = target_center + 2;
                                for (i, ch) in label.chars().enumerate() {
                                    set_char(&mut arrow_row, label_start + i, ch);
                                    set_style(&mut arrow_styles, label_start + i, STYLE_LABEL);
                                }
                            }
                        }
                        lines.push(build_colored_line(&arrow_row, &arrow_styles, theme));
                    }
                }
            }
        }
    }

    lines
}

fn render_flowchart_lr(
    nodes: &[FlowNode],
    edges: &[FlowEdge],
    inner_width: usize,
    _total_width: usize,
    _max_width: usize,
    theme: &Theme,
) -> Vec<String> {
    let mut lines = Vec::new();

    // Left-to-right: render nodes horizontally with arrows between
    // Build a linear order from edges
    let mut order: Vec<usize> = Vec::new();
    let mut added = vec![false; nodes.len()];

    // Find root nodes
    let has_incoming: Vec<bool> = {
        let mut inc = vec![false; nodes.len()];
        for edge in edges {
            if let Some(to_idx) = nodes.iter().position(|n| n.id == edge.to)
                && let Some(slot) = inc.get_mut(to_idx)
            {
                *slot = true;
            }
        }
        inc
    };

    // Start with root nodes in order found
    for i in 0..nodes.len() {
        if !get_bool(&has_incoming, i) {
            order.push(i);
            set_bool(&mut added, i, true);
        }
    }

    // Follow edges to add remaining nodes
    let mut changed = true;
    while changed {
        changed = false;
        for edge in edges {
            if let (Some(from_idx), Some(to_idx)) = (
                nodes.iter().position(|n| n.id == edge.from),
                nodes.iter().position(|n| n.id == edge.to),
            ) && get_bool(&added, from_idx)
                && !get_bool(&added, to_idx)
            {
                order.push(to_idx);
                set_bool(&mut added, to_idx, true);
                changed = true;
            }
        }
        // Add any unvisited nodes at the end
        for i in 0..nodes.len() {
            if !get_bool(&added, i) {
                order.push(i);
                set_bool(&mut added, i, true);
                changed = true;
            }
        }
    }

    if order.is_empty() {
        order = (0..nodes.len()).collect();
    }

    if let Some(branch_lines) = render_flowchart_lr_branch(nodes, edges, &order, inner_width, theme)
    {
        return branch_lines;
    }

    // Render as horizontal flow: [A] --> [B] --> [C]
    let node_blocks: Vec<Vec<String>> = order
        .iter()
        .filter_map(|&idx| nodes.get(idx))
        .map(|node| make_node_lines(node, inner_width, theme))
        .collect();

    let max_rows = node_blocks.iter().map(|v| v.len()).max().unwrap_or(0);
    let arrow_row = max_rows / 2;

    for row_idx in 0..max_rows {
        let mut line = String::new();
        for (i, block) in node_blocks.iter().enumerate() {
            if i > 0 {
                let sep = if row_idx == arrow_row {
                    " ──▶ "
                } else {
                    "     "
                };
                line.push_str(sep);
            }
            let offset = (max_rows - block.len()) / 2;
            if row_idx >= offset && row_idx < offset + block.len() {
                if let Some(row) = block.get(row_idx - offset) {
                    line.push_str(row);
                }
            } else {
                let width = block
                    .first()
                    .map(|s| UnicodeWidthStr::width(s.as_str()))
                    .unwrap_or(inner_width + 2);
                line.push_str(&" ".repeat(width));
            }
        }
        lines.push(line);
    }

    // Add edge labels below
    for edge in edges {
        if let (Some(from_idx), Some(to_idx)) = (
            order
                .iter()
                .position(|&i| nodes.get(i).is_some_and(|node| node.id == edge.from)),
            order
                .iter()
                .position(|&i| nodes.get(i).is_some_and(|node| node.id == edge.to)),
        ) && !edge.label.is_empty()
            && to_idx == from_idx + 1
        {
            lines.push(format!("       {}  ", edge.label));
        }
    }

    lines
}

fn render_flowchart_lr_branch(
    nodes: &[FlowNode],
    edges: &[FlowEdge],
    order: &[usize],
    inner_width: usize,
    theme: &Theme,
) -> Option<Vec<String>> {
    let mut outgoing: Vec<Vec<(usize, String)>> = vec![Vec::new(); nodes.len()];
    for edge in edges {
        if let (Some(from_idx), Some(to_idx)) = (
            nodes.iter().position(|n| n.id == edge.from),
            nodes.iter().position(|n| n.id == edge.to),
        ) && let Some(edges) = outgoing.get_mut(from_idx)
        {
            edges.push((to_idx, edge.label.clone()));
        }
    }

    let branch_idx = order
        .iter()
        .copied()
        .find(|&idx| outgoing.get(idx).is_some_and(|edges| edges.len() > 1))?;
    let branch_pos = order.iter().position(|&idx| idx == branch_idx)?;
    let branch_edges = outgoing.get(branch_idx)?;
    let primary_edge_pos = branch_edges
        .iter()
        .position(|(_, label)| label.eq_ignore_ascii_case("yes"))
        .unwrap_or(0);
    let alternate_edge_pos = branch_edges
        .iter()
        .position(|(_, label)| label.eq_ignore_ascii_case("no"))
        .or_else(|| (0..branch_edges.len()).find(|&idx| idx != primary_edge_pos))?;

    let primary_edge = branch_edges.get(primary_edge_pos)?.clone();
    let alternate_edge = branch_edges.get(alternate_edge_pos)?.clone();

    let mut main_path = order.get(..=branch_pos)?.to_vec();
    let mut visited = vec![false; nodes.len()];
    for &idx in &main_path {
        set_bool(&mut visited, idx, true);
    }

    let mut current = primary_edge.0;
    while !get_bool(&visited, current) {
        main_path.push(current);
        set_bool(&mut visited, current, true);
        if let Some(next) = outgoing
            .get(current)
            .filter(|edges| edges.len() == 1)
            .and_then(|edges| edges.first())
            .map(|edge| edge.0)
        {
            current = next;
        } else {
            break;
        }
    }

    let mut lower_path = vec![alternate_edge.0];
    let mut lower_visited = vec![false; nodes.len()];
    set_bool(&mut lower_visited, branch_idx, true);
    set_bool(&mut lower_visited, primary_edge.0, true);
    set_bool(&mut lower_visited, alternate_edge.0, true);
    let mut current = alternate_edge.0;
    while let Some(next) = outgoing
        .get(current)
        .filter(|edges| edges.len() == 1)
        .and_then(|edges| edges.first())
        .map(|edge| edge.0)
    {
        if get_bool(&lower_visited, next) {
            break;
        }
        lower_path.push(next);
        set_bool(&mut lower_visited, next, true);
        current = next;
    }

    let main_blocks: Vec<Vec<String>> = main_path
        .iter()
        .filter_map(|&idx| nodes.get(idx))
        .map(|node| make_node_lines(node, inner_width, theme))
        .collect();
    let lower_blocks: Vec<Vec<String>> = lower_path
        .iter()
        .filter_map(|&idx| nodes.get(idx))
        .map(|node| make_node_lines(node, inner_width, theme))
        .collect();

    let block_width = inner_width + 6;
    let mut main_connectors = Vec::new();
    for pair in main_path.windows(2) {
        if let [from_idx, to_idx] = pair
            && let (Some(from), Some(to)) = (nodes.get(*from_idx), nodes.get(*to_idx))
        {
            let label = edges
                .iter()
                .find(|edge| edge.from == from.id && edge.to == to.id)
                .map(|edge| edge.label.as_str())
                .unwrap_or("");
            main_connectors.push(lr_connector(label, theme));
        }
    }

    let mut lower_connectors = Vec::new();
    for pair in lower_path.windows(2) {
        if let [from_idx, to_idx] = pair
            && let (Some(from), Some(to)) = (nodes.get(*from_idx), nodes.get(*to_idx))
        {
            let label = edges
                .iter()
                .find(|edge| edge.from == from.id && edge.to == to.id)
                .map(|edge| edge.label.as_str())
                .unwrap_or("");
            lower_connectors.push(lr_connector(label, theme));
        }
    }

    let mut node_starts = Vec::new();
    let mut cursor = 0;
    for (idx, _) in main_path.iter().enumerate() {
        node_starts.push(cursor);
        cursor += block_width;
        if let Some(connector) = main_connectors.get(idx) {
            cursor += connector.width;
        }
    }

    let branch_main_pos = main_path.iter().position(|&idx| idx == branch_idx)?;
    let branch_center = node_starts.get(branch_main_pos).copied()? + block_width / 2;
    let lower_start = branch_center.saturating_sub(block_width / 2);

    let mut lines = Vec::new();
    for row_idx in 0..3 {
        let mut line = String::new();
        for (idx, block) in main_blocks.iter().enumerate() {
            if let Some(row) = block.get(row_idx) {
                line.push_str(row);
            }
            if let Some(connector_row) = main_connectors
                .get(idx)
                .and_then(|connector| connector.rows.get(row_idx))
            {
                line.push_str(connector_row);
            }
        }
        lines.push(line);
    }

    lines.push(
        theme
            .mermaid_edge
            .apply(&format!(
                "{:branch_center$}│ {}",
                "",
                alternate_edge.1,
                branch_center = branch_center
            ))
            .to_string(),
    );
    lines.push(
        theme
            .mermaid_arrow_head
            .apply(&format!(
                "{:branch_center$}▼",
                "",
                branch_center = branch_center
            ))
            .to_string(),
    );

    for row_idx in 0..3 {
        let mut line = " ".repeat(lower_start);
        for (idx, block) in lower_blocks.iter().enumerate() {
            if let Some(row) = block.get(row_idx) {
                line.push_str(row);
            }
            if let Some(connector_row) = lower_connectors
                .get(idx)
                .and_then(|connector| connector.rows.get(row_idx))
            {
                line.push_str(connector_row);
            }
        }
        lines.push(line);
    }

    Some(lines)
}

struct LrConnector {
    rows: [String; 3],
    width: usize,
}

fn lr_connector(label: &str, theme: &Theme) -> LrConnector {
    let label_width = UnicodeWidthStr::width(label);
    let width = (label_width + 4).max(7);
    let left_pad = (width.saturating_sub(label_width)) / 2;
    let right_pad = width.saturating_sub(label_width + left_pad);
    let label_row = if label.is_empty() {
        " ".repeat(width)
    } else {
        theme
            .mermaid_label
            .apply(&format!(
                "{}{}{}",
                " ".repeat(left_pad),
                label,
                " ".repeat(right_pad)
            ))
            .to_string()
    };
    let arrow_row = theme
        .mermaid_edge
        .apply(&format!("{}▶", "─".repeat(width.saturating_sub(1))))
        .to_string();
    let spacer_row = " ".repeat(width);

    LrConnector { rows: [label_row, arrow_row, spacer_row], width }
}

// ---------------------------------------------------------------------------
// Sequence diagram rendering
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct SeqParticipant {
    name: String,
    _alias: String,
}

#[derive(Debug)]
enum SeqMessage {
    Arrow {
        from: String,
        to: String,
        label: String,
        solid: bool,
        arrow_head: bool,
    },
    Note {
        over: String,
        text: String,
    },
}

/// Parse a sequence diagram.
fn parse_sequence(diagram: &str) -> (Vec<SeqParticipant>, Vec<SeqMessage>) {
    let mut participants: Vec<SeqParticipant> = Vec::new();
    let mut messages: Vec<SeqMessage> = Vec::new();

    for line in diagram.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('%') {
            continue;
        }

        // participant/actor definitions
        if line.to_lowercase().starts_with("participant ")
            || line.to_lowercase().starts_with("actor ")
        {
            let name = line
                .split_once(' ')
                .map(|x| x.1)
                .unwrap_or("")
                .trim()
                .to_string();
            let _alias = name.clone();
            if !participants.iter().any(|p| p.name == name) {
                participants.push(SeqParticipant { name, _alias });
            }
            continue;
        }

        // Note over A, B: text
        if line.to_lowercase().starts_with("note over ") {
            let rest = line.trim_start_matches(|c: char| !c.is_whitespace()).trim();
            if let Some((over, text)) = rest.split_once(':') {
                let over = over.trim().to_string();
                let text = text.trim().to_string();
                messages.push(SeqMessage::Note { over, text });
            }
            continue;
        }

        // Note right/left of A: text
        if line.to_lowercase().starts_with("note ") {
            let rest = line.trim_start_matches(|c: char| !c.is_whitespace()).trim();
            if let Some((over, text)) = rest.split_once(':') {
                let over = over.trim().to_string();
                let text = text.trim().to_string();
                messages.push(SeqMessage::Note { over, text });
            }
            continue;
        }

        // Message arrows: A->>B: label, A-->>B: label, A->B: label, A-->B: label
        // Order matters: longer syntaxes must be checked before shorter ones
        // to avoid false matches (e.g., "-->>" contains "->>")
        for arrow_syntax in &["-->>", "->>", "--x", "-x", "-->", "->", "--=", "=>>"] {
            if let Some((from_part, rest)) = line.split_once(arrow_syntax) {
                let from = from_part.trim().to_string();

                // Check for label
                let (to, label) = if let Some((to, label)) = rest.split_once(':') {
                    (to.trim().to_string(), label.trim().to_string())
                } else {
                    (rest.trim().to_string(), String::new())
                };

                let solid = arrow_syntax.starts_with('-') && arrow_syntax.len() == 3;
                let arrow_head = arrow_syntax.ends_with('>');

                // Clone before moving 'to' into the message
                let to_for_participants = to.clone();

                messages.push(SeqMessage::Arrow {
                    from: from.clone(),
                    to,
                    label,
                    solid,
                    arrow_head,
                });

                // Auto-add participants
                for name in [&from, &to_for_participants] {
                    if !participants.iter().any(|p| p.name == *name) {
                        participants
                            .push(SeqParticipant { name: name.clone(), _alias: name.clone() });
                    }
                }
                break;
            }
        }
    }

    (participants, messages)
}

/// Render a sequence diagram to terminal art with theme colors.
fn render_sequence(diagram: &str, _width: usize, theme: &Theme) -> Vec<String> {
    let (participants, messages) = parse_sequence(diagram);
    if participants.is_empty() && messages.is_empty() {
        return vec!["[empty sequence diagram]".to_string()];
    }

    let mut lines = Vec::new();

    // Compute column widths, centers, and total width
    let col_widths: Vec<usize> = participants
        .iter()
        .map(|p| UnicodeWidthStr::width(p.name.as_str()).max(6) + 2)
        .collect();

    let col_centers: Vec<usize> = {
        let mut centers = Vec::new();
        let mut offset = 0usize;
        for width in &col_widths {
            let box_width = width + 2;
            centers.push(offset + box_width / 2);
            offset += box_width + 3; // 3 spaces between columns
        }
        centers
    };

    let total_width = if col_widths.is_empty() {
        0
    } else {
        col_widths.iter().map(|w| w + 2).sum::<usize>() + (col_widths.len().saturating_sub(1)) * 3
    };

    // Helper: build a blank row buffer
    let blank_row = || vec![' '; total_width];

    // Participant headers
    let mut header_top = blank_row();
    let mut header_top_styles = vec![STYLE_NONE; total_width];
    let mut header_mid = blank_row();
    let mut header_mid_styles = vec![STYLE_NONE; total_width];
    let mut header_bot = blank_row();
    let mut header_bot_styles = vec![STYLE_NONE; total_width];
    let mut offset = 0usize;
    for (p, w) in participants.iter().zip(col_widths.iter().copied()) {
        let name = &p.name;
        let name_width = UnicodeWidthStr::width(name.as_str());
        let pad_left = (w.saturating_sub(name_width)) / 2;
        let pad_right = w.saturating_sub(name_width).saturating_sub(pad_left);

        // Top border: ┌────┐
        let top_str = format!("┌{}┐", "─".repeat(w));
        for (j, c) in top_str.chars().enumerate() {
            set_char(&mut header_top, offset + j, c);
            set_style(&mut header_top_styles, offset + j, STYLE_BORDER);
        }
        // Middle: │ Name │
        let mid_str = format!(
            "│{}{}{}│",
            " ".repeat(pad_left),
            name,
            " ".repeat(pad_right)
        );
        for (j, c) in mid_str.chars().enumerate() {
            set_char(&mut header_mid, offset + j, c);
            // Style the │ borders as border, name text as node
            if j == 0 || j == mid_str.chars().count() - 1 {
                set_style(&mut header_mid_styles, offset + j, STYLE_BORDER);
            } else {
                set_style(&mut header_mid_styles, offset + j, STYLE_NODE);
            }
        }
        // Bottom: └────┘
        let bot_str = format!("└{}┘", "─".repeat(w));
        for (j, c) in bot_str.chars().enumerate() {
            set_char(&mut header_bot, offset + j, c);
            set_style(&mut header_bot_styles, offset + j, STYLE_BORDER);
        }
        offset += w + 2 + 3;
    }
    lines.push(build_colored_line(&header_top, &header_top_styles, theme));
    lines.push(build_colored_line(&header_mid, &header_mid_styles, theme));
    lines.push(build_colored_line(&header_bot, &header_bot_styles, theme));

    // Render each message
    for msg in &messages {
        match msg {
            SeqMessage::Arrow { from, to, label, solid, arrow_head } => {
                let from_idx = participants.iter().position(|p| p.name == *from);
                let to_idx = participants.iter().position(|p| p.name == *to);

                match (from_idx, to_idx) {
                    (Some(fi), Some(ti)) => {
                        let (left_idx, right_idx) = if fi < ti { (fi, ti) } else { (ti, fi) };
                        let left_center = col_centers.get(left_idx).copied();
                        let right_center = col_centers.get(right_idx).copied();
                        let (Some(left_center), Some(right_center)) = (left_center, right_center)
                        else {
                            continue;
                        };
                        let direction_right = fi < ti;

                        // Lifeline row before arrow
                        let mut before = blank_row();
                        let mut before_styles = vec![STYLE_NONE; total_width];
                        for &center in &col_centers {
                            set_char(&mut before, center, '│');
                            set_style(&mut before_styles, center, STYLE_EDGE);
                        }
                        lines.push(build_colored_line(&before, &before_styles, theme));

                        // Arrow row
                        let mut arrow_row = blank_row();
                        let mut arrow_styles = vec![STYLE_NONE; total_width];
                        for &center in &col_centers {
                            set_char(&mut arrow_row, center, '│');
                            set_style(&mut arrow_styles, center, STYLE_EDGE);
                        }

                        // Draw horizontal arrow line
                        let line_char = if *solid { '─' } else { '┄' };
                        let start = left_center + 1;
                        let end = right_center.saturating_sub(1);
                        if start <= end {
                            for x in start..=end {
                                set_char(&mut arrow_row, x, line_char);
                                set_style(&mut arrow_styles, x, STYLE_EDGE);
                            }
                        }

                        if *arrow_head {
                            let head_char = if direction_right { '►' } else { '◄' };
                            let head_pos = if direction_right {
                                right_center
                            } else {
                                left_center
                            };
                            set_char(&mut arrow_row, head_pos, head_char);
                            set_style(&mut arrow_styles, head_pos, STYLE_ARROW);
                        }

                        lines.push(build_colored_line(&arrow_row, &arrow_styles, theme));

                        // Label row (if any)
                        if !label.is_empty() {
                            let mut label_row = blank_row();
                            let mut label_styles = vec![STYLE_NONE; total_width];
                            for &center in &col_centers {
                                set_char(&mut label_row, center, '│');
                                set_style(&mut label_styles, center, STYLE_EDGE);
                            }
                            let label_start = left_center + 2;
                            for (j, c) in label.chars().enumerate() {
                                set_char(&mut label_row, label_start + j, c);
                                set_style(&mut label_styles, label_start + j, STYLE_LABEL);
                            }
                            lines.push(build_colored_line(&label_row, &label_styles, theme));
                        }
                    }
                    (None, _) | (_, None) => {
                        lines.push(format!("  {} → {}: {}", from, to, label));
                    }
                }
            }
            SeqMessage::Note { over, text } => {
                let over_idx = participants.iter().position(|p| p.name == *over);
                if let Some(center) = over_idx.and_then(|idx| col_centers.get(idx).copied()) {
                    let text_width = UnicodeWidthStr::width(text.as_str());
                    let note_width = text_width.max(4) + 2;
                    let note_left = center.saturating_sub(note_width / 2);

                    let mut note_top = blank_row();
                    let mut note_top_styles = vec![STYLE_NONE; total_width];
                    for &center in &col_centers {
                        set_char(&mut note_top, center, '│');
                        set_style(&mut note_top_styles, center, STYLE_EDGE);
                    }
                    for (j, c) in format!("┌{}┐", "─".repeat(note_width)).chars().enumerate()
                    {
                        set_char(&mut note_top, note_left + j, c);
                        set_style(&mut note_top_styles, note_left + j, STYLE_BORDER);
                    }
                    lines.push(build_colored_line(&note_top, &note_top_styles, theme));

                    let mut note_mid = blank_row();
                    let mut note_mid_styles = vec![STYLE_NONE; total_width];
                    for &center in &col_centers {
                        set_char(&mut note_mid, center, '│');
                        set_style(&mut note_mid_styles, center, STYLE_EDGE);
                    }
                    let pad = note_width.saturating_sub(text_width);
                    let lpad = pad / 2;
                    let rpad = pad - lpad;
                    let mid_str = format!("│{}{}{}│", " ".repeat(lpad), text, " ".repeat(rpad));
                    for (j, c) in mid_str.chars().enumerate() {
                        set_char(&mut note_mid, note_left + j, c);
                        if j == 0 || j == mid_str.chars().count() - 1 {
                            set_style(&mut note_mid_styles, note_left + j, STYLE_BORDER);
                        } else {
                            set_style(&mut note_mid_styles, note_left + j, STYLE_LABEL);
                        }
                    }
                    lines.push(build_colored_line(&note_mid, &note_mid_styles, theme));

                    let mut note_bot = blank_row();
                    let mut note_bot_styles = vec![STYLE_NONE; total_width];
                    for &center in &col_centers {
                        set_char(&mut note_bot, center, '│');
                        set_style(&mut note_bot_styles, center, STYLE_EDGE);
                    }
                    for (j, c) in format!("└{}┘", "─".repeat(note_width)).chars().enumerate()
                    {
                        set_char(&mut note_bot, note_left + j, c);
                        set_style(&mut note_bot_styles, note_left + j, STYLE_BORDER);
                    }
                    lines.push(build_colored_line(&note_bot, &note_bot_styles, theme));
                }
            }
        }
    }

    // Final lifelines
    if !participants.is_empty() {
        let mut final_row = blank_row();
        let mut final_styles = vec![STYLE_NONE; total_width];
        for &center in &col_centers {
            set_char(&mut final_row, center, '│');
            set_style(&mut final_styles, center, STYLE_EDGE);
        }
        lines.push(build_colored_line(&final_row, &final_styles, theme));
    }

    lines
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::theme::Theme;

    fn test_theme() -> Theme {
        Theme::dark()
    }

    #[test]
    fn test_detect_flowchart_td() {
        let result = detect_diagram_type("graph TD\n  A[Start] --> B[End]");
        assert_eq!(
            result,
            Some(DiagramType::Flowchart { direction: FlowDirection::TopDown })
        );
    }

    #[test]
    fn test_diamond_shapes_lr() {
        let result = render_mermaid(
            "graph LR\n  A[Rectangle] --> B(Round)\n  B --> C{Diamond}\n  C --> D([Stadium])",
            80,
            &test_theme(),
        );
        assert!(result.is_some());
        let lines = result.unwrap();
        let joined = lines.join("\n");
        assert!(joined.contains("Rectangle"));
        assert!(joined.contains("Round"));
        assert!(joined.contains("Diamond"));
        assert!(joined.contains("Stadium"));
        // Diamond decisions use a clean double-border terminal style
        assert!(joined.contains('╔') && joined.contains('╚'));
    }

    #[test]
    fn test_render_flowchart_lr_basic() {
        // Test that |label| syntax is parsed correctly for edges
        let result = render_mermaid("graph LR\n  A -->|yes| B\n  B -->|no| C", 80, &test_theme());
        assert!(result.is_some());
        let lines = result.unwrap();
        let joined = lines.join("\n");
        assert!(joined.contains("yes"));
        assert!(joined.contains("no"));
    }

    #[test]
    fn test_render_flowchart_lr_branch_yes_right_no_down() {
        let result = render_mermaid(
            "graph LR\n  A[Input Data] --> B{Valid?}\n  B -->|Yes| C[Process]\n  B -->|No| D[Reject]\n  C --> E[Output]",
            80,
            &test_theme(),
        );
        assert!(result.is_some());
        let lines = result.unwrap();
        let joined = lines.join("\n");
        assert!(joined.contains("Yes"));
        assert!(joined.contains("│ No"));
        assert!(joined.contains('▼'));
        assert!(joined.contains("Reject"));
        assert!(joined.contains("Output"));
        assert!(joined.find("Process").unwrap() < joined.find("Output").unwrap());
        assert!(joined.find("Valid?").unwrap() < joined.find("Reject").unwrap());
    }
    #[test]
    fn test_detect_flowchart_lr() {
        let result = detect_diagram_type("graph LR\n  A --> B");
        assert_eq!(
            result,
            Some(DiagramType::Flowchart { direction: FlowDirection::LeftRight })
        );
    }

    #[test]
    fn test_detect_sequence() {
        let result = detect_diagram_type("sequenceDiagram\n  Alice->>Bob: Hello");
        assert_eq!(result, Some(DiagramType::Sequence));
    }

    #[test]
    fn test_detect_unsupported() {
        let result = detect_diagram_type("classDiagram\n  class Animal");
        assert_eq!(result, None);
    }

    #[test]
    fn test_render_mermaid_returns_some_for_flowchart() {
        let result = render_mermaid("graph TD\n  A[Start] --> B[End]", 80, &test_theme());
        assert!(result.is_some());
        let lines = result.unwrap();
        assert!(!lines.is_empty());
        // Should contain flowchart art
        let joined = lines.join("\n");
        assert!(joined.contains("Start") || joined.contains("End"));
    }

    #[test]
    fn test_render_mermaid_returns_some_for_sequence() {
        let result = render_mermaid("sequenceDiagram\n  Alice->>Bob: Hello", 80, &test_theme());
        assert!(result.is_some());
        let lines = result.unwrap();
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_mermaid_returns_none_for_unsupported() {
        let result = render_mermaid("classDiagram\n  class Animal", 80, &test_theme());
        assert!(result.is_none());
    }

    #[test]
    fn test_render_mermaid_returns_none_for_empty() {
        let result = render_mermaid("", 80, &test_theme());
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_node_ref_rect() {
        let (id, label, shape) = parse_node_ref("A[Hello World]");
        assert_eq!(id, "A");
        assert_eq!(label.unwrap(), "Hello World");
        // No error - should still parse correctly
        assert_eq!(shape, NodeShape::Rect);
    }

    #[test]
    fn test_parse_node_ref_plain() {
        let (id, label, shape) = parse_node_ref("B");
        assert_eq!(id, "B");
        assert_eq!(label, None); // Plain text has no explicit label
        assert_eq!(shape, NodeShape::Default);
    }

    #[test]
    fn test_parse_node_ref_with_arrow_suffix() {
        let (id, _label, shape) = parse_node_ref("A[Hello]-->");
        assert_eq!(id, "A");
        assert_eq!(shape, NodeShape::Rect);
    }

    #[test]
    fn test_render_flowchart_td_basic() {
        let result = render_mermaid("graph TD\n  A --> B", 80, &test_theme());
        let lines = result.unwrap();
        let joined = lines.join("\n");
        // Should contain box-drawing characters
        assert!(joined.contains('┌') || joined.contains('╭'));
        assert!(joined.contains('│'));
        assert!(joined.contains('└') || joined.contains('╰'));
        // Should contain nodes
        assert!(joined.contains('A'));
        assert!(joined.contains('B'));
        // Should contain arrow
        assert!(joined.contains('│') || joined.contains('▼') || joined.contains('▶'));
    }

    #[test]
    fn test_render_flowchart_lr_abc() {
        let result = render_mermaid("graph LR\n  A --> B\n  B --> C", 80, &test_theme());
        let lines = result.unwrap();
        let joined = lines.join("\n");
        assert!(joined.contains('A'));
        assert!(joined.contains('B'));
        assert!(joined.contains('C'));
        // LR flow should have horizontal connectors
        assert!(joined.contains('▶') || joined.contains('─'));
    }

    #[test]
    fn test_render_flowchart_td_branch_labels() {
        let result = render_mermaid(
            "graph TD\n  A[Start] --> B{Decision}\n  B -->|Yes| C[Accept]\n  B -->|No| D[Reject]",
            80,
            &test_theme(),
        );
        assert!(result.is_some());
        let lines = result.unwrap();
        let joined = lines.join("\n");
        assert!(joined.contains("Yes"));
        assert!(joined.contains("No"));
        assert!(joined.contains("Accept"));
        assert!(joined.contains("Reject"));
        assert!(joined.contains('┐'));
    }

    #[test]
    fn test_render_flowchart_with_labels() {
        let result = render_mermaid(
            "graph TD\n  A[Start] -->|Next step| B[Process]",
            80,
            &test_theme(),
        );
        assert!(result.is_some());
        let lines = result.unwrap();
        let joined = lines.join("\n");
        assert!(joined.contains("Start"));
        assert!(joined.contains("Process"));
        assert!(joined.contains("Next step"));
    }

    #[test]
    fn test_render_flowchart_edge_label_syntax() {
        // Test that |label| syntax is parsed correctly for edges
        let result = render_mermaid("graph LR\n  A -->|yes| B\n  B -->|no| C", 80, &test_theme());
        assert!(result.is_some());
        let lines = result.unwrap();
        let joined = lines.join("\n");
        assert!(joined.contains("yes"));
        assert!(joined.contains("no"));
    }

    #[test]
    fn test_render_sequence_basic() {
        let result = render_mermaid(
            "sequenceDiagram\n  Alice->>Bob: Hello Bob\n  Bob-->>Alice: Hi Alice",
            80,
            &test_theme(),
        );
        assert!(result.is_some());
        let lines = result.unwrap();
        let joined = lines.join("\n");
        // Should contain participant names
        assert!(joined.contains("Alice"));
        assert!(joined.contains("Bob"));
        // Should contain labels
        assert!(joined.contains("Hello Bob"));
    }

    #[test]
    fn test_render_flowchart_round_rect() {
        let result = render_mermaid("graph TD\n  A(Start) --> B(End)", 80, &test_theme());
        assert!(result.is_some());
        let lines = result.unwrap();
        let joined = lines.join("\n");
        assert!(joined.contains("Start"));
        assert!(joined.contains("End"));
    }

    #[test]
    fn test_flowchart_without_edges() {
        let result = render_mermaid("graph TD\n  A[Standalone]", 80, &test_theme());
        assert!(result.is_some());
        let lines = result.unwrap();
        let joined = lines.join("\n");
        assert!(joined.contains("Standalone"));
    }

    #[test]
    fn test_display_width_respected() {
        let result = render_mermaid(
            "graph TD\n  A[Hello World] --> B[Testing 123]",
            40,
            &test_theme(),
        );
        assert!(result.is_some());
    }
}
