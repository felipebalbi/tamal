//! Structured, rendering-agnostic diagnostics (spans as byte ranges).

/// A byte-offset span into the assembler source.
pub type Span = core::ops::Range<usize>;

/// Diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A hard error; assembly fails.
    Error,
    /// A non-fatal warning.
    Warning,
}

/// A structured, rendering-agnostic diagnostic. The CLI turns each into an
/// `ariadne::Report`; the library never depends on a renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Severity.
    pub severity: Severity,
    /// The primary human-readable message.
    pub message: String,
    /// The primary source span the message points at.
    pub primary: Span,
    /// Secondary labelled spans (span + note).
    pub labels: Vec<(Span, String)>,
    /// An optional help/hint line.
    pub help: Option<String>,
}

impl Diagnostic {
    /// An error diagnostic anchored at `primary`.
    pub fn error(primary: Span, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            primary,
            labels: Vec::new(),
            help: None,
        }
    }

    /// Attach a secondary labelled span.
    #[must_use]
    pub fn with_label(mut self, span: Span, text: impl Into<String>) -> Self {
        self.labels.push((span, text.into()));
        self
    }

    /// Attach a help/hint line.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_sets_fields() {
        let d = Diagnostic::error(3..7, "bad thing")
            .with_label(3..7, "here")
            .with_help("try that");
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, "bad thing");
        assert_eq!(d.primary, 3..7);
        assert_eq!(d.labels, vec![(3..7, "here".to_string())]);
        assert_eq!(d.help.as_deref(), Some("try that"));
    }
}
