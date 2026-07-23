// =============================================================================
// Markdown Parser & Engine (`src/markdown/mod.rs`)
// =============================================================================
// Main Rust backend engine for parsing Markdown documents into Slint `MarkdownElement` models
// using the high-performance `pulldown-cmark` CommonMark / GFM parser.
//
// Inline formatting (bold, italic, code) is handled by reconstructing the original Markdown
// syntax from `pulldown-cmark` events and passing it to `slint::StyledText::from_markdown()`
// (available in Slint >= 1.17). This lets Slint's native `StyledText` widget render
// bold/italic/code inline formatting without raw HTML injection.

pub mod bold;
pub mod code;
pub mod heading;
pub mod italic;
pub mod list;
pub mod paragraph;
pub mod rule;
pub mod spacer;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use slint::{SharedString, StyledText};

use crate::{MarkdownBlockType, MarkdownElement};

#[derive(Debug, Clone)]
struct ListState {
    is_ordered: bool,
    current_index: i32,
}

#[derive(Debug, Clone)]
struct ItemFrame {
    element_index: usize,
    depth: i32,
    is_ordered: bool,
    prefix: String,
    is_task: bool,
    is_checked: bool,
    // Reconstructed Markdown text for rich StyledText rendering
    rich_md: String,
    // Plain text (task status, index matching, etc.)
    plain_text: String,
}

/// Converts a Markdown string into a Slint `StyledText` value.
/// `from_markdown()` returns a `Result`; on parse error, falls back to a default
/// (empty StyledText) rather than panicking — safe for all user input.
fn to_styled_text(md: &str) -> StyledText {
    StyledText::from_markdown(md).unwrap_or_default()
}

/// Parses raw Markdown string into a vector of Slint `MarkdownElement` blocks.
/// Uses `slint::StyledText::from_markdown()` for native bold/italic/code rendering.
pub fn parse_markdown(markdown_text: &str) -> Vec<MarkdownElement> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let parser = Parser::new_ext(markdown_text, options);
    let mut elements = Vec::new();

    let mut current_heading_level: i32 = 1;
    let mut is_in_heading = false;
    let mut is_in_paragraph = false;

    // Buffer rebuilt in Markdown syntax so StyledText::from_markdown() renders it correctly
    let mut global_rich_md = String::new();

    let mut list_stack: Vec<ListState> = Vec::new();
    let mut item_stack: Vec<ItemFrame> = Vec::new();

    // Track end position of previous top-level block to compute blank line gaps
    let mut last_top_level_pos: usize = 0;

    // Helper closure to insert a Spacer element if blank lines exist between blocks
    let check_and_insert_spacer = |elements: &mut Vec<MarkdownElement>, last_pos: usize, current_start: usize| {
        if current_start > last_pos && last_pos < markdown_text.len() {
            let slice = &markdown_text[last_pos..current_start.min(markdown_text.len())];
            let blank_lines = spacer::calculate_blank_lines(slice);
            if blank_lines > 0 {
                elements.push(MarkdownElement {
                    block_type: MarkdownBlockType::Spacer,
                    text: SharedString::default(),
                    rich_text: to_styled_text(""),
                    level: blank_lines,
                    is_ordered: false,
                    prefix: SharedString::default(),
                    is_task: false,
                    is_checked: false,
                    depth: 0,
                });
            }
        }
    };

    for (event, range) in parser.into_offset_iter() {
        match event {
            // ---- Headings ----
            Event::Start(Tag::Heading { level, .. }) => {
                if list_stack.is_empty() {
                    check_and_insert_spacer(&mut elements, last_top_level_pos, range.start);
                }
                is_in_heading = true;
                current_heading_level = heading::heading_level_to_int(level);
                global_rich_md.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                if is_in_heading {
                    is_in_heading = false;
                    let md = global_rich_md.trim().to_string();
                    if !md.is_empty() {
                        elements.push(MarkdownElement {
                            block_type: MarkdownBlockType::Heading,
                            text: SharedString::from(md.clone()),
                            rich_text: to_styled_text(&md),
                            level: current_heading_level,
                            is_ordered: false,
                            prefix: SharedString::default(),
                            is_task: false,
                            is_checked: false,
                            depth: 0,
                        });
                        last_top_level_pos = range.end;
                    }
                    global_rich_md.clear();
                }
            }

            // ---- Paragraphs ----
            Event::Start(Tag::Paragraph) => {
                if list_stack.is_empty() {
                    check_and_insert_spacer(&mut elements, last_top_level_pos, range.start);
                    is_in_paragraph = true;
                    global_rich_md.clear();
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if is_in_paragraph && list_stack.is_empty() {
                    is_in_paragraph = false;
                    let md = paragraph::clean_paragraph_text(global_rich_md.trim());
                    if !md.is_empty() {
                        elements.push(MarkdownElement {
                            block_type: MarkdownBlockType::Paragraph,
                            text: SharedString::from(md.clone()),
                            rich_text: to_styled_text(&md),
                            level: 0,
                            is_ordered: false,
                            prefix: SharedString::default(),
                            is_task: false,
                            is_checked: false,
                            depth: 0,
                        });
                        last_top_level_pos = range.end;
                    }
                    global_rich_md.clear();
                }
            }

            // ---- Horizontal Rule (---) ----
            Event::Rule => {
                if list_stack.is_empty() {
                    check_and_insert_spacer(&mut elements, last_top_level_pos, range.start);
                    elements.push(MarkdownElement {
                        block_type: MarkdownBlockType::Rule,
                        text: SharedString::from(rule::format_rule()),
                        rich_text: to_styled_text(""),
                        level: 0,
                        is_ordered: false,
                        prefix: SharedString::default(),
                        is_task: false,
                        is_checked: false,
                        depth: 0,
                    });
                    last_top_level_pos = range.end;
                }
            }

            // ---- Lists ----
            Event::Start(Tag::List(start_number)) => {
                if list_stack.is_empty() {
                    check_and_insert_spacer(&mut elements, last_top_level_pos, range.start);
                }
                let is_ordered = start_number.is_some();
                let current_index = start_number.unwrap_or(1) as i32;
                list_stack.push(ListState { is_ordered, current_index });
            }
            Event::End(TagEnd::List(_)) => {
                list_stack.pop();
                if list_stack.is_empty() {
                    last_top_level_pos = range.end;
                }
            }

            // ---- List Items ----
            Event::Start(Tag::Item) => {
                let depth = (list_stack.len() as i32 - 1).max(0);
                let (is_ordered, prefix) = if let Some(last) = list_stack.last_mut() {
                    let is_ord = last.is_ordered;
                    let pref = if is_ord {
                        let p = list::format_ordered_prefix(last.current_index, depth);
                        last.current_index += 1;
                        p
                    } else {
                        String::new()
                    };
                    (is_ord, pref)
                } else {
                    (false, String::new())
                };

                let element_index = elements.len();
                // Pre-allocate placeholder to preserve document order
                elements.push(MarkdownElement {
                    block_type: MarkdownBlockType::ListItem,
                    text: SharedString::default(),
                    rich_text: to_styled_text(""),
                    level: 0,
                    is_ordered,
                    prefix: SharedString::from(prefix.clone()),
                    is_task: false,
                    is_checked: false,
                    depth,
                });

                item_stack.push(ItemFrame {
                    element_index,
                    depth,
                    is_ordered,
                    prefix,
                    is_task: false,
                    is_checked: false,
                    rich_md: String::new(),
                    plain_text: String::new(),
                });
            }
            Event::End(TagEnd::Item) => {
                if let Some(frame) = item_stack.pop() {
                    let md = frame.rich_md.trim().to_string();
                    if frame.element_index < elements.len() {
                        elements[frame.element_index] = MarkdownElement {
                            block_type: MarkdownBlockType::ListItem,
                            text: SharedString::from(frame.plain_text.trim()),
                            rich_text: to_styled_text(&md),
                            level: 0,
                            is_ordered: frame.is_ordered,
                            prefix: SharedString::from(frame.prefix),
                            is_task: frame.is_task,
                            is_checked: frame.is_checked,
                            depth: frame.depth,
                        };
                    }
                }
            }

            // Task List Markers
            Event::TaskListMarker(checked) => {
                if let Some(frame) = item_stack.last_mut() {
                    frame.is_task = true;
                    frame.is_checked = checked;
                }
            }

            // ---- Inline Formatting ----
            // Reconstruct Markdown delimiters so StyledText::from_markdown() renders them

            Event::Start(Tag::Strong) => {
                let marker = bold::open_bold_tag();
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str(marker);
                } else if is_in_heading || is_in_paragraph {
                    global_rich_md.push_str(marker);
                }
            }
            Event::End(TagEnd::Strong) => {
                let marker = bold::close_bold_tag();
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str(marker);
                } else if is_in_heading || is_in_paragraph {
                    global_rich_md.push_str(marker);
                }
            }

            Event::Start(Tag::Emphasis) => {
                let marker = italic::open_italic_tag();
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str(marker);
                } else if is_in_heading || is_in_paragraph {
                    global_rich_md.push_str(marker);
                }
            }
            Event::End(TagEnd::Emphasis) => {
                let marker = italic::close_italic_tag();
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str(marker);
                } else if is_in_heading || is_in_paragraph {
                    global_rich_md.push_str(marker);
                }
            }

            Event::Start(Tag::Strikethrough) => {
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str("~~");
                } else if is_in_heading || is_in_paragraph {
                    global_rich_md.push_str("~~");
                }
            }
            Event::End(TagEnd::Strikethrough) => {
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str("~~");
                } else if is_in_heading || is_in_paragraph {
                    global_rich_md.push_str("~~");
                }
            }

            // ---- Text & Code Content ----
            Event::Text(text) => {
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str(&text);
                    frame.plain_text.push_str(&text);
                } else if is_in_heading || is_in_paragraph {
                    global_rich_md.push_str(&text);
                }
            }
            Event::Code(code_str) => {
                let formatted = code::format_inline_code(&code_str);
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str(&formatted);
                    frame.plain_text.push_str(&code_str);
                } else if is_in_heading || is_in_paragraph {
                    global_rich_md.push_str(&formatted);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push('\n');
                    frame.plain_text.push('\n');
                } else if is_in_heading || is_in_paragraph {
                    global_rich_md.push('\n');
                }
            }
            _ => {}
        }
    }

    // Check for trailing blank lines at end of file
    check_and_insert_spacer(&mut elements, last_top_level_pos, markdown_text.len());

    // Remove placeholder items with no content
    elements.retain(|e| {
        e.block_type == MarkdownBlockType::ListItem
            || e.block_type == MarkdownBlockType::Rule
            || e.block_type == MarkdownBlockType::Spacer
            || !e.text.is_empty()
    });
    elements
}
