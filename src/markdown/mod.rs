// =============================================================================
// Markdown Parser & Engine (`src/markdown/mod.rs`)
// =============================================================================
// Main Rust backend engine for parsing Markdown documents into Slint `MarkdownElement` models
// using the high-performance `pulldown-cmark` CommonMark / GFM parser.
//
// Inline formatting (bold, italic, code, math) is handled by reconstructing the original Markdown
// syntax from `pulldown-cmark` events and passing it to `slint::StyledText::from_markdown()`
// (available in Slint >= 1.17). This lets Slint's native `StyledText` widget render
// bold/italic/code/math inline formatting without raw HTML injection.

pub mod bold;
pub mod code;
pub mod code_block;
pub mod heading;
pub mod italic;
pub mod list;
pub mod math;
pub mod paragraph;
pub mod rule;
pub mod spacer;
pub mod table;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use slint::{ModelRc, SharedString, StyledText, VecModel};

use crate::{MarkdownBlockType, MarkdownElement, MarkdownTableCell, MarkdownTableRow};

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

/// Helper struct for accumulating table parsing state across pulldown-cmark events
#[derive(Default)]
struct TableState {
    alignments: Vec<i32>,
    rows: Vec<MarkdownTableRow>,
    current_row_cells: Vec<MarkdownTableCell>,
    is_current_row_header: bool,
    current_cell_align_idx: usize,
    in_table: bool,
    in_table_head: bool,
    in_table_cell: bool,
}

impl TableState {
    fn flush_current_row(&mut self) {
        if !self.current_row_cells.is_empty() {
            let cells_model: ModelRc<MarkdownTableCell> =
                ModelRc::new(VecModel::from(self.current_row_cells.clone()));
            self.rows.push(MarkdownTableRow {
                cells: cells_model,
                is_header: self.is_current_row_header,
            });
            self.current_row_cells.clear();
        }
    }
}

/// Helper struct for accumulating code block parsing state across pulldown-cmark events
#[derive(Default)]
struct CodeBlockState {
    in_code_block: bool,
    language: String,
    code_buf: String,
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
    options.insert(Options::ENABLE_MATH);

    let parser = Parser::new_ext(markdown_text, options);
    let mut elements = Vec::new();

    let mut current_heading_level: i32 = 1;
    let mut is_in_heading = false;
    let mut is_in_paragraph = false;

    // Buffer rebuilt in Markdown syntax so StyledText::from_markdown() renders it correctly
    let mut global_rich_md = String::new();

    let mut list_stack: Vec<ListState> = Vec::new();
    let mut item_stack: Vec<ItemFrame> = Vec::new();

    // Table parsing state
    let mut table_state = TableState::default();

    // Code block parsing state
    let mut code_block_state = CodeBlockState::default();

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
                    rows: ModelRc::default(),
                    language: SharedString::default(),
                });
            }
        }
    };

    for (event, range) in parser.into_offset_iter() {
        match event {
            // ---- Headings ----
            Event::Start(Tag::Heading { level, .. }) => {
                if list_stack.is_empty() && !table_state.in_table && !code_block_state.in_code_block {
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
                            rows: ModelRc::default(),
                            language: SharedString::default(),
                        });
                        last_top_level_pos = range.end;
                    }
                    global_rich_md.clear();
                }
            }

            // ---- Paragraphs ----
            Event::Start(Tag::Paragraph) => {
                if list_stack.is_empty() && !table_state.in_table && !code_block_state.in_code_block {
                    check_and_insert_spacer(&mut elements, last_top_level_pos, range.start);
                    is_in_paragraph = true;
                    global_rich_md.clear();
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if is_in_paragraph && list_stack.is_empty() && !table_state.in_table && !code_block_state.in_code_block {
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
                            rows: ModelRc::default(),
                            language: SharedString::default(),
                        });
                        last_top_level_pos = range.end;
                    }
                    global_rich_md.clear();
                }
            }

            // ---- Horizontal Rule (---) ----
            Event::Rule => {
                if list_stack.is_empty() && !table_state.in_table && !code_block_state.in_code_block {
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
                        rows: ModelRc::default(),
                        language: SharedString::default(),
                    });
                    last_top_level_pos = range.end;
                }
            }

            // ---- Code Blocks ----
            Event::Start(Tag::CodeBlock(kind)) => {
                if list_stack.is_empty() && !table_state.in_table {
                    check_and_insert_spacer(&mut elements, last_top_level_pos, range.start);
                }
                code_block_state.in_code_block = true;
                code_block_state.language = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                code_block_state.code_buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                if code_block_state.in_code_block {
                    code_block_state.in_code_block = false;
                    let highlighted_md = code_block::highlight_code_block(
                        &code_block_state.code_buf,
                        &code_block_state.language,
                    );
                    let lang_label = if code_block_state.language.is_empty() {
                        String::new()
                    } else {
                        code_block_state.language.to_uppercase()
                    };
                    elements.push(MarkdownElement {
                        block_type: MarkdownBlockType::CodeBlock,
                        text: SharedString::from(code_block_state.code_buf.clone()),
                        rich_text: to_styled_text(&highlighted_md),
                        level: 0,
                        is_ordered: false,
                        prefix: SharedString::default(),
                        is_task: false,
                        is_checked: false,
                        depth: 0,
                        rows: ModelRc::default(),
                        language: SharedString::from(lang_label),
                    });
                    last_top_level_pos = range.end;
                }
            }

            // ---- Math Events ($...$ and $$...$$) ----
            Event::InlineMath(math_expr) => {
                let formatted = math::format_inline_math(&math_expr);
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str(&formatted);
                    frame.plain_text.push_str(&math_expr);
                } else if is_in_heading || is_in_paragraph || table_state.in_table_cell {
                    global_rich_md.push_str(&formatted);
                }
            }
            Event::DisplayMath(math_expr) => {
                if list_stack.is_empty() && !table_state.in_table && !code_block_state.in_code_block {
                    check_and_insert_spacer(&mut elements, last_top_level_pos, range.start);
                    let display_str = math::format_display_math(&math_expr);
                    elements.push(MarkdownElement {
                        block_type: MarkdownBlockType::DisplayMath,
                        text: SharedString::from(math_expr.to_string()),
                        rich_text: to_styled_text(&display_str),
                        level: 0,
                        is_ordered: false,
                        prefix: SharedString::default(),
                        is_task: false,
                        is_checked: false,
                        depth: 0,
                        rows: ModelRc::default(),
                        language: SharedString::default(),
                    });
                    last_top_level_pos = range.end;
                }
            }

            // ---- Tables ----
            Event::Start(Tag::Table(alignments)) => {
                if list_stack.is_empty() && !code_block_state.in_code_block {
                    check_and_insert_spacer(&mut elements, last_top_level_pos, range.start);
                }
                table_state.in_table = true;
                table_state.alignments = alignments.into_iter().map(table::alignment_to_int).collect();
                table_state.rows.clear();
            }
            Event::End(TagEnd::Table) => {
                if table_state.in_table {
                    table_state.flush_current_row();
                    table_state.in_table = false;
                    let rows_model: ModelRc<MarkdownTableRow> =
                        ModelRc::new(VecModel::from(table_state.rows.clone()));
                    elements.push(MarkdownElement {
                        block_type: MarkdownBlockType::Table,
                        text: SharedString::default(),
                        rich_text: to_styled_text(""),
                        level: 0,
                        is_ordered: false,
                        prefix: SharedString::default(),
                        is_task: false,
                        is_checked: false,
                        depth: 0,
                        rows: rows_model,
                        language: SharedString::default(),
                    });
                    last_top_level_pos = range.end;
                }
            }

            Event::Start(Tag::TableHead) => {
                table_state.in_table_head = true;
                table_state.is_current_row_header = true;
                table_state.current_row_cells.clear();
                table_state.current_cell_align_idx = 0;
            }
            Event::End(TagEnd::TableHead) => {
                table_state.flush_current_row();
                table_state.in_table_head = false;
            }

            Event::Start(Tag::TableRow) => {
                table_state.current_row_cells.clear();
                table_state.is_current_row_header = table_state.in_table_head;
                table_state.current_cell_align_idx = 0;
            }
            Event::End(TagEnd::TableRow) => {
                table_state.flush_current_row();
            }

            Event::Start(Tag::TableCell) => {
                table_state.in_table_cell = true;
                global_rich_md.clear();
            }
            Event::End(TagEnd::TableCell) => {
                table_state.in_table_cell = false;
                let align = table_state
                    .alignments
                    .get(table_state.current_cell_align_idx)
                    .copied()
                    .unwrap_or(0);
                table_state.current_cell_align_idx += 1;

                let md = global_rich_md.trim().to_string();
                table_state.current_row_cells.push(MarkdownTableCell {
                    text: SharedString::from(md.clone()),
                    rich_text: to_styled_text(&md),
                    alignment: align,
                    is_header: table_state.is_current_row_header,
                });
                global_rich_md.clear();
            }

            // ---- Lists ----
            Event::Start(Tag::List(start_number)) => {
                if list_stack.is_empty() && !table_state.in_table && !code_block_state.in_code_block {
                    check_and_insert_spacer(&mut elements, last_top_level_pos, range.start);
                }
                let is_ordered = start_number.is_some();
                let current_index = start_number.unwrap_or(1) as i32;
                list_stack.push(ListState { is_ordered, current_index });
            }
            Event::End(TagEnd::List(_)) => {
                list_stack.pop();
                if list_stack.is_empty() && !table_state.in_table && !code_block_state.in_code_block {
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
                    rows: ModelRc::default(),
                    language: SharedString::default(),
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
                            rows: ModelRc::default(),
                            language: SharedString::default(),
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
                } else if is_in_heading || is_in_paragraph || table_state.in_table_cell {
                    global_rich_md.push_str(marker);
                }
            }
            Event::End(TagEnd::Strong) => {
                let marker = bold::close_bold_tag();
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str(marker);
                } else if is_in_heading || is_in_paragraph || table_state.in_table_cell {
                    global_rich_md.push_str(marker);
                }
            }

            Event::Start(Tag::Emphasis) => {
                let marker = italic::open_italic_tag();
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str(marker);
                } else if is_in_heading || is_in_paragraph || table_state.in_table_cell {
                    global_rich_md.push_str(marker);
                }
            }
            Event::End(TagEnd::Emphasis) => {
                let marker = italic::close_italic_tag();
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str(marker);
                } else if is_in_heading || is_in_paragraph || table_state.in_table_cell {
                    global_rich_md.push_str(marker);
                }
            }

            Event::Start(Tag::Strikethrough) => {
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str("~~");
                } else if is_in_heading || is_in_paragraph || table_state.in_table_cell {
                    global_rich_md.push_str("~~");
                }
            }
            Event::End(TagEnd::Strikethrough) => {
                if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str("~~");
                } else if is_in_heading || is_in_paragraph || table_state.in_table_cell {
                    global_rich_md.push_str("~~");
                }
            }

            // ---- Text & Code Content ----
            Event::Text(text) => {
                if code_block_state.in_code_block {
                    code_block_state.code_buf.push_str(&text);
                } else if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push_str(&text);
                    frame.plain_text.push_str(&text);
                } else if is_in_heading || is_in_paragraph || table_state.in_table_cell {
                    global_rich_md.push_str(&text);
                }
            }
            Event::Code(code_str) => {
                if code_block_state.in_code_block {
                    code_block_state.code_buf.push_str(&code_str);
                } else {
                    let formatted = code::format_inline_code(&code_str);
                    if let Some(frame) = item_stack.last_mut() {
                        frame.rich_md.push_str(&formatted);
                        frame.plain_text.push_str(&code_str);
                    } else if is_in_heading || is_in_paragraph || table_state.in_table_cell {
                        global_rich_md.push_str(&formatted);
                    }
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if code_block_state.in_code_block {
                    code_block_state.code_buf.push('\n');
                } else if let Some(frame) = item_stack.last_mut() {
                    frame.rich_md.push('\n');
                    frame.plain_text.push('\n');
                } else if is_in_heading || is_in_paragraph || table_state.in_table_cell {
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
            || e.block_type == MarkdownBlockType::Table
            || e.block_type == MarkdownBlockType::CodeBlock
            || e.block_type == MarkdownBlockType::DisplayMath
            || !e.text.is_empty()
    });
    elements
}
