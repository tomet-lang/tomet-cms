//! Conflict discovery and single-conflict resolution API.
//!
//! Provides inspection of `@conflict` markers in a `.tmt` document and byte-identical
//! resolution of individual conflicts by splicing the chosen side directly into the
//! source text at the marker's span.

use std::fmt;

use tomet::ast::{
    Block, Document, Element, ElementValue, Entry, Inline, List, Placement, Sigil, Span, Value,
};

/// Location/form of a conflict within a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// A standalone block-level conflict element.
    Block,
    /// An inline conflict element within a paragraph or inline run.
    Inline,
    /// A conflict sitting in a value position (e.g. `{data}` map or sequence).
    Value,
}

/// An unresolved conflict detected in document text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// 0-based index of this conflict in document order.
    pub index: usize,
    /// Exact byte range of the `@conflict(...)` element in the source text.
    pub span: Span,
    /// The structural kind of conflict.
    pub kind: ConflictKind,
    /// Rendered content of side A.
    pub a: String,
    /// Rendered content of side B.
    pub b: String,
}

/// Choice of side when resolving a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictSide {
    /// Choose side A.
    A,
    /// Choose side B.
    B,
}

/// Errors returned by the conflict inspection and resolution API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictError {
    /// Failed to parse the document source text.
    ParseError(String),
    /// Conflict index is out of bounds.
    IndexOutOfBounds {
        index: usize,
        count: usize,
    },
    /// A `@conflict` marker element is malformed (e.g. missing `a` or `b` argument).
    InvalidMarker {
        index: usize,
        message: String,
    },
}

impl fmt::Display for ConflictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParseError(msg) => write!(f, "parse error: {msg}"),
            Self::IndexOutOfBounds { index, count } => {
                write!(
                    f,
                    "conflict index {index} out of bounds (found {count} conflicts)"
                )
            }
            Self::InvalidMarker { index, message } => {
                write!(f, "invalid conflict at index {index}: {message}")
            }
        }
    }
}

impl std::error::Error for ConflictError {}

/// Lists all `@conflict` elements in `source` in top-down document order.
pub fn list_conflicts(source: &str) -> Result<Vec<Conflict>, ConflictError> {
    let doc = tomet::parser::parse_document(source)
        .map_err(|e| ConflictError::ParseError(e.to_string()))?;

    let mut conflicts = Vec::new();
    collect_conflicts_in_doc(&doc, &mut conflicts)?;
    Ok(conflicts)
}

/// Resolves a single conflict at `index` to the chosen `side`, returning new text
/// with the `@conflict` element replaced and the rest of the document byte-identical.
pub fn resolve_conflict(
    source: &str,
    index: usize,
    side: ConflictSide,
) -> Result<String, ConflictError> {
    let conflicts = list_conflicts(source)?;
    let count = conflicts.len();
    let conflict = conflicts
        .get(index)
        .ok_or(ConflictError::IndexOutOfBounds { index, count })?;

    let replacement = match side {
        ConflictSide::A => &conflict.a,
        ConflictSide::B => &conflict.b,
    };

    let start = conflict.span.start;
    let end = conflict.span.end;

    if start > end || end > source.len() {
        return Err(ConflictError::InvalidMarker {
            index,
            message: format!("invalid span range {start}..{end} for source of length {}", source.len()),
        });
    }

    let mut out = String::with_capacity(source.len() + replacement.len());
    out.push_str(&source[..start]);
    out.push_str(replacement);
    out.push_str(&source[end..]);

    Ok(out)
}

fn is_conflict_element(el: &Element) -> bool {
    matches!(&el.sigil, Sigil::Named(name) if name.is_bare() && name.name == "conflict")
}

fn extract_sides_from_args(
    el: &Element,
    index: usize,
) -> Result<(&Value, &Value), ConflictError> {
    let Value::Map(entries) = el.args.as_ref().ok_or_else(|| ConflictError::InvalidMarker {
        index,
        message: "missing (args) in @conflict element".to_string(),
    })? else {
        return Err(ConflictError::InvalidMarker {
            index,
            message: "expected map in @conflict (args)".to_string(),
        });
    };

    let a = entries.iter().find(|(k, _)| k == "a").map(|(_, v)| v);
    let b = entries.iter().find(|(k, _)| k == "b").map(|(_, v)| v);

    match (a, b) {
        (Some(a), Some(b)) => Ok((a, b)),
        _ => Err(ConflictError::InvalidMarker {
            index,
            message: "missing 'a' or 'b' side in @conflict".to_string(),
        }),
    }
}

fn blocks_to_text(blocks: &[Block]) -> String {
    let doc = Document {
        blocks: blocks.to_vec(),
        span: Span::default(),
    };
    tomet::printer::document_to_tm(&doc).trim().to_string()
}

fn inlines_to_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) => s.push_str(&t.value),
            Inline::Raw(r) => s.push_str(&r.value),
            Inline::SoftBreak(_) => s.push(' '),
            Inline::LineBreak(_) => s.push('\n'),
            Inline::Element(el) => {
                let doc = Document {
                    blocks: vec![Block::Element(el.clone())],
                    span: Span::default(),
                };
                s.push_str(tomet::printer::document_to_tm(&doc).trim());
            }
        }
    }
    s
}

fn value_to_text(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => Value::float_text(*f),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.clone(),
        Value::Null => "null".to_string(),
        Value::Blocks(blocks) => blocks_to_text(blocks),
        Value::Element(el) => {
            let doc = Document {
                blocks: vec![Block::Element(*el.clone())],
                span: Span::default(),
            };
            tomet::printer::document_to_tm(&doc).trim().to_string()
        }
        other => format!("{other:?}"),
    }
}

fn value_side_to_text(val: &Value, kind: ConflictKind) -> String {
    match (kind, val) {
        (ConflictKind::Block, Value::Blocks(blocks)) => blocks_to_text(blocks),
        (ConflictKind::Inline, Value::Blocks(blocks)) => match blocks.first() {
            Some(Block::Paragraph(p)) => inlines_to_text(&p.content),
            _ => blocks_to_text(blocks),
        },
        _ => value_to_text(val),
    }
}

fn parse_conflict_element(
    el: &Element,
    conflicts: &mut Vec<Conflict>,
) -> Result<(), ConflictError> {
    let index = conflicts.len();
    let (val_a, val_b) = extract_sides_from_args(el, index)?;

    let kind = match (&val_a, &val_b, el.placement) {
        (Value::Blocks(_), Value::Blocks(_), Placement::Inline) => ConflictKind::Inline,
        (Value::Blocks(_), Value::Blocks(_), Placement::Block) => ConflictKind::Block,
        _ => ConflictKind::Value,
    };

    let a = value_side_to_text(val_a, kind);
    let b = value_side_to_text(val_b, kind);

    conflicts.push(Conflict {
        index,
        span: el.span,
        kind,
        a,
        b,
    });

    Ok(())
}

fn collect_conflicts_in_doc(
    doc: &Document,
    conflicts: &mut Vec<Conflict>,
) -> Result<(), ConflictError> {
    collect_conflicts_in_blocks(&doc.blocks, conflicts)
}

fn collect_conflicts_in_blocks(
    blocks: &[Block],
    conflicts: &mut Vec<Conflict>,
) -> Result<(), ConflictError> {
    for block in blocks {
        match block {
            Block::Element(el) => {
                if is_conflict_element(el) {
                    parse_conflict_element(el, conflicts)?;
                } else {
                    collect_conflicts_in_element(el, conflicts)?;
                }
            }
            Block::Paragraph(p) => {
                collect_conflicts_in_inlines(&p.content, conflicts)?;
            }
            Block::Section(s) => {
                collect_conflicts_in_inlines(&s.title, conflicts)?;
                if let Some(args) = &s.args {
                    collect_conflicts_in_value(args, conflicts)?;
                }
                collect_conflicts_in_blocks(&s.blocks, conflicts)?;
                if let Some(val) = &s.value {
                    collect_conflicts_in_element_value(val, conflicts)?;
                }
            }
            Block::List(l) => {
                collect_conflicts_in_list(l, conflicts)?;
            }
        }
    }
    Ok(())
}

fn collect_conflicts_in_element(
    el: &Element,
    conflicts: &mut Vec<Conflict>,
) -> Result<(), ConflictError> {
    if let Some(args) = &el.args {
        collect_conflicts_in_value(args, conflicts)?;
    }
    if let Some(content) = &el.content {
        collect_conflicts_in_blocks(content, conflicts)?;
    }
    if let Some(val) = &el.value {
        collect_conflicts_in_element_value(val, conflicts)?;
    }
    for connect in &el.connects {
        collect_conflicts_in_element(connect, conflicts)?;
    }
    Ok(())
}

fn collect_conflicts_in_inlines(
    inlines: &[Inline],
    conflicts: &mut Vec<Conflict>,
) -> Result<(), ConflictError> {
    for inline in inlines {
        if let Inline::Element(el) = inline {
            if is_conflict_element(el) {
                parse_conflict_element(el, conflicts)?;
            } else {
                collect_conflicts_in_element(el, conflicts)?;
            }
        }
    }
    Ok(())
}

fn collect_conflicts_in_list(
    l: &List,
    conflicts: &mut Vec<Conflict>,
) -> Result<(), ConflictError> {
    for item in &l.items {
        collect_conflicts_in_element(&item.element, conflicts)?;
        if let Some(sub) = &item.sublist {
            collect_conflicts_in_list(sub, conflicts)?;
        }
    }
    Ok(())
}

fn collect_conflicts_in_element_value(
    value: &ElementValue,
    conflicts: &mut Vec<Conflict>,
) -> Result<(), ConflictError> {
    let ElementValue::Group(entries) = value else {
        return Ok(());
    };
    for entry in entries {
        match entry {
            Entry::Pair(_, val) => {
                collect_conflicts_in_value(val, conflicts)?;
            }
            Entry::Element(el) => {
                if is_conflict_element(el) {
                    parse_conflict_element(el, conflicts)?;
                } else {
                    collect_conflicts_in_element(el, conflicts)?;
                }
            }
        }
    }
    Ok(())
}

fn collect_conflicts_in_value(
    value: &Value,
    conflicts: &mut Vec<Conflict>,
) -> Result<(), ConflictError> {
    match value {
        Value::Element(el) => {
            if is_conflict_element(el) {
                parse_conflict_element(el, conflicts)?;
            } else {
                collect_conflicts_in_element(el, conflicts)?;
            }
        }
        Value::Map(entries) => {
            for (_, v) in entries {
                collect_conflicts_in_value(v, conflicts)?;
            }
        }
        Value::Seq(items) => {
            for v in items {
                collect_conflicts_in_value(v, conflicts)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_block_conflict_resolution() {
        let text = "Alpha.\n\n@conflict(\n  a: [\n    Mine.\n  ]\n  b: [\n    Theirs.\n  ]\n)\n\nBeta.\n";
        let conflicts = list_conflicts(text).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::Block);
        assert_eq!(conflicts[0].a, "Mine.");
        assert_eq!(conflicts[0].b, "Theirs.");

        let resolved_a = resolve_conflict(text, 0, ConflictSide::A).unwrap();
        assert_eq!(resolved_a, "Alpha.\n\nMine.\n\nBeta.\n");

        let resolved_b = resolve_conflict(text, 0, ConflictSide::B).unwrap();
        assert_eq!(resolved_b, "Alpha.\n\nTheirs.\n\nBeta.\n");
    }

    #[test]
    fn single_inline_conflict_resolution() {
        let text = "The quick fox @conflict(a: [runs], b: [jumps]) over the dog.\n";
        let conflicts = list_conflicts(text).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::Inline);
        assert_eq!(conflicts[0].a, "runs");
        assert_eq!(conflicts[0].b, "jumps");

        let resolved_a = resolve_conflict(text, 0, ConflictSide::A).unwrap();
        assert_eq!(resolved_a, "The quick fox runs over the dog.\n");

        let resolved_b = resolve_conflict(text, 0, ConflictSide::B).unwrap();
        assert_eq!(resolved_b, "The quick fox jumps over the dog.\n");
    }

    #[test]
    fn conflict_nested_in_data_scalar_sides() {
        let text = "::meta{ title: \"Doc\", count: @conflict(a: 10, b: 20) }\n";
        let conflicts = list_conflicts(text).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::Value);
        assert_eq!(conflicts[0].a, "10");
        assert_eq!(conflicts[0].b, "20");

        let resolved_a = resolve_conflict(text, 0, ConflictSide::A).unwrap();
        assert_eq!(resolved_a, "::meta{ title: \"Doc\", count: 10 }\n");

        let resolved_b = resolve_conflict(text, 0, ConflictSide::B).unwrap();
        assert_eq!(resolved_b, "::meta{ title: \"Doc\", count: 20 }\n");
    }

    #[test]
    fn several_conflicts_resolution_leaves_rest_byte_identical() {
        let text = concat!(
            "// Header comment\n",
            "::meta{ x: @conflict(a: 1, b: 2) }\n\n",
            "Intro paragraph.\n\n",
            "@conflict(\n  a: [\n    Block A.\n  ]\n  b: [\n    Block B.\n  ]\n)\n\n",
            "Outro paragraph with @conflict(a: [inline A], b: [inline B]).\n"
        );

        let conflicts = list_conflicts(text).unwrap();
        assert_eq!(conflicts.len(), 3);
        assert_eq!(conflicts[0].kind, ConflictKind::Value);
        assert_eq!(conflicts[1].kind, ConflictKind::Block);
        assert_eq!(conflicts[2].kind, ConflictKind::Inline);

        // Resolve conflict 1 (Block) first, keeping side B.
        let after_1 = resolve_conflict(text, 1, ConflictSide::B).unwrap();

        // The text around conflict 0 and conflict 2 must remain completely identical
        assert!(after_1.starts_with("// Header comment\n::meta{ x: @conflict(a: 1, b: 2) }\n\nIntro paragraph.\n\nBlock B.\n\nOutro paragraph"));
        assert!(after_1.contains("@conflict(a: [inline A], b: [inline B])"));

        // Re-listing on the updated text now has 2 remaining conflicts
        let remaining = list_conflicts(&after_1).unwrap();
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].kind, ConflictKind::Value);
        assert_eq!(remaining[1].kind, ConflictKind::Inline);

        // Resolve the remaining Value conflict to side A
        let after_2 = resolve_conflict(&after_1, 0, ConflictSide::A).unwrap();
        assert!(after_2.contains("::meta{ x: 1 }"));

        // Resolve the remaining Inline conflict to side B
        let after_3 = resolve_conflict(&after_2, 0, ConflictSide::B).unwrap();
        assert!(after_3.contains("Outro paragraph with inline B.\n"));

        // No conflicts left
        let final_conflicts = list_conflicts(&after_3).unwrap();
        assert!(final_conflicts.is_empty());
    }

    #[test]
    fn index_out_of_bounds() {
        let text = "Plain text without conflicts.\n";
        assert!(list_conflicts(text).unwrap().is_empty());

        let res = resolve_conflict(text, 0, ConflictSide::A);
        assert!(matches!(
            res,
            Err(ConflictError::IndexOutOfBounds { index: 0, count: 0 })
        ));
    }
}
