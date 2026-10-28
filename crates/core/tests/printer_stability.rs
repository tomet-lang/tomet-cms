//! Tests evaluating round-trip stability of `tomet::printer::document_to_tm`
//! and AST three-way merging.
//!
//! Captures current printer formatting characteristics and divergences when
//! documents are re-serialized from AST during a merge.

use tomet::parser::parse_document;
use tomet::printer::document_to_tm;

#[test]
fn roundtrip_headings_and_sections() {
    let input = concat!(
        "=[ Main Heading ]\n\n",
        "Introductory paragraph under main heading.\n\n",
        "==[ Sub Heading ]\n\n",
        "Body paragraph under sub heading.\n"
    );
    let doc = parse_document(input).unwrap();
    let output = document_to_tm(&doc);

    // Divergence: Heading brackets lose inner padding (`=[Main Heading]`),
    // and blank line between heading and its first child block is dropped.
    let expected_current_output = concat!(
        "=[Main Heading]\n",
        "Introductory paragraph under main heading.\n",
        "==[Sub Heading]\n",
        "Body paragraph under sub heading.\n"
    );
    assert_eq!(output, expected_current_output);
}

#[test]
fn roundtrip_paragraphs_and_inlines() {
    let input = concat!(
        "This is a paragraph with *italic*, **bold**, and `inline code`.\n\n",
        "Second paragraph with a link: @link(target: \"https://example.com\")[Example].\n"
    );
    let doc = parse_document(input).unwrap();
    let output = document_to_tm(&doc);

    // Divergence: Markdown inline formatting sugars (`*em*`, `**strong**`)
    // are desugared at parse time and printed as explicit `@em[...]` and `@strong[...]`.
    let expected_current_output = concat!(
        "This is a paragraph with @em[italic], @strong[bold], and `inline code`.\n\n",
        "Second paragraph with a link: @link(target: \"https://example.com\")[Example].\n"
    );
    assert_eq!(output, expected_current_output);
}

#[test]
fn roundtrip_nested_elements() {
    let input = concat!(
        "@callout(type: \"info\")[\n",
        "  Important note inside callout.\n",
        "]\n"
    );
    let doc = parse_document(input).unwrap();
    let output = document_to_tm(&doc);

    // Divergence: Block-indented content is flattened into single line brackets,
    // and quotes on bare string arguments are stripped (`type: info`).
    let expected_current_output = "@callout(type: info)[Important note inside callout.]\n";
    assert_eq!(output, expected_current_output);
}

#[test]
fn roundtrip_meta_and_data() {
    let input = "::meta{ title: \"Document Title\", count: 42 }\n";
    let doc = parse_document(input).unwrap();
    let output = document_to_tm(&doc);

    // Divergence: Inner spaces inside `{ ... }` are removed.
    let expected_current_output = "::meta{title: \"Document Title\", count: 42}\n";
    assert_eq!(output, expected_current_output);
}

#[test]
fn roundtrip_comments() {
    let input = concat!(
        "// A leading comment\n",
        "Paragraph after comment.\n"
    );
    let doc = parse_document(input).unwrap();
    let output = document_to_tm(&doc);

    // Divergence: Comments are completely stripped because AST does not retain them.
    assert_eq!(output, "Paragraph after comment.\n");
}

#[test]
fn roundtrip_fenced_code() {
    let input = concat!(
        "```rust\n",
        "fn main() {\n",
        "    println!(\"hello\");\n",
        "}\n",
        "```\n"
    );
    let doc = parse_document(input).unwrap();
    let output = document_to_tm(&doc);

    // Stable: Fenced code blocks are preserved verbatim.
    assert_eq!(output, input);
}

#[test]
fn merge_disjoint_edits_roundtrip() {
    let base_text = concat!(
        "=[ Title ]\n\n",
        "First section content.\n\n",
        "==[ Section Two ]\n\n",
        "Second section content.\n"
    );
    let local_text = concat!(
        "=[ Title ]\n\n",
        "First section content edited locally.\n\n",
        "==[ Section Two ]\n\n",
        "Second section content.\n"
    );
    let remote_text = concat!(
        "=[ Title ]\n\n",
        "First section content.\n\n",
        "==[ Section Two ]\n\n",
        "Second section content edited remotely.\n"
    );

    let base_doc = parse_document(base_text).unwrap();
    let local_doc = parse_document(local_text).unwrap();
    let remote_doc = parse_document(remote_text).unwrap();

    let res = tomet::merge::merge(&base_doc, &local_doc, &remote_doc);
    assert!(res.clean);

    let merged_text = document_to_tm(&res.document);

    // The merge combines disjoint edits accurately, with the aforementioned
    // heading formatting applied to all sections during serialization.
    let expected = concat!(
        "=[Title]\n",
        "First section content edited locally.\n",
        "==[Section Two]\n",
        "Second section content edited remotely.\n"
    );
    assert_eq!(merged_text, expected);
}
