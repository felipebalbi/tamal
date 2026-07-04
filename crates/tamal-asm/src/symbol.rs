//! The symbol table (`.equ` constants + label addresses) and address passes.

use std::collections::HashMap;

use crate::diagnostics::Span;

/// A resolved symbol value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sym {
    /// A `.equ` constant.
    Equ(i64),
    /// A label's instruction-word address.
    Label(u16),
}

/// `.equ` constants and label addresses, keyed by name (with defining span).
#[derive(Debug, Default, Clone)]
pub struct SymbolTable {
    map: HashMap<String, (Sym, Span)>,
}

impl SymbolTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a symbol value.
    pub fn get(&self, name: &str) -> Option<Sym> {
        self.map.get(name).map(|(s, _)| *s)
    }

    /// Insert a symbol; on a duplicate name, returns `Err(prior_defining_span)`.
    pub fn insert(&mut self, name: &str, sym: Sym, span: Span) -> Result<(), Span> {
        if let Some((_, prev)) = self.map.get(name) {
            return Err(prev.clone());
        }
        self.map.insert(name.to_string(), (sym, span));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_get() {
        let mut t = SymbolTable::new();
        assert!(t.insert("A", Sym::Equ(7), 0..1).is_ok());
        assert!(t.insert("L", Sym::Label(3), 2..3).is_ok());
        assert_eq!(t.get("A"), Some(Sym::Equ(7)));
        assert_eq!(t.get("L"), Some(Sym::Label(3)));
        assert_eq!(t.get("missing"), None);
    }

    #[test]
    fn duplicate_insert_reports_prior_span() {
        let mut t = SymbolTable::new();
        t.insert("A", Sym::Equ(1), 0..1).unwrap();
        assert_eq!(t.insert("A", Sym::Equ(2), 5..6), Err(0..1));
    }
}
