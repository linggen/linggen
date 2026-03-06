/// Lightweight inline markdown to ratatui conversion.
///
/// Handles: headers, bold, italic, inline code, code fences, lists.
/// Does not attempt full CommonMark — just the most common constructs.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub fn markdown_to_lines(input: &str, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;
    // Box width: terminal width minus 2-char left margin, minus 1 for safety
    let box_inner = (width as usize).saturating_sub(4); // "  ┌" + "┐" = 4 chars of framing

    for raw_line in input.lines() {
        if raw_line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            if in_code_block {
                // Opening fence: ┌───────┐
                lines.push(Line::from(Span::styled(
                    format!("  ┌{}┐", "─".repeat(box_inner)),
                    Style::default().fg(Color::DarkGray),
                )));
            } else {
                // Closing fence: └───────┘
                lines.push(Line::from(Span::styled(
                    format!("  └{}┘", "─".repeat(box_inner)),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            continue;
        }

        if in_code_block {
            // Pad content to fill box width: │ content     │
            let content = raw_line;
            let content_display_len = content.chars().count();
            // Inner space: box_inner minus "│ " prefix (2) minus "│" suffix (0, it's outside)
            // Actually: "  │ {content padded} │" → left margin(2) + │(1) + space(1) + content + pad + space(0) + │(1)
            let pad = box_inner.saturating_sub(content_display_len + 2); // 2 = "│ " prefix inside box
            lines.push(Line::from(vec![
                Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    raw_line.to_string(),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    format!("{}│", " ".repeat(pad)),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            continue;
        }

        let trimmed = raw_line.trim();

        // Horizontal rules
        if (trimmed == "---" || trimmed == "***" || trimmed == "___")
            || (trimmed.len() >= 3
                && trimmed
                    .chars()
                    .all(|c| c == '-' || c == '*' || c == '_' || c == ' ')
                && trimmed.chars().filter(|c| !c.is_whitespace()).count() >= 3
                && {
                    let first_non_ws = trimmed.chars().find(|c| !c.is_whitespace()).unwrap();
                    trimmed
                        .chars()
                        .all(|c| c == first_non_ws || c.is_whitespace())
                })
        {
            lines.push(Line::from(Span::styled(
                "  ────────────────────────────────────────",
                Style::default().fg(Color::DarkGray),
            )));
            continue;
        }

        // Headers — check longest prefix first
        if trimmed.starts_with("#### ") {
            let rest = &trimmed[trimmed.find(' ').unwrap() + 1..];
            lines.push(Line::from(Span::styled(
                format!("   {rest}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("### ") {
            lines.push(Line::from(Span::styled(
                format!("   {rest}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            lines.push(Line::from(Span::styled(
                format!("  {rest}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            lines.push(Line::from(Span::styled(
                rest.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            continue;
        }

        // Blockquotes
        if let Some(rest) = trimmed.strip_prefix("> ") {
            let mut spans = vec![Span::styled(
                "  │ ",
                Style::default().fg(Color::DarkGray),
            )];
            spans.extend(parse_inline_markdown(rest));
            lines.push(Line::from(spans));
            continue;
        }
        if trimmed == ">" {
            lines.push(Line::from(Span::styled(
                "  │",
                Style::default().fg(Color::DarkGray),
            )));
            continue;
        }

        // Bullet lists
        if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            let mut spans = vec![Span::styled(
                "  · ",
                Style::default().fg(Color::DarkGray),
            )];
            spans.extend(parse_inline_markdown(rest));
            lines.push(Line::from(spans));
            continue;
        }

        // Numbered lists
        if let Some(dot_pos) = trimmed.find(". ") {
            let prefix = &trimmed[..dot_pos];
            if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
                let rest = &trimmed[dot_pos + 2..];
                let mut spans = vec![Span::styled(
                    format!("  {}. ", prefix),
                    Style::default().fg(Color::DarkGray),
                )];
                spans.extend(parse_inline_markdown(rest));
                lines.push(Line::from(spans));
                continue;
            }
        }

        // Empty line
        if trimmed.is_empty() {
            lines.push(Line::from(""));
            continue;
        }

        // Regular paragraph line — parse inline markdown
        let spans = parse_inline_markdown(trimmed);
        let mut full_spans = vec![Span::raw("  ".to_string())];
        full_spans.extend(spans);
        lines.push(Line::from(full_spans));
    }

    lines
}

/// Parse inline markdown: **bold**, *italic*, `code`, and plain text.
fn parse_inline_markdown(input: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut chars = input.chars().peekable();
    let mut buf = String::new();

    while let Some(ch) = chars.next() {
        match ch {
            '`' => {
                // Flush plain buffer
                if !buf.is_empty() {
                    spans.push(Span::raw(std::mem::take(&mut buf)));
                }
                // Collect until closing backtick
                let mut code = String::new();
                for c in chars.by_ref() {
                    if c == '`' {
                        break;
                    }
                    code.push(c);
                }
                spans.push(Span::styled(
                    code,
                    Style::default().fg(Color::Yellow),
                ));
            }
            '[' => {
                // Try to parse a markdown link [text](url)
                let mut link_text = String::new();
                let mut found_link = false;
                for c in chars.by_ref() {
                    if c == ']' {
                        if chars.peek() == Some(&'(') {
                            chars.next(); // consume '('
                            for c2 in chars.by_ref() {
                                if c2 == ')' {
                                    break;
                                }
                            }
                            found_link = true;
                        }
                        break;
                    }
                    link_text.push(c);
                }
                if found_link {
                    if !buf.is_empty() {
                        spans.push(Span::raw(std::mem::take(&mut buf)));
                    }
                    spans.push(Span::styled(
                        link_text,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::UNDERLINED),
                    ));
                } else {
                    buf.push('[');
                    buf.push_str(&link_text);
                    buf.push(']');
                }
            }
            '*' => {
                // Check for bold (**) vs italic (*)
                if chars.peek() == Some(&'*') {
                    chars.next(); // consume second *
                    if !buf.is_empty() {
                        spans.push(Span::raw(std::mem::take(&mut buf)));
                    }
                    let mut bold = String::new();
                    loop {
                        match chars.next() {
                            Some('*') if chars.peek() == Some(&'*') => {
                                chars.next();
                                break;
                            }
                            Some(c) => bold.push(c),
                            None => break,
                        }
                    }
                    spans.push(Span::styled(
                        bold,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ));
                } else {
                    if !buf.is_empty() {
                        spans.push(Span::raw(std::mem::take(&mut buf)));
                    }
                    let mut italic = String::new();
                    for c in chars.by_ref() {
                        if c == '*' {
                            break;
                        }
                        italic.push(c);
                    }
                    spans.push(Span::styled(
                        italic,
                        Style::default().add_modifier(Modifier::ITALIC),
                    ));
                }
            }
            _ => {
                buf.push(ch);
            }
        }
    }

    if !buf.is_empty() {
        spans.push(Span::raw(buf));
    }

    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }

    spans
}
