use inquire::{Confirm, InquireError, Select, Text};
use nucleo_matcher::{
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
    Matcher, Utf32Str,
};

use super::style::{is_section_header, BACK};

/// User's selection result: a choice, Back navigation, or error.
pub enum PromptResult<T> {
    /// User selected a valid choice.
    Choice(T),
    /// User selected "← Back" or pressed Escape.
    Back,
    /// Prompt was interrupted (Ctrl+C).
    Interrupted,
}

// ── Nucleo scorer ────────────────────────────────────────────────────

/// Score a string against input using nucleo fuzzy matching.
fn nucleo_score(input: &str, haystack: &str) -> Option<i64> {
    if input.is_empty() {
        return Some(0);
    }
    let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
    let pattern = Pattern::new(
        input,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut buf = Vec::new();
    let hay = Utf32Str::new(haystack, &mut buf);
    pattern.score(hay, &mut matcher).map(|s| s as i64)
}

/// Scorer closure for `Select::with_scorer` with `String` options.
#[allow(clippy::ptr_arg)] // inquire's Scorer<String> requires &String, not &str
fn nucleo_scorer_string(input: &str, option: &String, _: &str, _: usize) -> Option<i64> {
    nucleo_score(input, option.as_str())
}

// ── Prompt wrappers ──────────────────────────────────────────────────

/// Select prompt with "← Back" appended and section header filtering.
///
/// If the user selects a section header, re-prompts automatically.
/// Returns `PromptResult::Back` on Escape or "← Back" selection.
pub fn styled_select(message: &str, choices: Vec<String>) -> PromptResult<String> {
    let mut items = choices;
    items.push(BACK.to_string());

    loop {
        let result = Select::new(message, items.clone())
            .with_page_size(20)
            .prompt();

        match result {
            Ok(ref choice) if choice == BACK => return PromptResult::Back,
            Ok(ref choice) if is_section_header(choice) => continue,
            Ok(choice) => return PromptResult::Choice(choice),
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                return PromptResult::Back;
            }
            Err(_) => return PromptResult::Interrupted,
        }
    }
}

/// Fuzzy select with nucleo scoring, "← Back", and section header filtering.
pub fn styled_fuzzy(message: &str, choices: Vec<String>) -> PromptResult<String> {
    let mut items = choices;
    items.push(BACK.to_string());

    loop {
        let result = Select::new(message, items.clone())
            .with_scorer(&nucleo_scorer_string)
            .with_page_size(20)
            .prompt();

        match result {
            Ok(ref choice) if choice == BACK => return PromptResult::Back,
            Ok(ref choice) if is_section_header(choice) => continue,
            Ok(choice) => return PromptResult::Choice(choice),
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                return PromptResult::Back;
            }
            Err(_) => return PromptResult::Interrupted,
        }
    }
}

/// Text input prompt. Returns `PromptResult::Back` on Escape/cancel.
pub fn styled_text(message: &str) -> PromptResult<String> {
    match Text::new(message).prompt() {
        Ok(text) if text.is_empty() => PromptResult::Back,
        Ok(text) => PromptResult::Choice(text),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            PromptResult::Back
        }
        Err(_) => PromptResult::Interrupted,
    }
}

/// Yes/no confirmation prompt.
pub fn styled_confirm(message: &str, default: bool) -> PromptResult<bool> {
    match Confirm::new(message).with_default(default).prompt() {
        Ok(answer) => PromptResult::Choice(answer),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            PromptResult::Back
        }
        Err(_) => PromptResult::Interrupted,
    }
}
