//! The symbol table (`.equ` constants + label addresses) and address passes.

use std::collections::HashMap;

use crate::diagnostics::{Diagnostic, Span};
use crate::encoder;
use crate::parser::{Line, LineKind, OperandKind};

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

/// Pass 0: evaluate `.equ` directives in source order (value = number or an
/// already-defined symbol). Collects every problem rather than failing fast.
#[allow(dead_code)]
pub(crate) fn collect_equs(lines: &[Line]) -> (SymbolTable, Vec<Diagnostic>) {
    let mut syms = SymbolTable::new();
    let mut diags = Vec::new();
    for line in lines {
        let LineKind::Directive { name, args } = &line.kind else {
            continue;
        };
        if name != "equ" {
            continue;
        }
        if args.len() != 2 {
            diags.push(Diagnostic::error(
                line.span.clone(),
                "`.equ` takes NAME, VALUE",
            ));
            continue;
        }
        let sym_name = match &args[0].kind {
            OperandKind::Ident(s) => s.clone(),
            OperandKind::Num(_) => {
                diags.push(Diagnostic::error(
                    args[0].span.clone(),
                    "`.equ` name must be an identifier",
                ));
                continue;
            }
        };
        match encoder::resolve_imm(&args[1], &syms) {
            Ok(v) => {
                if let Err(prev) = syms.insert(&sym_name, Sym::Equ(v), args[0].span.clone()) {
                    diags.push(
                        Diagnostic::error(
                            args[0].span.clone(),
                            format!("duplicate symbol `{sym_name}`"),
                        )
                        .with_label(prev, "first defined here"),
                    );
                }
            }
            Err(d) => diags.push(d),
        }
    }
    (syms, diags)
}

/// Pass 1: assign each label the address of the next instruction word, summing
/// per-instruction word counts. Returns the total word count and any duplicate
/// diagnostics.
#[allow(dead_code)]
pub(crate) fn assign_addresses(lines: &[Line], syms: &mut SymbolTable) -> (usize, Vec<Diagnostic>) {
    let mut addr: usize = 0;
    let mut diags = Vec::new();
    for line in lines {
        match &line.kind {
            LineKind::Label(name) => {
                if let Err(prev) = syms.insert(name, Sym::Label(addr as u16), line.span.clone()) {
                    diags.push(
                        Diagnostic::error(line.span.clone(), format!("duplicate symbol `{name}`"))
                            .with_label(prev, "first defined here"),
                    );
                }
            }
            LineKind::Instr { mnemonic, operands } => {
                addr += usize::from(encoder::instr_word_count(mnemonic, operands, syms));
            }
            LineKind::Directive { .. } => {}
        }
    }
    (addr, diags)
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

    use crate::lexer::lex;
    use crate::parser::parse;

    fn lines(src: &str) -> Vec<crate::parser::Line> {
        parse(&lex(src).unwrap()).unwrap()
    }

    #[test]
    fn collect_equs_resolves_in_order() {
        let (t, d) = collect_equs(&lines(".equ A, 0x44\n.equ B, A\n"));
        assert!(d.is_empty());
        assert_eq!(t.get("A"), Some(Sym::Equ(0x44)));
        assert_eq!(t.get("B"), Some(Sym::Equ(0x44)));
    }

    #[test]
    fn collect_equs_reports_duplicate_and_bad_arity() {
        let (_t, d) = collect_equs(&lines(".equ A, 1\n.equ A, 2\n"));
        assert!(d.iter().any(|x| x.message.contains("duplicate")));
        let (_t2, d2) = collect_equs(&lines(".equ A\n"));
        assert!(d2.iter().any(|x| x.message.contains("NAME, VALUE")));
    }

    #[test]
    fn addresses_account_for_li_expansion() {
        // a:0 ; li x5,0x1800 (3 words: 0,1,2) ; b:3 ; cs_assert (3) ; c:4
        let ls = lines("a:\n  li x5, 0x1800\nb:\n  cs_assert\nc:\n");
        let (mut t, d0) = collect_equs(&ls);
        assert!(d0.is_empty());
        let (total, d) = assign_addresses(&ls, &mut t);
        assert!(d.is_empty());
        assert_eq!(t.get("a"), Some(Sym::Label(0)));
        assert_eq!(t.get("b"), Some(Sym::Label(3)));
        assert_eq!(t.get("c"), Some(Sym::Label(4)));
        assert_eq!(total, 4);
    }

    #[test]
    fn duplicate_label_is_reported() {
        let ls = lines("a:\n  cs_assert\na:\n");
        let (mut t, _) = collect_equs(&ls);
        let (_total, d) = assign_addresses(&ls, &mut t);
        assert!(d.iter().any(|x| x.message.contains("duplicate")));
    }
}
