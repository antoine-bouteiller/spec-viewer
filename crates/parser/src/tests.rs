use super::*;
use tree::Element;

fn render(source: &str) -> Parsed {
    parse(source, "fixture.md")
}
fn mdx(source: &str) -> Parsed {
    parse(source, "fixture.mdx")
}
fn elements(nodes: &[Node]) -> Vec<&Element> {
    let mut found = Vec::new();
    for node in nodes {
        match node {
            Node::Element(element) => {
                found.push(element);
                found.extend(elements(&element.children));
            }
            Node::Bridge(children) => found.extend(elements(children)),
            _ => {}
        }
    }
    found
}
fn attrs<'a>(parsed: &'a Parsed, name: &str) -> Vec<&'a str> {
    elements(&parsed.children)
        .into_iter()
        .filter_map(|element| element.attr(name))
        .collect()
}
fn class<'a>(parsed: &'a Parsed, name: &str) -> &'a Element {
    elements(&parsed.children)
        .into_iter()
        .find(|element| element.attr("class") == Some(name))
        .unwrap()
}
fn text(nodes: &[Node]) -> String {
    nodes.iter().map(tree::plain_text).collect()
}

#[test]
fn headings() {
    let result = render("## Repeated heading\n\n## Repeated heading");
    assert_eq!(attrs(&result, "id"), ["repeated-heading", "repeated-heading-2"]);
    let result = mdx("## A <FeatureViewer>nested &amp;</FeatureViewer> end");
    assert_eq!(result.toc[0].text, "A nested & end");
    assert_eq!(result.toc[0].id, "a-nested-end");
}

#[test]
fn gfm_table() {
    let result = render("| Name | Status |\n| --- | --- |\n| T-003 | Done |");
    assert!(elements(&result.children).iter().any(|element| element.tag == "table"));
}
#[test]
fn toc_depth_and_order() {
    assert_eq!(
        render("## First section\n\n### First detail\n\n#### Excluded detail\n\n## Second section").toc,
        vec![
            TocEntry {
                depth: 2,
                id: "first-section".into(),
                text: "First section".into()
            },
            TocEntry {
                depth: 3,
                id: "first-detail".into(),
                text: "First detail".into()
            },
            TocEntry {
                depth: 2,
                id: "second-section".into(),
                text: "Second section".into()
            },
        ]
    );
}
#[test]
fn toc_deduplicates_by_first_document_position() {
    let entries = ["after", "inside", "before", "before", "missing"].map(|id| TocEntry {
        id: id.into(),
        text: id.into(),
        depth: 2,
    });
    let nodes = vec![
        tree::el(
            "section",
            &[("id", "before")],
            vec![Node::Bridge(vec![tree::el("h2", &[("id", "inside")], vec![])])],
        ),
        tree::el("h2", &[("id", "before")], vec![]),
        tree::el("h2", &[("id", "after")], vec![]),
    ];
    let toc = ordered_toc(&entries, &nodes);
    assert_eq!(
        toc.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(),
        ["before", "inside", "after"]
    );
}
#[test]
fn h4_id_without_toc() {
    let result = render("#### Deep section");
    assert_eq!(attrs(&result, "id"), ["deep-section"]);
    assert!(result.toc.is_empty());
}
#[test]
fn bare_xref_chip() {
    let result = render("See VC-001 for acceptance criteria.");
    let chip = class(&result, "xref-chip");
    assert_eq!(chip.attr("data-kind"), Some("vc"));
    assert_eq!(chip.attr("data-ref"), Some("VC-001"));
    assert_eq!(chip.children, vec![tree::text("VC-001")]);
}
#[test]
fn xref_families() {
    let result = render("`[G-1]` `[NG-1]` `[PI-1]` `[KD-1]` `[SO-1]` `[CT-1]` `[VC-1]` `[C-1]` `[OQ-1]` `[EX-1]`");
    assert_eq!(
        attrs(&result, "data-kind"),
        ["g", "ng", "pi", "kd", "so", "ct", "vc", "c", "oq", "ex"]
    );
}
#[test]
fn bracketed_xref() {
    let result = render("See `[KD-3.1]` for details.");
    let chip = class(&result, "xref-chip");
    assert_eq!(chip.attr("data-kind"), Some("kd"));
    assert_eq!(chip.attr("data-ref"), Some("KD-3.1"));
}
#[test]
fn xrefs_protect_code() {
    let result = render("```text\nAC-001\n```\n\n`VC-1` `before [KD-1] after`");
    assert_eq!(attrs(&result, "data-ref"), Vec::<&str>::new());
    assert_eq!(class(&result, "language-text").children, vec![tree::text("AC-001\n")]);
}

#[test]
fn mermaid_fence() {
    let result = render("```mermaid\ngraph TD\n  A --> B\n```");
    assert!(result.has_mermaid);
    let diagram = class(&result, "mermaid");
    assert_eq!(diagram.tag, "pre");
    assert_eq!(diagram.children, vec![tree::text("graph TD\n  A --> B\n")]);
}
#[test]
fn mmd_fence() {
    let result = render("```mmd\ngraph LR\n  A --> B\n```");
    assert!(result.has_mermaid);
    assert_eq!(class(&result, "mermaid").children, vec![tree::text("graph LR\n  A --> B\n")]);
}
#[test]
fn typescript_has_no_mermaid() {
    let result = render("```ts\nconst answer: number = 42\n```");
    assert!(!result.has_mermaid);
    assert_eq!(
        class(&result, "language-ts").children,
        vec![tree::text("const answer: number = 42\n")]
    );
}
#[test]
fn mermaid_state_is_local() {
    render("```mermaid\ngraph TD\n  A --> B\n```");
    assert!(!render("Plain text").has_mermaid);
}
#[test]
fn unknown_language() {
    let result = render("```brainfuck\n+++.\n```");
    assert_eq!(class(&result, "language-brainfuck").children, vec![tree::text("+++.\n")]);
}

#[test]
fn code_tabs() {
    let result = mdx("<CodeTabs><CodeTab label=\"TypeScript\">Tab content</CodeTab></CodeTabs>");
    let panel = class(&result, "md-codetab-panel");
    assert_eq!(panel.attr("data-label"), Some("TypeScript"));
    assert_eq!(text(&panel.children), "TypeScriptTab content");
}
#[test]
fn comparison() {
    let result = mdx("<Comparison><Option name=\"First\">Option content</Option></Comparison>");
    let option = class(&result, "md-comparison-option");
    assert_eq!(option.attr("data-name"), Some("First"));
    assert_eq!(text(&option.children), "FirstOption content");
}
#[test]
fn decision_matrix() {
    let result = mdx("<DecisionMatrix score=\"5\">Matrix content</DecisionMatrix>");
    assert_eq!(class(&result, "md-matrix").tag, "table");
    assert_eq!(attrs(&result, "data-score"), ["5"]);
    assert_eq!(text(&result.children), "Matrix content");
}
#[test]
fn annotated_diff() {
    let result = mdx("<AnnotatedDiff>Diff content<MarginNote line=\"3\">Note content</MarginNote></AnnotatedDiff>");
    assert_eq!(text(&class(&result, "md-annotated-diff-content").children), "Diff content");
    let note = class(&result, "md-margin-note");
    assert_eq!(note.attr("data-line"), Some("3"));
    assert_eq!(text(&note.children), "Note content");
}
#[test]
fn flowchart() {
    let result = mdx("<Flowchart>graph TD\nA --> B<NodeDetail name=\"A\">Node content</NodeDetail></Flowchart>");
    assert!(result.has_mermaid);
    assert_eq!(text(&class(&result, "mermaid").children), "graph TD\nA --> B");
    let detail = class(&result, "md-node-detail");
    assert_eq!(detail.attr("data-name"), Some("A"));
    assert_eq!(text(&detail.children), "ANode content");
}
#[test]
fn step_rail() {
    let result = mdx("<StepRail><Step title=\"First\">Step content</Step></StepRail>");
    assert_eq!(class(&result, "md-steprail").tag, "ol");
    let step = class(&result, "md-step");
    assert_eq!(step.attr("data-title"), Some("First"));
    assert_eq!(text(&step.children), "FirstStep content");
}
#[test]
fn unknown_component() {
    let result = mdx("<FeatureViewer>\n\nFeature content\n\n</FeatureViewer>");
    let component = class(&result, "md-unknown");
    assert_eq!(component.tag, "div");
    assert_eq!(component.attr("data-component"), Some("FeatureViewer"));
    assert_eq!(text(&component.children), "Feature content");
}
#[test]
fn unknown_inline_component() {
    let result = mdx("before <FeatureViewer>child</FeatureViewer> after");
    let component = class(&result, "md-unknown");
    assert_eq!(component.tag, "span");
    assert_eq!(component.attr("data-component"), Some("FeatureViewer"));
    assert_eq!(text(&result.children), "before child after");
}
#[test]
fn component_heading_order() {
    let result = mdx("## Before\n\n<StepRail>\n<Step title=\"First\">\n\n## Repeated\n\n</Step>\n</StepRail>\n\n## Repeated");
    assert_eq!(
        result.toc,
        vec![
            TocEntry {
                depth: 2,
                id: "before".into(),
                text: "Before".into()
            },
            TocEntry {
                depth: 2,
                id: "repeated".into(),
                text: "Repeated".into()
            },
            TocEntry {
                depth: 2,
                id: "repeated-2".into(),
                text: "Repeated".into()
            }
        ]
    );
    assert_eq!(attrs(&result, "id"), ["before", "repeated", "repeated-2"]);
}
#[test]
fn expression_placeholder() {
    let result = mdx("{someExpression}");
    assert_eq!(class(&result, "md-unknown").attr("data-component"), Some("expression"));
    assert_eq!(text(&result.children), "someExpression");
}
#[test]
fn mdx_inline_code_angles() {
    let result = mdx("`List<String>`");
    let code = elements(&result.children)
        .into_iter()
        .find(|element| element.tag == "code")
        .unwrap();
    assert_eq!(code.children, vec![tree::text("List<String>")]);
    assert!(attrs(&result, "data-component").is_empty());
}
#[test]
fn markdown_jsx_stays_raw() {
    let result = render("<StepRail>x</StepRail>");
    assert_eq!(
        result.children,
        vec![tree::el(
            "p",
            &[],
            vec![Node::Raw("<StepRail>".into()), tree::text("x"), Node::Raw("</StepRail>".into())],
        )]
    );
}
#[test]
fn degraded_mdx_keeps_prose() {
    let result = mdx("The prose before the component remains readable.\n\n<StepRail>");
    assert_eq!(result.degraded.as_ref().unwrap().reason, "mdx-parse");
    assert!(matches!(result.children.first(), Some(Node::Element(element))
        if element.attr("class") == Some("md-degraded")));
    assert!(text(&result.children).contains("The prose before the component remains readable."));
}
#[test]
fn mdx_failure_does_not_panic() {
    assert!(std::panic::catch_unwind(|| mdx("Prose\n\n<StepRail>")).is_ok());
}
#[test]
fn panic_fallback_retains_source() {
    let result = parse_with("<script>alert(\"failure\")</script>", "fixture.md", |_, _| {
        panic!("pipeline unavailable")
    });
    assert!(matches!(result.children.first(), Some(Node::Element(element))
        if element.attr("class") == Some("md-degraded")));
    assert!(matches!(result.children.last(), Some(Node::Element(element))
        if element.tag == "pre"
            && element.children == vec![Node::SourceText("<script>alert(\"failure\")</script>".into())]));
    assert_eq!(result.degraded.unwrap().reason, "mdx-parse");
    assert!(!result.has_mermaid);
    assert!(result.toc.is_empty());
}
#[test]
fn relative_doc_links() {
    let result = parse(
        "[leaf](document-render.spec.md) [up](../../other/x.spec.mdx#s) [web](https://example.com/a.md)",
        "plan-viewer/spec/viewer.spec.md",
    );
    assert_eq!(
        attrs(&result, "data-doc"),
        ["plan-viewer/spec/document-render.spec.md", "other/x.spec.mdx"]
    );
}
#[test]
fn unknown_bare_prefix() {
    let result = render("ZZ-001");
    assert_eq!(text(&result.children), "ZZ-001");
    assert!(attrs(&result, "data-ref").is_empty());
}
#[test]
fn bracketed_other_prefix() {
    let result = render("`[FR-2]`");
    let chip = class(&result, "xref-chip");
    assert_eq!(chip.attr("data-kind"), Some("other"));
    assert_eq!(chip.attr("data-ref"), Some("FR-2"));
}

#[test]
fn footnote_boolean_attributes() {
    let result = render("Text[^a] again[^a].\n\n[^a]: Foot *note*");
    let found = elements(&result.children);
    assert_eq!(
        found
            .iter()
            .filter(|element| element
                .attrs
                .iter()
                .any(|(key, value)| key == "data-footnote-ref" && value.is_none()))
            .count(),
        2
    );
    assert_eq!(class(&result, "footnotes").tag, "section");
    assert!(attrs(&result, "id").contains(&"footnotes"));
    let back = found
        .iter()
        .find(|element| element.attr("aria-label") == Some("Back to reference 1-2"))
        .unwrap();
    assert_eq!(back.attr("data-footnote-backref"), Some(""));
    assert_eq!(text(&back.children), "↩2");
}
#[test]
fn crlf_text_preserved() {
    let result = render("First\r\n second\r\n\r\n`one\r\ntwo` and [one\r\ntwo](x.md)");
    assert_eq!(text(&result.children), "First\r\nsecond\none two and one\r\ntwo");
    assert_eq!(attrs(&result, "data-doc"), ["x.md"]);
}

#[test]
fn mdx_closing_tag_diagnostic() {
    let result = mdx("Prose\n\n<StepRail>");
    assert_eq!(
        result.degraded.unwrap(),
        Degraded {
            reason: "mdx-parse".into(),
            message: "Expected a closing tag for `<StepRail>` (3:1-3:11)".into(),
            line: 0
        }
    );
}

#[test]
fn parsed_toc_is_in_document_order() {
    let parsed = parse(
        "## Before\n\n<FeatureViewer>\n\n## Inside\n\n</FeatureViewer>\n\n## After",
        "fixture.mdx",
    );
    assert_eq!(
        parsed.toc.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(),
        vec!["before", "inside", "after"]
    );
}
#[test]
fn final_fallback_tree_has_source_block() {
    let parsed = parse_with("source ' < &", "fixture.md", |_, _| {
        Err(Degraded {
            reason: "mdx-parse".into(),
            message: "failed".into(),
            line: 0,
        })
    });
    assert!(matches!(parsed.children.first(), Some(Node::Element(element))
        if element.attr("class") == Some("md-degraded")));
    assert!(matches!(parsed.children.last(), Some(Node::Element(element))
        if element.tag == "pre" && element.children == vec![Node::SourceText("source ' < &".into())]));
}

#[test]
fn literal_jsx_attributes() {
    let result = mdx("before <a href=\"other.spec.md\" title=\"a &amp; &quot; ' b\" empty disabled data-x={ignored}>VC-1</a> <br /> after");
    let found = elements(&result.children);
    let link = found.iter().find(|element| element.tag == "a").unwrap();
    assert_eq!(link.attr("href"), Some("other.spec.md"));
    assert_eq!(link.attr("title"), Some("a & \" ' b"));
    assert_eq!(link.attrs.len(), 2);
    assert!(attrs(&result, "data-doc").is_empty());
    assert_eq!(attrs(&result, "data-ref"), ["VC-1"]);
}
#[test]
fn esm_and_quoted_expression_placeholders() {
    let result =
        mdx("import Component from 'x'\nexport const config = {\n  value: 1\n}\n\n{someExpression}\n\ninline {\"quoted } and {\"} end");
    assert!(result.degraded.is_none());
    let found = elements(&result.children);
    let module = found
        .iter()
        .find(|element| element.attr("data-component") == Some("module"))
        .unwrap();
    assert_eq!(
        text(&module.children),
        "import Component from 'x'\nexport const config = {\n  value: 1\n}"
    );
    assert!(
        found
            .iter()
            .any(|element| element.attr("data-component") == Some("expression") && text(&element.children) == "\"quoted } and {\"")
    );
}
#[test]
fn component_defaults_and_paragraph_child_groups() {
    for source in [
        "<CodeTabs>\n\n<CodeTab>a</CodeTab> <CodeTab>b</CodeTab>\n\n</CodeTabs>",
        "<CodeTabs><CodeTab>a</CodeTab> <CodeTab>b</CodeTab></CodeTabs>",
    ] {
        let result = mdx(source);
        assert_eq!(attrs(&result, "data-label"), ["Tab 1", "Tab 2"]);
        let panels = elements(&result.children)
            .into_iter()
            .filter(|element| element.attr("class") == Some("md-codetab-panel"))
            .map(|element| text(&element.children))
            .collect::<Vec<_>>();
        assert_eq!(panels, ["Tab 1a", "Tab 2b"]);
    }
    for (source, attr, expected) in [
        ("<Comparison><Option>a</Option></Comparison>", "data-name", "Option 1"),
        ("<StepRail><Step>x</Step></StepRail>", "data-title", "Step 1"),
        ("<DecisionMatrix>x</DecisionMatrix>", "data-score", ""),
        ("<AnnotatedDiff>x<MarginNote>y</MarginNote></AnnotatedDiff>", "data-line", "1"),
        ("<Flowchart>graph TD<NodeDetail>x</NodeDetail></Flowchart>", "data-name", "Node 1"),
    ] {
        assert_eq!(attrs(&mdx(source), attr), [expected]);
    }
}
#[test]
fn gfm_task_and_nested_lists() {
    let result = render("- [x] checked\n- [ ] unchecked\n  - nested\n\n3. ordered\n\n   loose\n\n4. next");
    let found = elements(&result.children);
    let inputs = found.iter().filter(|element| element.tag == "input").collect::<Vec<_>>();
    assert_eq!(inputs.len(), 2);
    assert!(inputs[0].attrs.iter().any(|(key, _)| key == "checked"));
    assert!(!inputs[1].attrs.iter().any(|(key, _)| key == "checked"));
    assert!(inputs.iter().all(|element| element.attrs.iter().any(|(key, _)| key == "disabled")));
    assert_eq!(found.iter().filter(|element| element.tag == "ul").count(), 2);
    assert_eq!(attrs(&result, "start"), ["3"]);
    assert!(text(&result.children).contains("loose"));
}
#[test]
fn table_alignment_and_missing_cells() {
    let result = render("| L | C | R |\n|:--|:-:|--:|\n| a | b | c |\n| d | e |");
    let found = elements(&result.children);
    let headers = found
        .iter()
        .filter(|element| element.tag == "th")
        .map(|element| (element.attr("align").unwrap(), text(&element.children)))
        .collect::<Vec<_>>();
    assert_eq!(headers, [("left", "L".into()), ("center", "C".into()), ("right", "R".into())]);
    let cells = found.iter().filter(|element| element.tag == "td").collect::<Vec<_>>();
    assert_eq!(cells.len(), 6);
    assert_eq!(cells[5].attr("align"), Some("right"));
    assert!(cells[5].children.is_empty());
}
#[test]
fn entities_raw_html_hard_breaks_and_references() {
    let result = render(
        "~~strike~~ ~one~ https://example.com x@y.com www.example.org\n\n<div>\nRaw VC-1\n</div>\n\nInline <b>VC-2</b> &amp; &copy; &lt;\n\nHard  \nbreak\\\n next\n\n![image & title](<x y.png> \"say 'hi'\") [ref][r]\n\n[r]: ../x.spec.md \"Title & quote\"",
    );
    let found = elements(&result.children);
    assert_eq!(found.iter().filter(|element| element.tag == "del").count(), 2);
    assert_eq!(
        attrs(&result, "href"),
        ["https://example.com", "mailto:x@y.com", "http://www.example.org", "../x.spec.md"]
    );
    assert!(result.children.contains(&Node::Raw("<div>\nRaw VC-1\n</div>".into())));
    assert_eq!(found.iter().filter(|element| element.tag == "br").count(), 2);
    assert!(text(&result.children).contains("& © <"));
    let image = found.iter().find(|element| element.tag == "img").unwrap();
    assert_eq!(image.attr("src"), Some("x%20y.png"));
    assert_eq!(image.attr("alt"), Some("image & title"));
    assert_eq!(image.attr("title"), Some("say 'hi'"));
    assert!(
        found
            .iter()
            .any(|element| element.tag == "a" && element.attr("title") == Some("Title & quote"))
    );
}
#[test]
fn xref_word_boundaries_and_limits() {
    let result = render("VC-1 VC-12345 VC-1.2 xVC-2 _VC-3 éVC-4 VC-9x [VC-6] `[ZZ-99.3]` `[KD-1.2.3]`");
    assert_eq!(attrs(&result, "data-ref"), ["VC-1", "VC-1", "VC-4", "VC-6", "ZZ-99.3"]);
    let found = elements(&result.children);
    assert!(
        found
            .iter()
            .any(|element| element.tag == "code" && text(&element.children) == "[KD-1.2.3]")
    );
    assert!(
        found
            .iter()
            .any(|element| element.attr("data-ref") == Some("ZZ-99.3") && element.attr("data-kind") == Some("other"))
    );
}
#[test]
fn doc_links_reject_nonrelative_and_escaping_targets() {
    let result = parse(
        "[abs](/x.md) [escape](../../x.md) [query](x.md?x=1) [anchor](x.mdx#x) [dot](./a/../b.md) [scheme](ftp://a.md) [encoded](x%20y.md)",
        "spec/fixture.md",
    );
    assert_eq!(attrs(&result, "data-doc"), ["spec/x.mdx", "spec/b.md", "spec/x%20y.md"]);
}
#[test]
fn shared_tree_retains_components_code_and_navigation() {
    let parsed = parse(
        "<CodeTabs>\n<CodeTab label=\"TS\">\n\n```ts\nconst value = 1\n```\n\nVC-1 [doc](x.spec.md)\n\n</CodeTab>\n</CodeTabs>",
        "spec/fixture.mdx",
    );
    let found = elements(&parsed.children);
    assert!(
        found
            .iter()
            .any(|element| element.attr("class") == Some("md-codetab-panel") && element.attr("data-label") == Some("TS"))
    );
    assert!(found.iter().any(|element| element.tag == "code"
        && element.attr("class") == Some("language-ts")
        && element.children == vec![tree::Node::Text("const value = 1\n".into())]));
    assert!(
        found
            .iter()
            .any(|element| element.attr("data-ref") == Some("VC-1") && element.attr("data-kind") == Some("vc"))
    );
    assert!(found.iter().any(|element| element.attr("data-doc") == Some("spec/x.spec.md")));
    assert!(
        !found
            .iter()
            .any(|element| element.attr("class").is_some_and(|class| class.starts_with("shiki")))
    );
}
#[test]
fn mdx_soft_breaks_between_child_nodes() {
    let result = mdx("before <FeatureViewer>one\n*two*\nthree\n<ChildViewer>four</ChildViewer>\nfive</FeatureViewer> after");
    assert_eq!(text(&result.children), "before one\n\ntwo\n\nthree\n\nfour\n\nfive after");
    let result = mdx("<FeatureViewer>[one\n*two*\nthree](x.md)\nnext</FeatureViewer>");
    assert_eq!(text(&result.children), "one\ntwo\nthree\n\nnext");
    assert_eq!(attrs(&result, "data-doc"), ["x.md"]);
}

#[test]
fn mdx_trailing_angle_degrades_but_escaped_angle_does_not() {
    let result = mdx("before <");
    assert_eq!(
        result.degraded.unwrap(),
        Degraded {
            reason: "mdx-parse".into(),
            message: "Unexpected end of file before name, expected a character that can start a name, such as a letter, `$`, or `_`".into(),
            line: 1
        }
    );
    for source in ["before \\<", "`<`", "&lt;"] {
        assert!(mdx(source).degraded.is_none());
    }
}
