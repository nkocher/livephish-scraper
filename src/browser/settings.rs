use crate::config::{save_config, Config};
use crate::models::FormatCode;
use crate::transcode::check_ffmpeg;

use super::prompt::{styled_select, styled_text, PromptResult};
use super::style::{clear_screen, dim, print_section};

/// Settings menu loop — edit format, postprocess, output dir.
pub fn edit_settings(config: &mut Config) {
    loop {
        clear_screen();

        let format_display = FormatCode::from_name(&config.format)
            .map(|fc| fc.label().to_string())
            .unwrap_or_else(|| config.format.to_uppercase());

        let postprocess_display = match config.postprocess_codec.as_str() {
            "flac" => "Convert to FLAC",
            "alac" => "Convert to ALAC",
            _ => "None",
        };

        print_section("Settings", None);

        let choices = vec![
            format!("Audio format        [{format_display}]"),
            format!("AAC conversion      [{postprocess_display}]"),
            format!("Output directory     [{}]", config.output_dir),
        ];

        match styled_select("", choices) {
            PromptResult::Choice(choice) => {
                if choice.starts_with("Audio format") {
                    edit_format(config);
                } else if choice.starts_with("AAC conversion") {
                    edit_postprocess(config);
                } else if choice.starts_with("Output directory") {
                    edit_output_dir(config);
                }
            }
            PromptResult::Back | PromptResult::Interrupted => return,
        }
    }
}

/// Edit audio format preference.
fn edit_format(config: &mut Config) {
    let format_codes = [
        FormatCode::Flac,
        FormatCode::Alac,
        FormatCode::Aac,
        FormatCode::Mqa,
        FormatCode::Ra360,
    ];

    let labels: Vec<String> = format_codes
        .iter()
        .map(|fc| format!("{} ({})", fc.label(), fc.name()))
        .collect();

    match styled_select("Audio format:", labels.clone()) {
        PromptResult::Choice(label) => {
            if let Some(pos) = labels.iter().position(|l| *l == label) {
                let new_format = format_codes[pos].name().to_string();
                if new_format != config.format {
                    config.format = new_format;
                    save_config(config);
                }
                println!("\x1b[38;5;113mAudio format saved.\x1b[0m");
            }
        }
        PromptResult::Back | PromptResult::Interrupted => {}
    }
}

/// Edit AAC post-processing codec.
fn edit_postprocess(config: &mut Config) {
    let choices = vec![
        "None (keep original)".to_string(),
        "Convert AAC to FLAC".to_string(),
        "Convert AAC to ALAC (lossless)".to_string(),
    ];

    match styled_select("AAC conversion:", choices) {
        PromptResult::Choice(choice) => {
            let codec = match choice {
                s if s.contains("FLAC") => "flac",
                s if s.contains("ALAC") => "alac",
                _ => "none",
            };

            let effective = if codec != "none" && !check_ffmpeg() {
                println!(
                    "\x1b[38;5;214mffmpeg not found \u{2014} AAC tracks will not be converted.\x1b[0m"
                );
                println!("  {}", dim("Install ffmpeg and ensure it's on your PATH."));
                "none"
            } else {
                codec
            };

            if effective != config.postprocess_codec {
                config.postprocess_codec = effective.to_string();
                save_config(config);
            }
            println!("\x1b[38;5;113mAAC conversion saved.\x1b[0m");
        }
        PromptResult::Back | PromptResult::Interrupted => {}
    }
}

/// Edit output directory.
fn edit_output_dir(config: &mut Config) {
    match styled_text(&format!(
        "Output directory {}:",
        dim(&format!("[{}]", config.output_dir))
    )) {
        PromptResult::Choice(dir) => {
            let trimmed = dir.trim().to_string();
            if trimmed.is_empty() {
                println!("\x1b[38;5;214mEmpty directory \u{2014} keeping current value.\x1b[0m");
                return;
            }
            config.output_dir = trimmed;
            save_config(config);
            println!("\x1b[38;5;113mOutput directory saved.\x1b[0m");
        }
        PromptResult::Back | PromptResult::Interrupted => {}
    }
}
