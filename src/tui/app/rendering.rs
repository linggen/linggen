use super::super::display::*;
use super::super::render;
use super::{App, ConnectionStatus};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

impl App {
    pub fn render(&mut self, f: &mut ratatui::Frame) {
        use ratatui::layout::{Constraint, Direction, Layout};

        // Fixed bottom: divider(1) + input(1) + divider(1) + status(1) = 4 lines
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),   // Scrollable content
                Constraint::Length(4), // Fixed input area
            ])
            .split(f.area());

        let content_area = chunks[0];
        let input_area = chunks[1];
        let width = content_area.width;
        let content_height = content_area.height as usize;

        // ── Scrollable content ──────────────────────────────────────
        let mut all_lines: Vec<Line<'static>> = Vec::new();
        all_lines.extend(self.banner.iter().cloned());

        for block in &self.blocks {
            // Skip inline plan blocks that are executing — sticky footer handles them.
            if matches!(block, DisplayBlock::PlanBlock { status, .. } if status != "planned" && status != "completed") {
                continue;
            }
            all_lines.extend(render::render_block(block, width));
            // Show interactive prompt right after a pending plan block
            if matches!(block, DisplayBlock::PlanBlock { status, .. } if status == "planned") {
                if let Some(prompt) = &self.prompt {
                    all_lines.push(Line::from(""));
                    for (i, option) in prompt.options.iter().enumerate() {
                        let marker = if i == prompt.selected { ">" } else { " " };
                        let label = format!("  {} {}. {}", marker, i + 1, option);
                        let style = if i == prompt.selected {
                            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        all_lines.push(Line::from(Span::styled(label, style)));
                    }
                    all_lines.push(Line::from(""));
                    all_lines.push(Line::from(Span::styled(
                        "  ↑↓ to select, Enter to confirm, or type below",
                        Style::default().fg(Color::DarkGray),
                    )));
                    all_lines.push(Line::from(""));
                }
            }
        }

        // AskUser prompt (not plan-related) — render at end of blocks
        if self.pending_ask_user_id.is_some() {
            if let Some(prompt) = &self.prompt {
                all_lines.push(Line::from(""));
                for (i, option) in prompt.options.iter().enumerate() {
                    let marker = if i == prompt.selected { ">" } else { " " };
                    let label = format!("  {} {}. {}", marker, i + 1, option);
                    let style = if i == prompt.selected {
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    all_lines.push(Line::from(Span::styled(label, style)));
                }
                all_lines.push(Line::from(""));
                all_lines.push(Line::from(Span::styled(
                    "  ↑↓ to select, Enter to confirm, or type below",
                    Style::default().fg(Color::DarkGray),
                )));
                all_lines.push(Line::from(""));
            }
        }

        // Active (in-progress) tool group
        if let Some(group) = &self.active_tool_group {
            all_lines.extend(render::render_tool_group_active(&group.steps, self.verbose_mode));
        }

        // Active (in-progress) subagent tree — rendered live
        if !self.active_subagents.is_empty() {
            let entries: Vec<&SubagentEntry> = self.active_subagents.values().collect();
            all_lines.extend(render::render_subagent_group_live(&entries));
        }

        // Thinking animation (replaces raw thinking tokens)
        if self.is_thinking_phase && self.streaming_buffer.is_empty() {
            let tick = (self.app_start.elapsed().as_millis() / 200) % 4;
            let dots = ".".repeat(tick as usize);
            all_lines.push(Line::from(Span::styled(
                format!("  Thinking{dots}"),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
            )));
        }

        // Streaming buffer
        if self.is_streaming && !self.streaming_buffer.is_empty() {
            for l in self.streaming_buffer.lines() {
                all_lines.push(Line::from(Span::raw(l.to_string())));
            }
        }

        // Sticky plan: show the active executing plan at the bottom of content.
        // Skip "planned" (shown inline) and "completed".
        let active_plan_summary = self.blocks.iter().rev().find_map(|b| {
            if let DisplayBlock::PlanBlock {
                summary,
                status,
            } = b
            {
                if status != "completed" && status != "planned" {
                    return Some(summary.clone());
                }
            }
            None
        });
        if let Some(summary) = active_plan_summary {
            all_lines.push(Line::from(""));
            all_lines.extend(render::render_plan_sticky(&summary));
        }

        // Compute total wrapped rows (accounts for line wrapping)
        let total_wrapped: usize = all_lines
            .iter()
            .map(|line| {
                let w = line.width();
                if w == 0 || width == 0 {
                    1
                } else {
                    (w + width as usize - 1) / width as usize
                }
            })
            .sum();

        let max_scroll = total_wrapped.saturating_sub(content_height);
        self.scroll_offset = self.scroll_offset.min(max_scroll);
        let scroll_y = max_scroll.saturating_sub(self.scroll_offset);

        let text = Text::from(all_lines);
        let output = Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((scroll_y as u16, 0));
        f.render_widget(output, content_area);

        // Scroll indicator overlay when not at the bottom
        if self.scroll_offset > 0 {
            let indicator = Paragraph::new(Line::from(Span::styled(
                format!("  ··· {} more rows above ···", scroll_y),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )));
            let indicator_area = ratatui::layout::Rect {
                x: content_area.x,
                y: content_area.y,
                width: content_area.width,
                height: 1,
            };
            f.render_widget(indicator, indicator_area);
        }

        // ── Autocomplete popup (overlay above input) ────────────────
        // Defensive: ensure autocomplete is cleared if input no longer warrants it
        if !self.autocomplete.is_empty() {
            let should_show = (self.input.starts_with('/') && !self.input[1..].contains(' '))
                || (self.input.starts_with('@') && !self.input[1..].contains(' '));
            if !should_show {
                self.autocomplete.clear();
            }
        }
        if !self.autocomplete.is_empty() {
            let max_items = 8.min(self.autocomplete.len());
            let popup_height = max_items as u16;
            if popup_height > 0 && content_area.height > popup_height {
                let popup_y = content_area.y + content_area.height - popup_height;
                let mut popup_lines: Vec<Line<'static>> = Vec::new();
                for (i, item) in self.autocomplete.iter().take(max_items).enumerate() {
                    let is_selected = i == self.autocomplete_selected;
                    let marker = if is_selected { "> " } else { "  " };
                    let style = if is_selected {
                        Style::default()
                            .fg(Color::Cyan)
                            .bg(Color::Black)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray).bg(Color::Black)
                    };
                    let desc_style = if is_selected {
                        Style::default().fg(Color::White).bg(Color::Black)
                    } else {
                        Style::default().fg(Color::DarkGray).bg(Color::Black)
                    };
                    // Pad each line to full width so it covers underlying content
                    let label_text = format!("{}{}", marker, item.label);
                    let desc_text = format!("  {}", item.description);
                    let used = label_text.len() + desc_text.len();
                    let padding = if (width as usize) > used {
                        " ".repeat(width as usize - used)
                    } else {
                        String::new()
                    };
                    popup_lines.push(Line::from(vec![
                        Span::styled(label_text, style),
                        Span::styled(desc_text, desc_style),
                        Span::styled(padding, Style::default().bg(Color::Black)),
                    ]));
                }
                let popup_area = ratatui::layout::Rect {
                    x: content_area.x,
                    y: popup_y,
                    width: content_area.width,
                    height: popup_height,
                };
                let popup = Paragraph::new(popup_lines);
                f.render_widget(popup, popup_area);
            }
        }

        // ── Fixed input area (always visible at bottom) ─────────────
        let divider = "─".repeat(width as usize);
        let mut input_spans = vec![
            Span::styled(
                "> ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(self.input.clone()),
        ];
        if !self.pending_images.is_empty() {
            input_spans.push(Span::styled(
                format!("  [{} image{}]", self.pending_images.len(), if self.pending_images.len() == 1 { "" } else { "s" }),
                Style::default().fg(Color::Magenta),
            ));
        }
        let input_line = Line::from(input_spans);

        let state_color = match self.status_state.as_str() {
            "thinking" => Color::Blue,
            "calling_tool" => Color::Yellow,
            "working" | "sending" => Color::Green,
            "model_loading" => Color::Magenta,
            _ => Color::DarkGray,
        };
        let mut status_spans = vec![
            Span::styled(
                format!(" {} ", self.status_agent),
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::raw("  "),
            Span::styled(
                format!(" {} ", self.status_state),
                Style::default().fg(Color::White).bg(state_color),
            ),
        ];
        if let Some(tool) = &self.status_tool {
            status_spans.push(Span::styled("  ", Style::default()));
            status_spans.push(Span::styled(
                tool.clone(),
                Style::default().fg(Color::Yellow),
            ));
        }
        // Show "Esc to cancel" hint when agent is running
        if self.status_state != "idle" && self.prompt.is_none() {
            status_spans.push(Span::styled("  ", Style::default()));
            status_spans.push(Span::styled(
                "Esc to cancel",
                Style::default().fg(Color::DarkGray),
            ));
        }
        if let ConnectionStatus::Disconnected = &self.connection_status {
            status_spans.push(Span::styled("  ", Style::default()));
            status_spans.push(Span::styled(
                " DISCONNECTED ",
                Style::default().fg(Color::White).bg(Color::Red),
            ));
        }

        let bottom = Paragraph::new(vec![
            Line::from(Span::styled(divider.clone(), Style::default().fg(Color::DarkGray))),
            input_line,
            Line::from(Span::styled(divider, Style::default().fg(Color::DarkGray))),
            Line::from(status_spans),
        ]);
        f.render_widget(bottom, input_area);

        // Position cursor on the input line (2nd line of input_area)
        let cursor_y = input_area.y + 1;
        let cursor_x = input_area.x + 2 + self.input.len() as u16;
        f.set_cursor_position((cursor_x, cursor_y));
    }
}
