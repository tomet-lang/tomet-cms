//! Tests verifying byte-for-byte round-trip and merge stability of `tomet::edit`
//! and `tomet::merge::merge`.
//!
//! Ensures that untouched headings, blank lines, inline sugar (`*em*`, `**strong**`),
//! comments (`//`, `/* */`), multi-line elements, `{ ... }` brace padding, and
//! fenced code blocks are preserved verbatim during edits and three-way merges.

use tomet::edit::{export_doc, import_source};
use tomet::merge::merge;

#[test]
fn roundtrip_headings_and_sections() {
    let input = concat!(
        "=[ Main Heading ]\n\n",
        "Introductory paragraph under main heading.\n\n",
        "==[ Sub Heading ]\n\n",
        "Body paragraph under sub heading.\n"
    );
    let output = export_doc(&import_source(input));
    assert_eq!(output, input);
    assert_eq!(merge(input, input, input).text, input);
}

#[test]
fn roundtrip_paragraphs_and_inlines() {
    let input = concat!(
        "This is a paragraph with *italic*, **bold**, and `inline code`.\n\n",
        "Second paragraph with a link: @link(target: \"https://example.com\")[Example].\n"
    );
    let output = export_doc(&import_source(input));
    assert_eq!(output, input);
    assert_eq!(merge(input, input, input).text, input);
}

#[test]
fn roundtrip_nested_elements() {
    let input = concat!(
        "@callout(type: \"info\")[\n",
        "  Important note inside callout.\n",
        "]\n"
    );
    let output = export_doc(&import_source(input));
    assert_eq!(output, input);
    assert_eq!(merge(input, input, input).text, input);
}

#[test]
fn roundtrip_meta_and_data() {
    let input = "::meta{ title: \"Document Title\", count: 42 }\n";
    let output = export_doc(&import_source(input));
    assert_eq!(output, input);
    assert_eq!(merge(input, input, input).text, input);
}

#[test]
fn roundtrip_comments() {
    let input = concat!(
        "// A leading comment\n",
        "/* A block comment */\n",
        "Paragraph after comment.\n"
    );
    let output = export_doc(&import_source(input));
    assert_eq!(output, input);
    assert_eq!(merge(input, input, input).text, input);
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
    let output = export_doc(&import_source(input));
    assert_eq!(output, input);
    assert_eq!(merge(input, input, input).text, input);
}

#[test]
fn merge_disjoint_edits_preserves_all_untouched_formatting_byte_for_byte() {
    let base_text = concat!(
        "::meta{ title: \"Doc\", count: 1 }\n\n",
        "=[ Title ]\n\n",
        "// Comment before first section\n",
        "First section with *italic* and **bold**.\n\n",
        "@callout(type: \"info\")[\n",
        "  Multi-line callout content.\n",
        "]\n\n",
        "==[ Section Two ]\n\n",
        "Second section content.\n"
    );
    let local_text = base_text.replace(
        "First section with *italic* and **bold**.",
        "First section edited locally with *italic* and **bold**.",
    );
    let remote_text = base_text.replace(
        "Second section content.",
        "Second section content edited remotely.",
    );

    let res = merge(base_text, &local_text, &remote_text);
    assert!(res.clean);

    let expected = concat!(
        "::meta{ title: \"Doc\", count: 1 }\n\n",
        "=[ Title ]\n\n",
        "// Comment before first section\n",
        "First section edited locally with *italic* and **bold**.\n\n",
        "@callout(type: \"info\")[\n",
        "  Multi-line callout content.\n",
        "]\n\n",
        "==[ Section Two ]\n\n",
        "Second section content edited remotely.\n"
    );
    assert_eq!(res.text, expected);
}
