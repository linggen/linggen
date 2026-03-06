/// Rendering logic: converts DisplayBlock variants into Vec<Line<'static>>.
///
/// Visual style follows Claude Code conventions:
/// - Tool steps: ⏺ bullet with ⎿ Done result lines
/// - Subagent tree: box-drawing characters (├─, └─, │)
/// - Agent messages: markdown-rendered with [agent] prefix
/// - Plan blocks: checkbox indicators [x] [~] [ ]

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::display::*;
use super::markdown;

/// Render a single DisplayBlock into terminal lines.
pub fn render_block(block: &DisplayBlock, width: u16) -> Vec<Line<'static>> {
    match block {
        DisplayBlock::UserMessage { text, image_count } => render_user_message(text, *image_count),
        DisplayBlock::AgentMessage { agent_id, text } => render_agent_message(agent_id, text, width),
        DisplayBlock::SystemMessage { text } => render_system_message(text),
        DisplayBlock::ToolGroup {
            steps,
            collapsed,
            estimated_tokens,
            duration_secs,
            ..
        } => render_tool_group(steps, *collapsed, *estimated_tokens, *duration_secs),
        DisplayBlock::SubagentGroup { entries, collapsed } => render_subagent_group(entries, *collapsed),
        DisplayBlock::PlanBlock {
            summary,
            status,
        } => render_plan_block(summary, status),
        DisplayBlock::TurnSummary {
            tool_count,
            estimated_tokens,
            duration_secs,
        } => render_turn_summary(*tool_count, *estimated_tokens, *duration_secs),
    }
}

fn render_user_message(text: &str, image_count: usize) -> Vec<Line<'static>> {
    let mut spans = vec![
        Span::styled(
            "> ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(text.to_string(), Style::default().fg(Color::White)),
    ];
    if image_count > 0 {
        spans.push(Span::styled(
            format!(
                " [{} image{}]",
                image_count,
                plural(image_count)
            ),
            Style::default().fg(Color::Magenta),
        ));
    }
    vec![Line::from(spans)]
}

fn render_agent_message(agent_id: &str, text: &str, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let md_lines = markdown::markdown_to_lines(text, width);

    if md_lines.is_empty() {
        return lines;
    }

    // Claude Code-style: ● bullet prefix with agent name
    let bullet = Span::styled(
        "● ",
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );

    if let Some(first) = md_lines.first() {
        let mut first_spans = vec![bullet];
        first_spans.extend(first.spans.iter().cloned());
        // Append dim agent tag at end of first line
        first_spans.push(Span::styled(
            format!("  [{agent_id}]"),
            Style::default().fg(Color::DarkGray),
        ));
        lines.push(Line::from(first_spans));
    }

    // Remaining lines get indentation to align with first line's content (after "● ")
    let indent = "  ";
    for md_line in md_lines.iter().skip(1) {
        let mut spans = vec![Span::raw(indent.to_string())];
        spans.extend(md_line.spans.iter().cloned());
        lines.push(Line::from(spans));
    }

    lines
}

fn render_system_message(text: &str) -> Vec<Line<'static>> {
    vec![Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(Color::Yellow),
    ))]
}

fn render_turn_summary(
    tool_count: usize,
    estimated_tokens: Option<usize>,
    duration_secs: Option<u64>,
) -> Vec<Line<'static>> {
    let mut parts = Vec::new();
    if tool_count > 0 {
        parts.push(format!(
            "{} tool use{}",
            tool_count,
            plural(tool_count)
        ));
    }
    if let Some(tokens) = estimated_tokens {
        parts.push(format!("{} tokens", format_compact_tokens(tokens)));
    }
    if let Some(secs) = duration_secs {
        parts.push(format!("{}s", secs));
    }
    let summary = if parts.is_empty() {
        "Done".to_string()
    } else {
        format!("Done ({})", parts.join(" · "))
    };
    vec![Line::from(vec![
        Span::styled("  ⎿  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            summary,
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        ),
    ])]
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Format a token count as a compact string (e.g. "30.2k").
fn format_compact_tokens(tokens: usize) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}m", tokens as f64 / 1_000_000.0)
    } else if tokens >= 10_000 {
        format!("{}k", tokens / 1000)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        format!("{}", tokens)
    }
}

/// Render a tool group with ⏺ bullets and ⎿ result lines.
pub fn render_tool_group(
    steps: &[ToolStep],
    collapsed: bool,
    estimated_tokens: Option<usize>,
    duration_secs: Option<u64>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if collapsed {
        // Collapsed summary: "Done (3 tool uses · 30.2k tokens · 16s)"
        let tool_count = steps.len();
        let mut parts = vec![format!(
            "{} tool use{}",
            tool_count,
            plural(tool_count)
        )];
        if let Some(tokens) = estimated_tokens {
            parts.push(format!("{} tokens", format_compact_tokens(tokens)));
        }
        if let Some(secs) = duration_secs {
            parts.push(format!("{}s", secs));
        }
        let summary = format!("Done ({})", parts.join(" · "));
        lines.push(Line::from(vec![
            Span::styled("  ⎿  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                summary,
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
        return lines;
    }

    for step in steps {
        let bullet_color = match step.status {
            StepStatus::InProgress => Color::Yellow,
            StepStatus::Done => Color::Green,
            StepStatus::Failed => Color::Red,
        };

        let result_text = match (&step.status, &step.result_detail) {
            (StepStatus::InProgress, _) => "Running...".to_string(),
            (StepStatus::Failed, Some(detail)) => format!("Failed ({})", detail),
            (StepStatus::Failed, None) => "Failed".to_string(),
            (StepStatus::Done, Some(detail)) => format!("Done ({})", detail),
            (StepStatus::Done, None) => "Done".to_string(),
        };

        // Tool line: ⏺ ToolName(args)
        let args_display = if step.args_summary.is_empty() {
            String::new()
        } else {
            format!("({})", truncate_str(&step.args_summary, 80))
        };

        lines.push(Line::from(vec![
            Span::styled("  ⏺ ", Style::default().fg(bullet_color)),
            Span::styled(
                step.tool_name.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(args_display, Style::default().fg(Color::White)),
        ]));

        // Result line: ⎿  Done (detail)
        lines.push(Line::from(vec![
            Span::styled("    ⎿  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                result_text,
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        // Output lines (bash live output) — show last 3 lines
        if !step.output_lines.is_empty() {
            let total = step.output_lines.len();
            let start = total.saturating_sub(3);
            if start > 0 {
                lines.push(Line::from(Span::styled(
                    format!("    ... +{} more lines", start),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            for line in &step.output_lines[start..] {
                lines.push(Line::from(vec![
                    Span::styled("    ⎿ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        truncate_str(line, 120),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        }
    }

    // Summary line
    if steps.len() > 1 {
        let summary = tool_group_summary(steps);
        lines.push(Line::from(Span::styled(
            format!("  {summary}"),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    lines
}

/// Render in-progress tool steps (active group, not yet finalized).
/// Shows ALL steps live with status indicators.
pub fn render_tool_group_active(steps: &[ToolStep], verbose: bool) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if steps.is_empty() {
        return lines;
    }

    for step in steps {
        let bullet_color = match step.status {
            StepStatus::InProgress => Color::Yellow,
            StepStatus::Done => Color::Green,
            StepStatus::Failed => Color::Red,
        };

        let args_display = if step.args_summary.is_empty() {
            String::new()
        } else {
            format!("({})", truncate_str(&step.args_summary, 80))
        };

        // Tool line: ⏺ ToolName(args)
        lines.push(Line::from(vec![
            Span::styled("  ⏺ ", Style::default().fg(bullet_color)),
            Span::styled(
                step.tool_name.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(args_display, Style::default().fg(Color::White)),
        ]));

        match step.status {
            StepStatus::InProgress => {
                // Show last output line for in-progress steps (live Bash output)
                if let Some(last_line) = step.output_lines.last() {
                    lines.push(Line::from(vec![
                        Span::styled("    ⎿ ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            truncate_str(last_line, 100),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                }
            }
            StepStatus::Done | StepStatus::Failed => {
                // Result line
                let result_text = match (&step.status, &step.result_detail) {
                    (StepStatus::Failed, Some(detail)) => format!("Failed ({})", detail),
                    (StepStatus::Failed, None) => "Failed".to_string(),
                    (StepStatus::Done, Some(detail)) => format!("Done ({})", detail),
                    (StepStatus::Done, None) => "Done".to_string(),
                    _ => unreachable!(),
                };
                lines.push(Line::from(vec![
                    Span::styled("    ⎿  ", Style::default().fg(Color::DarkGray)),
                    Span::styled(result_text, Style::default().fg(Color::DarkGray)),
                ]));

                // In verbose mode, show last 3 output lines for done steps
                if verbose && !step.output_lines.is_empty() {
                    let total = step.output_lines.len();
                    let start = total.saturating_sub(3);
                    if start > 0 {
                        lines.push(Line::from(Span::styled(
                            format!("    ... +{} more lines", start),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                    for line in &step.output_lines[start..] {
                        lines.push(Line::from(vec![
                            Span::styled("    ⎿ ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                truncate_str(line, 120),
                                Style::default().fg(Color::DarkGray),
                            ),
                        ]));
                    }
                }
            }
        }
    }

    lines
}

/// Build stats string: "· 13 tool uses · 62.7k tokens"
fn subagent_stats_str(entry: &SubagentEntry) -> String {
    let mut parts = Vec::new();
    if entry.tool_count > 0 {
        parts.push(format!(
            "{} tool use{}",
            entry.tool_count,
            plural(entry.tool_count)
        ));
    }
    if let Some(tokens) = entry.estimated_tokens {
        if tokens > 0 {
            parts.push(format!("{} tokens", format_compact_tokens(tokens)));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" · {}", parts.join(" · "))
    }
}

/// Render a finalized (done) subagent group — Claude Code style per-entry blocks.
fn render_subagent_group(entries: &[SubagentEntry], collapsed: bool) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for entry in entries {
        let task_preview = truncate_str(&entry.task, 60);
        let label = if entry.agent_name.is_empty() { "Task" } else { &entry.agent_name };
        // Header: ⏺ agent_name(task_preview)
        lines.push(Line::from(vec![
            Span::styled("  ⏺ ", Style::default().fg(Color::Green)),
            Span::styled(
                label.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({})", task_preview),
                Style::default().fg(Color::White),
            ),
        ]));

        if collapsed {
            // Collapsed: ⎿  Done (stats)
            let mut parts = vec![format!(
                "{} tool use{}",
                entry.tool_count,
                plural(entry.tool_count)
            )];
            if let Some(tokens) = entry.estimated_tokens {
                if tokens > 0 {
                    parts.push(format!("{} tokens", format_compact_tokens(tokens)));
                }
            }
            lines.push(Line::from(vec![
                Span::styled("    ⎿  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("Done ({})", parts.join(" · ")),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        } else {
            // Expanded: show each tool step
            for (i, step) in entry.tool_steps.iter().enumerate() {
                let is_last = i == entry.tool_steps.len() - 1;
                let connector = if is_last { "⎿" } else { "│" };
                let bullet_color = match step.status {
                    StepStatus::Done => Color::Green,
                    StepStatus::Failed => Color::Red,
                    StepStatus::InProgress => Color::Yellow,
                };
                let args_display = if step.args_summary.is_empty() {
                    String::new()
                } else {
                    format!("({})", truncate_str(&step.args_summary, 60))
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("    {connector}  "),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled("⏺ ", Style::default().fg(bullet_color)),
                    Span::styled(
                        step.tool_name.clone(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(args_display, Style::default().fg(Color::White)),
                ]));
            }
            // If no tool steps recorded, show Done summary
            if entry.tool_steps.is_empty() {
                let mut parts = vec![format!(
                    "{} tool use{}",
                    entry.tool_count,
                    plural(entry.tool_count)
                )];
                if let Some(tokens) = entry.estimated_tokens {
                    if tokens > 0 {
                        parts.push(format!("{} tokens", format_compact_tokens(tokens)));
                    }
                }
                lines.push(Line::from(vec![
                    Span::styled("    ⎿  ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("Done ({})", parts.join(" · ")),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
        }
    }

    lines
}

/// Render live (in-progress) subagent — Claude Code style per-entry blocks.
pub(super) fn render_subagent_group_live(entries: &[&SubagentEntry]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for entry in entries {
        let is_running = entry.status == SubagentStatus::Running;
        let bullet_color = match entry.status {
            SubagentStatus::Running => Color::Yellow,
            SubagentStatus::Failed => Color::Red,
            SubagentStatus::Done => Color::Green,
        };
        let task_preview = truncate_str(&entry.task, 60);
        let label = if entry.agent_name.is_empty() { "Task" } else { &entry.agent_name };

        // Header: ⏺ agent_name(task_preview)
        lines.push(Line::from(vec![
            Span::styled("  ⏺ ", Style::default().fg(bullet_color)),
            Span::styled(
                label.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({})", task_preview),
                Style::default().fg(Color::White),
            ),
        ]));

        // Current activity sub-lines (only when running) — show last 3
        if is_running {
            if !entry.tool_steps.is_empty() {
                let total = entry.tool_steps.len();
                let start = total.saturating_sub(3);
                let visible = &entry.tool_steps[start..];
                for (i, step) in visible.iter().enumerate() {
                    let is_last = start + i == total - 1;
                    let connector = if is_last { "⎿" } else { "│" };
                    let step_bullet_color = match step.status {
                        StepStatus::Done => Color::Green,
                        StepStatus::Failed => Color::Red,
                        StepStatus::InProgress => Color::Yellow,
                    };
                    let args_display = if step.args_summary.is_empty() {
                        String::new()
                    } else {
                        format!("({})", truncate_str(&step.args_summary, 50))
                    };
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("    {connector}  "),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled("⏺ ", Style::default().fg(step_bullet_color)),
                        Span::styled(
                            step.tool_name.clone(),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(args_display, Style::default().fg(Color::White)),
                    ]));
                }
                if start > 0 {
                    lines.push(Line::from(vec![
                        Span::styled("       ", Style::default()),
                        Span::styled(
                            format!("+{} more", start),
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::ITALIC),
                        ),
                    ]));
                }
            } else if let Some(activity) = &entry.current_activity {
                let activity_preview = truncate_str(activity, 60);
                lines.push(Line::from(vec![
                    Span::styled("    ⎿  ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        activity_preview.to_string(),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        } else {
            // Done entry in live view — show stats
            let stats = subagent_stats_str(entry);
            if !stats.is_empty() {
                // stats starts with " · ", strip leading " · " for Done(...)
                let stats_inner = stats.trim_start_matches(" · ");
                lines.push(Line::from(vec![
                    Span::styled("    ⎿  ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("Done ({})", stats_inner),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled("    ⎿  ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        "Done".to_string(),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
        }
    }

    lines
}

pub(super) fn render_plan_block(summary: &str, status: &str) -> Vec<Line<'static>> {
    let (header_icon, header_color) = match status {
        "planned" => ("◇", Color::Cyan),
        "completed" => ("✓", Color::Green),
        _ => ("✻", Color::Yellow), // executing/approved
    };
    vec![Line::from(vec![
        Span::styled(
            format!("{header_icon} "),
            Style::default().fg(header_color),
        ),
        Span::styled(
            summary.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ])]
}

/// Generate a summary string for a tool group, e.g. "Read 2 files, ran 1 command".
fn tool_group_summary(steps: &[ToolStep]) -> String {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for step in steps {
        *counts.entry(step.tool_name.as_str()).or_insert(0) += 1;
    }

    let mut parts = Vec::new();
    let order = ["Read", "Write", "Edit", "Bash", "Grep", "Glob", "Task"];
    for name in order {
        if let Some(&count) = counts.get(name) {
            let label = match name {
                "Read" => format!("read {} file{}", count, plural(count)),
                "Write" => format!("wrote {} file{}", count, plural(count)),
                "Edit" => format!("edited {} file{}", count, plural(count)),
                "Bash" => format!("ran {} command{}", count, plural(count)),
                "Grep" => format!(
                    "searched {} pattern{}",
                    count,
                    plural(count)
                ),
                "Glob" => format!(
                    "listed {} glob{}",
                    count,
                    plural(count)
                ),
                "Task" => format!(
                    "delegated {} task{}",
                    count,
                    plural(count)
                ),
                _ => format!("{} x{}", name, count),
            };
            parts.push(label);
            counts.remove(name);
        }
    }
    // Remaining tools not in the order list
    for (name, count) in &counts {
        parts.push(format!("{} x{}", name, count));
    }

    if parts.is_empty() {
        "No tool calls".to_string()
    } else {
        parts.join(", ")
    }
}

/// Render a compact sticky plan progress view for the bottom of scrollable content.
pub fn render_plan_sticky(summary: &str) -> Vec<Line<'static>> {
    vec![Line::from(vec![
        Span::styled("✻ ", Style::default().fg(Color::Yellow)),
        Span::styled(
            summary.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ])]
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}
