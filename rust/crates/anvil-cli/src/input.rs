use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::io::{self, IsTerminal, Write};

use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{
    Cmd, CompletionType, Config, Context, EditMode, Editor, Helper, KeyCode, KeyEvent, Modifiers,
};

use crate::completion;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOutcome {
    Submit(String),
    Cancel,
    Exit,
}

/// Contextual completion helper that adapts to what the user is typing.
struct SlashCommandHelper {
    /// Static completions (slash commands + common arguments).
    completions: Vec<String>,
    /// Session IDs for /resume and /session switch completion.
    session_ids: Vec<String>,
    current_line: RefCell<String>,
}

impl SlashCommandHelper {
    fn new(completions: Vec<String>) -> Self {
        Self {
            completions: normalize_completions(completions),
            session_ids: Vec::new(),
            current_line: RefCell::new(String::new()),
        }
    }

    fn reset_current_line(&self) {
        self.current_line.borrow_mut().clear();
    }

    fn current_line(&self) -> String {
        self.current_line.borrow().clone()
    }

    fn set_current_line(&self, line: &str) {
        let mut current = self.current_line.borrow_mut();
        current.clear();
        current.push_str(line);
    }

    fn set_completions(&mut self, completions: Vec<String>) {
        self.completions = normalize_completions(completions);
    }

    fn set_session_ids(&mut self, ids: Vec<String>) {
        self.session_ids = ids;
    }

    /// Generate context-aware candidates for the given line at cursor position.
    fn contextual_candidates(&self, line: &str, pos: usize) -> Vec<Pair> {
        let trimmed = line.trim();
        let prefix = &line[..pos];

        // No input or no leading '/': offer slash commands + file paths
        if !prefix.starts_with('/') {
            // Check if it looks like a file path argument (contains '.' or '/' or '~')
            let last_word = prefix.split_whitespace().last().unwrap_or("");
            let looks_like_path = last_word.contains('/') || last_word.contains('.') || last_word == "~";
            if looks_like_path || (prefix.is_empty() && !trimmed.is_empty()) {
                let path_prefix = if last_word == "~" {
                    // Expand ~ to home directory
                    if let Ok(home) = std::env::var("HOME") {
                        home
                    } else {
                        return Vec::new();
                    }
                } else if last_word.starts_with("~/") {
                    if let Ok(home) = std::env::var("HOME") {
                        format!("{}/{}", home.trim_end_matches('/'), &last_word[2..])
                    } else {
                        return Vec::new();
                    }
                } else {
                    last_word.to_string()
                };
                if let Ok(paths) = completion::complete_file_path(&path_prefix) {
                    return paths.into_iter().map(|p| Pair {
                        display: p.clone(),
                        replacement: p,
                    }).collect();
                }
            }
            return Vec::new();
        }

        // Contextual completion for commands with arguments
        let parts: Vec<&str> = trimmed.splitn(3, ' ').collect();

        // If we only have the command name (no space), complete the command
        if parts.len() == 1 || !trimmed.contains(' ') {
            // Complete the slash command name
            let cmd_prefix = prefix;
            return self
                .completions
                .iter()
                .filter(|candidate| {
                    // Match candidates that start with what the user typed
                    candidate.starts_with(cmd_prefix)
                        // Also match bare slash commands: "/st" matches "/status"
                        || (candidate.starts_with(prefix) && candidate.len() > prefix.len())
                })
                .map(|candidate| Pair {
                    display: candidate.clone(),
                    replacement: candidate.clone(),
                })
                .collect();
        }

        // We have a command + arguments — provide argument completions
        let command = parts[0];
        // The argument the user is typing (after the space)
        let arg_prefix = trimmed[command.len()..].trim_start();

        match command {
            "/model" | "/effort" => {
                let partial = arg_prefix;
                let candidates = completion::complete_model_name(partial);
                return candidates.into_iter().map(|c| Pair {
                    display: c[command.len()..].trim().to_string(),
                    replacement: c[command.len()..].trim().to_string(),
                }).collect();
            }
            "/permissions" => {
                let partial = arg_prefix;
                return completion::PERMISSION_MODES
                    .iter()
                    .filter(|m| m.starts_with(partial))
                    .map(|m| Pair {
                        display: m.to_string(),
                        replacement: m.to_string(),
                    })
                    .collect();
            }
            "/config" => {
                let partial = arg_prefix;
                let sections = ["env", "hooks", "model", "plugins"];
                return sections
                    .iter()
                    .filter(|s| s.starts_with(partial))
                    .map(|s| Pair {
                        display: s.to_string(),
                        replacement: s.to_string(),
                    })
                    .collect();
            }
            "/resume" | "/session switch" | "/session delete" => {
                let partial = arg_prefix.to_lowercase();
                let mut candidates: Vec<Pair> = self
                    .session_ids
                    .iter()
                    .filter(|id| id.to_lowercase().starts_with(&partial))
                    .map(|id| Pair {
                        display: id.clone(),
                        replacement: id.clone(),
                    })
                    .collect();
                // Also offer file path completion for session files
                if let Ok(paths) = completion::complete_file_path(arg_prefix) {
                    candidates.extend(paths.into_iter().map(|p| Pair {
                        display: p.clone(),
                        replacement: p,
                    }));
                }
                return candidates;
            }
            "/export" | "/teleport" | "/mcp show" | "/plugin install" | "/skills install" => {
                // File path completion for commands that take a path argument
                if let Ok(paths) = completion::complete_file_path(arg_prefix) {
                    return paths.into_iter().map(|p| Pair {
                        display: p.clone(),
                        replacement: p,
                    }).collect();
                }
            }
            "/session fork" => {
                // No specific completion, but allow file paths
                if arg_prefix.contains('/') || arg_prefix.contains('.') {
                    if let Ok(paths) = completion::complete_file_path(arg_prefix) {
                        return paths.into_iter().map(|p| Pair {
                            display: p.clone(),
                            replacement: p,
                        }).collect();
                    }
                }
            }
            "/session" if parts.len() >= 2 => {
                let action = parts[1];
                let sub_prefix = if parts.len() > 2 { parts[2] } else { "" };
                // Complete subcommands: list, switch, fork, delete
                let session_actions = ["list", "switch", "fork", "delete"];
                if parts.len() == 2 || sub_prefix.is_empty() {
                    return session_actions
                        .iter()
                        .filter(|a| a.starts_with(action))
                        .map(|a| Pair {
                            display: format!("/session {a}"),
                            replacement: format!("/session {a}"),
                        })
                        .collect();
                }
                // Complete session IDs for switch/fork/delete
                if matches!(action, "switch" | "fork" | "delete") {
                    return self
                        .session_ids
                        .iter()
                        .filter(|id| id.starts_with(sub_prefix))
                        .map(|id| Pair {
                            display: id.clone(),
                            replacement: id.clone(),
                        })
                        .collect();
                }
            }
            "/mcp" if parts.len() >= 2 => {
                let sub = parts[1];
                if parts.len() == 2 {
                    let actions = ["list", "show", "help"];
                    return actions
                        .iter()
                        .filter(|a| a.starts_with(sub))
                        .map(|a| Pair {
                            display: format!("/mcp {a}"),
                            replacement: format!("/mcp {a}"),
                        })
                        .collect();
                }
            }
            "/plugin" | "/plugins" | "/marketplace" => {
                if parts.len() == 2 {
                    let sub = parts[1];
                    let actions = ["list", "install", "enable", "disable", "uninstall", "update"];
                    return actions
                        .iter()
                        .filter(|a| a.starts_with(sub))
                        .map(|a| Pair {
                            display: format!("/{} {a}", parts[0].trim_start_matches('/')),
                            replacement: format!("/{} {a}", parts[0].trim_start_matches('/')),
                        })
                        .collect();
                }
            }
            "/history" => {
                // Common history count values
                let counts = ["10", "20", "50", "100"];
                return counts
                    .iter()
                    .filter(|c| c.starts_with(arg_prefix))
                    .map(|c| Pair {
                        display: c.to_string(),
                        replacement: c.to_string(),
                    })
                    .collect();
            }
            _ => {}
        }

        // Fallback: match against static completions
        self
            .completions
            .iter()
            .filter(|candidate| candidate.starts_with(prefix))
            .map(|candidate| Pair {
                display: candidate.clone(),
                replacement: candidate.clone(),
            })
            .collect()
    }
}

impl Completer for SlashCommandHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        let candidates = self.contextual_candidates(line, pos);
        // Return start=0 so replacement completely overwrites the current word
        Ok((0, candidates))
    }
}

impl Hinter for SlashCommandHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Option<String> {
        // Only show hints when cursor is at end of line
        if pos != line.len() {
            return None;
        }

        let trimmed = line.trim();

        if trimmed == "/" {
            // Show first few matching commands as hint when user types just "/"
            let preview: Vec<&str> = self
                .completions
                .iter()
                .filter(|c| c.starts_with('/') && c.len() > 1)
                .take(5)
                .map(|s| s.trim_start_matches('/'))
                .collect();
            if !preview.is_empty() {
                let remaining = self.completions.len().saturating_sub(5);
                if remaining > 0 {
                    return Some(format!(
                        " ({}) +{} more, Tab to see all",
                        preview.join(", "),
                        remaining
                    ));
                }
                return Some(format!(" ({}) ... Tab to select", preview.join(", ")));
            }
        } else if trimmed.len() > 1 {
            let command = trimmed.split_whitespace().next().unwrap_or("");
            if trimmed.starts_with('/') && !trimmed.contains(' ') {
                // Completing a slash command name
                let matches: Vec<&String> = self
                    .completions
                    .iter()
                    .filter(|c| c.starts_with(trimmed))
                    .collect();

                if matches.is_empty() {
                    return Some(" (no matching commands)".to_string());
                }

                if matches.len() == 1 {
                    let completion = matches[0].trim_start_matches(trimmed);
                    if !completion.is_empty() {
                        return Some(format!("{}  [Tab to complete]", completion));
                    }
                    return Some(" [exact match]".to_string());
                }

                // Multiple matches - show them
                let preview: Vec<&str> = matches.iter().take(4).map(|s| s.as_str()).collect();
                let remaining = matches.len().saturating_sub(4);
                if remaining > 0 {
                    return Some(format!(
                        " ({} matches: {} +{})",
                        matches.len(),
                        preview.join(", "),
                        remaining
                    ));
                }
                return Some(format!(
                    " ({} matches: {})",
                    matches.len(),
                    preview.join(", ")
                ));
            } else if command == "/model" || command == "/effort" {
                // Show model alias hints
                let partial = trimmed[command.len()..].trim();
                let matches: Vec<&str> = completion::KNOWN_MODEL_ALIASES
                    .iter()
                    .filter(|m| m.starts_with(partial))
                    .copied()
                    .collect();
                if !matches.is_empty() {
                    let preview = matches.join(", ");
                    return Some(format!(" ({})  [Tab to complete]", preview));
                }
            } else if command == "/permissions" {
                let partial = trimmed[command.len()..].trim();
                let matches: Vec<&str> = completion::PERMISSION_MODES
                    .iter()
                    .filter(|m| m.starts_with(partial))
                    .copied()
                    .collect();
                if !matches.is_empty() {
                    let preview = matches.join(", ");
                    return Some(format!(" ({})  [Tab to complete]", preview));
                }
            } else if matches!(command, "/resume" | "/session switch" | "/session delete") {
                let partial = trimmed[command.len()..].trim().to_lowercase();
                let matches: Vec<&String> = self
                    .session_ids
                    .iter()
                    .filter(|id| id.to_lowercase().starts_with(&partial))
                    .collect();
                if !matches.is_empty() {
                    let preview: Vec<&str> = matches.iter().take(3).map(|s| s.as_str()).collect();
                    let remaining = matches.len().saturating_sub(3);
                    if remaining > 0 {
                        return Some(format!(
                            " ({} matches: {} +{})",
                            matches.len(),
                            preview.join(", "),
                            remaining
                        ));
                    }
                    return Some(format!(
                        " ({} matches: {})  [Tab to complete]",
                        matches.len(),
                        preview.join(", ")
                    ));
                }
            } else if matches!(command, "/export" | "/teleport" | "/mcp show") {
                // File path hints — only when there are interesting directories
                if let Some(arg) = trimmed.split_whitespace().nth(1) {
                    if arg.contains('/') || arg.contains('.') || arg.is_empty() {
                        if let Ok(paths) = completion::complete_file_path(arg) {
                            if !paths.is_empty() {
                                let preview: Vec<&str> = paths.iter().take(3).map(|s| s.as_str()).collect();
                                let remaining = paths.len().saturating_sub(3);
                                if remaining > 0 {
                                    return Some(format!(
                                        " ({}, … +{} more)  [Tab to complete]",
                                        preview.join(", "),
                                        remaining
                                    ));
                                }
                                return Some(format!(
                                    " ({})  [Tab to complete]",
                                    preview.join(", ")
                                ));
                            }
                        }
                    }
                }
            }
        } else if trimmed.is_empty() {
            // Show hint when line is empty
            return Some(" (type / for commands)".to_string());
        }
        None
    }
}

impl Highlighter for SlashCommandHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        self.set_current_line(line);
        Cow::Borrowed(line)
    }

    fn highlight_char(&self, line: &str, _pos: usize, _kind: CmdKind) -> bool {
        self.set_current_line(line);
        false
    }
}

impl Validator for SlashCommandHelper {}
impl Helper for SlashCommandHelper {}

pub struct LineEditor {
    prompt: String,
    editor: Editor<SlashCommandHelper, DefaultHistory>,
}

impl LineEditor {
    #[must_use]
    pub fn new(prompt: impl Into<String>, completions: Vec<String>) -> Self {
        let config = Config::builder()
            .completion_type(CompletionType::List)
            .edit_mode(EditMode::Emacs)
            .build();
        let mut editor = Editor::<SlashCommandHelper, DefaultHistory>::with_config(config)
            .expect("rustyline editor should initialize");
        editor.set_helper(Some(SlashCommandHelper::new(completions)));
        editor.bind_sequence(KeyEvent(KeyCode::Char('J'), Modifiers::CTRL), Cmd::Newline);
        editor.bind_sequence(KeyEvent(KeyCode::Enter, Modifiers::SHIFT), Cmd::Newline);

        Self {
            prompt: prompt.into(),
            editor,
        }
    }

    pub fn push_history(&mut self, entry: impl Into<String>) {
        let entry = entry.into();
        if entry.trim().is_empty() {
            return;
        }

        let _ = self.editor.add_history_entry(entry);
    }

    pub fn set_completions(&mut self, completions: Vec<String>) {
        if let Some(helper) = self.editor.helper_mut() {
            helper.set_completions(completions);
        }
    }

    pub fn set_session_ids(&mut self, ids: Vec<String>) {
        if let Some(helper) = self.editor.helper_mut() {
            helper.set_session_ids(ids);
        }
    }

    pub fn read_line(&mut self) -> io::Result<ReadOutcome> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return self.read_line_fallback();
        }

        if let Some(helper) = self.editor.helper_mut() {
            helper.reset_current_line();
        }

        match self.editor.readline(&self.prompt) {
            Ok(line) => Ok(ReadOutcome::Submit(line)),
            Err(ReadlineError::Interrupted) => {
                let has_input = !self.current_line().is_empty();
                self.finish_interrupted_read()?;
                if has_input {
                    Ok(ReadOutcome::Cancel)
                } else {
                    Ok(ReadOutcome::Exit)
                }
            }
            Err(ReadlineError::Eof) => {
                self.finish_interrupted_read()?;
                Ok(ReadOutcome::Exit)
            }
            Err(error) => Err(io::Error::other(error)),
        }
    }

    fn current_line(&self) -> String {
        self.editor
            .helper()
            .map_or_else(String::new, SlashCommandHelper::current_line)
    }

    fn finish_interrupted_read(&mut self) -> io::Result<()> {
        if let Some(helper) = self.editor.helper_mut() {
            helper.reset_current_line();
        }
        let mut stdout = io::stdout();
        writeln!(stdout)
    }

    fn read_line_fallback(&self) -> io::Result<ReadOutcome> {
        let mut stdout = io::stdout();
        write!(stdout, "{}", self.prompt)?;
        stdout.flush()?;

        let mut buffer = String::new();
        let bytes_read = io::stdin().read_line(&mut buffer)?;
        if bytes_read == 0 {
            return Ok(ReadOutcome::Exit);
        }

        while matches!(buffer.chars().last(), Some('\n' | '\r')) {
            buffer.pop();
        }
        Ok(ReadOutcome::Submit(buffer))
    }
}

fn slash_command_prefix(line: &str, pos: usize) -> Option<&str> {
    // Allow completion from any cursor position, not just end of line.
    // This lets users move the cursor mid-line and still get completions.
    let prefix = &line[..pos];
    if !prefix.starts_with('/') {
        return None;
    }

    Some(prefix)
}

fn normalize_completions(completions: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    completions
        .into_iter()
        .filter(|candidate| candidate.starts_with('/'))
        .filter(|candidate| seen.insert(candidate.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{slash_command_prefix, LineEditor, SlashCommandHelper};
    use rustyline::completion::Completer;
    use rustyline::highlight::Highlighter;
    use rustyline::history::{DefaultHistory, History};
    use rustyline::Context;

    #[test]
    fn extracts_slash_command_prefixes_from_any_cursor_position() {
        // Cursor at end of line
        assert_eq!(slash_command_prefix("/he", 3), Some("/he"));
        assert_eq!(slash_command_prefix("/help me", 8), Some("/help me"));
        assert_eq!(
            slash_command_prefix("/session switch ses", 19),
            Some("/session switch ses")
        );
        // Non-slash command returns None
        assert_eq!(slash_command_prefix("hello", 5), None);
        // Mid-line completion now supported - returns prefix up to cursor
        assert_eq!(slash_command_prefix("/help", 2), Some("/h"));
        assert_eq!(slash_command_prefix("/hello world", 6), Some("/hello"));
    }

    #[test]
    fn completes_matching_slash_commands() {
        let helper = SlashCommandHelper::new(vec![
            "/help".to_string(),
            "/hello".to_string(),
            "/status".to_string(),
        ]);
        let history = DefaultHistory::new();
        let ctx = Context::new(&history);
        let (start, matches) = helper
            .complete("/he", 3, &ctx)
            .expect("completion should work");

        assert_eq!(start, 0);
        assert_eq!(
            matches
                .into_iter()
                .map(|candidate| candidate.replacement)
                .collect::<Vec<_>>(),
            vec!["/help".to_string(), "/hello".to_string()]
        );
    }

    #[test]
    fn completes_matching_slash_command_arguments() {
        let helper = SlashCommandHelper::new(vec![
            "/model".to_string(),
            "/model opus".to_string(),
            "/model sonnet".to_string(),
            "/session switch alpha".to_string(),
        ]);
        let history = DefaultHistory::new();
        let ctx = Context::new(&history);
        let (start, matches) = helper
            .complete("/model o", 8, &ctx)
            .expect("completion should work");

        assert_eq!(start, 0);
        assert_eq!(
            matches
                .into_iter()
                .map(|candidate| candidate.replacement)
                .collect::<Vec<_>>(),
            vec!["/model opus".to_string()]
        );
    }

    #[test]
    fn ignores_non_slash_command_completion_requests() {
        let helper = SlashCommandHelper::new(vec!["/help".to_string()]);
        let history = DefaultHistory::new();
        let ctx = Context::new(&history);
        let (_, matches) = helper
            .complete("hello", 5, &ctx)
            .expect("completion should work");

        assert!(matches.is_empty());
    }

    #[test]
    fn tracks_current_buffer_through_highlighter() {
        let helper = SlashCommandHelper::new(Vec::new());
        let _ = helper.highlight("draft", 5);

        assert_eq!(helper.current_line(), "draft");
    }

    #[test]
    fn push_history_ignores_blank_entries() {
        let mut editor = LineEditor::new("> ", vec!["/help".to_string()]);
        editor.push_history("   ");
        editor.push_history("/help");

        assert_eq!(editor.editor.history().len(), 1);
    }

    #[test]
    fn set_completions_replaces_and_normalizes_candidates() {
        let mut editor = LineEditor::new("> ", vec!["/help".to_string()]);
        editor.set_completions(vec![
            "/model opus".to_string(),
            "/model opus".to_string(),
            "status".to_string(),
        ]);

        let helper = editor.editor.helper().expect("helper should exist");
        assert_eq!(helper.completions, vec!["/model opus".to_string()]);
    }

    #[test]
    fn contextual_candidates_offers_model_aliases() {
        let helper = SlashCommandHelper::new(vec!["/model".to_string()]);
        let candidates = helper.contextual_candidates("/model ", 7);
        assert!(
            candidates.iter().any(|c| c.replacement == "pro"),
            "should offer 'pro' as a model alias"
        );
    }

    #[test]
    fn contextual_candidates_offers_permission_modes() {
        let helper = SlashCommandHelper::new(vec!["/permissions".to_string()]);
        let candidates = helper.contextual_candidates("/permissions ", 13);
        assert!(
            candidates.iter().any(|c| c.replacement == "read-only"),
            "should offer 'read-only' as a permission mode"
        );
    }

    #[test]
    fn contextual_candidates_offers_config_sections() {
        let helper = SlashCommandHelper::new(vec!["/config".to_string()]);
        let candidates = helper.contextual_candidates("/config ", 8);
        assert!(
            candidates.iter().any(|c| c.replacement == "env"),
            "should offer 'env' as a config section"
        );
    }

    #[test]
    fn contextual_candidates_offers_session_ids() {
        let mut helper = SlashCommandHelper::new(vec!["/resume".to_string()]);
        helper.set_session_ids(vec!["session-alpha".to_string(), "session-beta".to_string()]);
        let candidates = helper.contextual_candidates("/resume sess", 14);
        assert!(
            candidates.iter().any(|c| c.replacement == "session-alpha"),
            "should offer 'session-alpha'"
        );
    }

    #[test]
    fn contextual_candidates_offers_session_subcommands() {
        let helper = SlashCommandHelper::new(vec!["/session".to_string()]);
        let candidates = helper.contextual_candidates("/session sw", 12);
        assert!(
            candidates.iter().any(|c| c.replacement == "/session switch"),
            "should offer '/session switch'"
        );
    }

    #[test]
    fn contextual_candidates_offers_mcp_subcommands() {
        let helper = SlashCommandHelper::new(vec!["/mcp".to_string()]);
        let candidates = helper.contextual_candidates("/mcp l", 6);
        assert!(
            candidates.iter().any(|c| c.replacement == "/mcp list"),
            "should offer '/mcp list'"
        );
    }
}
