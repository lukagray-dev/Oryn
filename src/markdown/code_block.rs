// =============================================================================
// Markdown Code Block Formatter (`src/markdown/code_block.rs`)
// =============================================================================
// Syntax-highlighting engine for fenced Markdown code blocks (`rust`, `js`, `python`, etc.)
// powered by the `syntect` crate. Converts token styles to HTML `<font color="...">` tags
// for Slint `StyledText` rendering.

use std::sync::LazyLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

/// Lazy-loaded global SyntaxSet containing built-in Sublime Text language definitions
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

/// Lazy-loaded global ThemeSet containing dark syntax highlighting color themes
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// Escapes HTML special characters in code strings to prevent XML/HTML parse errors.
fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Formats a fenced code block with full syntax highlighting driven by `syntect`.
/// Maps language string `lang` (e.g., `"rust"`, `"python"`, `"json"`, `"bash"`) to syntax rules.
pub fn highlight_code_block(code: &str, lang: &str) -> String {
    let syntax = SYNTAX_SET
        .find_syntax_by_token(lang)
        .or_else(|| SYNTAX_SET.find_syntax_by_first_line(code))
        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());

    let theme = THEME_SET
        .themes
        .get("base16-ocean.dark")
        .or_else(|| THEME_SET.themes.values().next())
        .expect("At least one theme must be loaded");

    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut result = String::with_capacity(code.len() * 2);

    for line in code.lines() {
        if let Ok(ranges) = highlighter.highlight_line(line, &SYNTAX_SET) {
            for (style, text) in ranges {
                let color_hex = format!(
                    "#{:02x}{:02x}{:02x}",
                    style.foreground.r, style.foreground.g, style.foreground.b
                );
                let escaped = escape_html(text);
                result.push_str(&format!("<font color=\"{}\">{}</font>", color_hex, escaped));
            }
        } else {
            result.push_str(&escape_html(line));
        }
        result.push('\n');
    }

    // Trim trailing newline added by loop if original code didn't end with one
    if !code.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}
