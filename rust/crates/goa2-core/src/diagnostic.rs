//! Internal diagnostic classification and bounded boundary context. These codes
//! do not change HTTP/worker error packets or make message text a stable API.
#![warn(missing_docs)]
use crate::interpreter::EngineError;

/// Stable native diagnostic categories, independent of explanatory strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// Unsupported native rules or representation.
    Unsupported,
    /// Invalid input, version, seat or revision.
    InvalidInput,
    /// A rules execution failure.
    Rule,
    /// Cooperative cancellation.
    Cancelled,
    /// The execution deadline was reached.
    Deadline,
    /// The bounded advance exhausted its instruction count.
    StepLimit,
    /// Adapter/storage/task failure outside the rules error enum.
    Internal,
}
impl Category {
    /// Stable code for logs/metrics; not automatically added to public packets.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::InvalidInput => "invalid_input",
            Self::Rule => "rule",
            Self::Cancelled => "cancelled",
            Self::Deadline => "deadline",
            Self::StepLimit => "step_limit",
            Self::Internal => "internal",
        }
    }
}
/// Named boundary only: no credentials, checkpoints, prompts or arbitrary payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    /// Trusted content loading.
    Content,
    /// Private checkpoint restoration.
    Checkpoint,
    /// Revision-checked player command.
    Command,
    /// Bounded search/recommendation.
    Search,
    /// Worker request dispatch.
    Worker,
    /// Durable storage or blocking task adapter.
    Storage,
}
/// A classification with bounded context; detailed private state stays with its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// What failed.
    pub category: Category,
    /// Where it failed.
    pub boundary: Boundary,
}
impl EngineError {
    /// Categorize by variant, never by parsing mutable diagnostic text.
    pub const fn category(&self) -> Category {
        match self {
            Self::Unsupported(_) => Category::Unsupported,
            Self::InvalidInput(_) => Category::InvalidInput,
            Self::Rule(_) => Category::Rule,
            Self::Cancelled => Category::Cancelled,
            Self::Deadline => Category::Deadline,
            Self::StepLimit => Category::StepLimit,
        }
    }
    /// Attach safe boundary context without changing the original error/display.
    pub const fn diagnostic(&self, boundary: Boundary) -> Diagnostic {
        Diagnostic {
            category: self.category(),
            boundary,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn categories_ignore_private_text_and_keep_original_display() {
        for (error, category) in [
            (
                EngineError::Unsupported("private-sentinel".into()),
                Category::Unsupported,
            ),
            (
                EngineError::InvalidInput("private-sentinel".into()),
                Category::InvalidInput,
            ),
            (EngineError::Rule("private-sentinel".into()), Category::Rule),
            (EngineError::Cancelled, Category::Cancelled),
            (EngineError::Deadline, Category::Deadline),
            (EngineError::StepLimit, Category::StepLimit),
        ] {
            let before = error.to_string();
            let diagnostic = error.diagnostic(Boundary::Worker);
            assert_eq!(diagnostic.category, category);
            assert!(!format!("{diagnostic:?}").contains("private-sentinel"));
            assert_eq!(error.to_string(), before);
            assert!(!category.code().is_empty());
        }
        assert_eq!(Category::Internal.code(), "internal");
    }
}
