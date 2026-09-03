use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    DeleteTier1,
    DeleteTier2,
    Human,
    Indeterminate,
}

#[derive(Debug, Clone)]
pub struct Classification {
    pub class: Class,
    pub reason: String,
}

const AI_TIER1_PREFIXES: &[&str] = &["You are ", "You are executing"];
const AI_TIER1_SUBSTRINGS: &[&str] = &["Worktree:", "WORKTREE:", "leaf L"];
const AI_TIER2_SUBSTRINGS: &[&str] = &[
    "OBJECTIVE",
    "Read-only",
    "Do NOT modify",
    "STRICT FILE",
];

pub fn classify_first_user_message(raw: &str) -> Classification {
    let stripped_owned = strip_task(raw);
    let stripped = stripped_owned.trim();
    let lower = stripped.to_ascii_lowercase();
    let len = stripped.chars().count();
    let starts_lowercase = stripped
        .chars()
        .next()
        .map(|c| c.is_ascii_lowercase())
        .unwrap_or(false);
    let has_ai_marker = ai_marker(stripped, &lower);
    let human_marker = human_marker(&lower);

    // KEEP override (highest priority): terminal pastes and session resumes.
    // The user pastes shell output / resumes prior sessions into forge; those
    // first messages may contain command output that looks AI-ish, but they are
    // human-originated and must NOT be deleted. Detect before any AI tier.
    if looks_like_terminal_paste(stripped, &lower) {
        return Classification {
            class: Class::Human,
            reason: "terminal paste / session resume (keep)".to_string(),
        };
    }

    if AI_TIER1_PREFIXES.iter().any(|p| stripped.starts_with(p))
        || AI_TIER1_SUBSTRINGS.iter().any(|s| stripped.contains(s))
    {
        return Classification {
            class: Class::DeleteTier1,
            reason: "tier1 AI marker".to_string(),
        };
    }

    if AI_TIER2_SUBSTRINGS.iter().any(|s| stripped.contains(s))
        || (len > 800 && looks_formal_imperative(stripped))
    {
        return Classification {
            class: Class::DeleteTier2,
            reason: "tier2 AI marker".to_string(),
        };
    }

    if human_marker || (len < 120 && starts_lowercase && !has_ai_marker) {
        return Classification {
            class: Class::Human,
            reason: "human marker".to_string(),
        };
    }

    Classification {
        class: Class::Indeterminate,
        reason: "fallback keep".to_string(),
    }
}

pub fn strip_task(input: &str) -> String {
    let trimmed = input.trim();
    if let Some(inner) = trimmed.strip_prefix("<task>") {
        if let Some(inner) = inner.strip_suffix("</task>") {
            return inner.to_string();
        }
    }
    trimmed.to_string()
}

fn ai_marker(text: &str, lower: &str) -> bool {
    lower.contains("worktree:")
        || lower.contains("you are executing")
        || lower.contains("leaf l")
        || lower.contains("objective")
        || lower.contains("read-only")
        || lower.contains("do not modify")
        || lower.contains("strict file")
        || text.starts_with("You are ")
        || text.starts_with("You are executing")
}

fn human_marker(lower: &str) -> bool {
    lower.starts_with("do the next items")
        || lower.starts_with("resume.")
        || lower.starts_with("proc")
        || lower.starts_with("work on omniroute")
        || lower.starts_with("call agents")
        || lower.starts_with("am i the only one")
}

/// Detect terminal pastes / session-resume dumps that the user pasted into
/// forge. These are human-originated and must be KEPT even if their body
/// contains command output that resembles AI task language.
fn looks_like_terminal_paste(text: &str, lower: &str) -> bool {
    // Shell login / banner markers.
    lower.contains("last login:")
        || lower.contains("on ttys")
        // forge / CLI ASCII banner fragments and prompt.
        || text.contains("❯")
        || text.contains("_____\n|  ___")
        || lower.contains("v25.7.0")
        || lower.contains("nightly")
        // Pasted agent run logs (timestamped execute lines, tool markers).
        || text.contains("⏺ [")
        || lower.contains("] execute [/bin/")
        // zsh/bash prompt remnants with a path segment and a trailing prompt char.
        || (text.contains("~/C/P/repos") && (text.contains('$') || text.contains('%')))
        // "Let me verify ... Execute" style resumed-session preambles.
        || (lower.starts_with("let me ") && lower.contains("execute"))
}

fn looks_formal_imperative(text: &str) -> bool {
    let imperative_hits = [
        "must",
        "should",
        "do not",
        "never",
        "refuse",
        "verify",
        "run it only",
        "delete nothing",
    ];
    let lower = text.to_ascii_lowercase();
    imperative_hits.iter().any(|s| lower.contains(s))
}

pub fn first_user_message(messages: &Value) -> Option<String> {
    messages
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|arr| arr.get(2))
        .and_then(|m| m.get("message"))
        .and_then(|m| m.get("text"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
}
