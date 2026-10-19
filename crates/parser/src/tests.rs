use super::*;
fn parse(source: &str, path: &str) -> Document {
    super::parse(source, path, &ParseConfig::default())
}

fn render(source: &str) -> Document {
    parse(source, "fixture.md")
}
fn mdx(source: &str) -> Document {
    parse(source, "fixture.mdx")
}

#[derive(Default)]
struct Nodes<'a> {
    blocks: Vec<&'a Block>,
    inlines: Vec<&'a Inline>,
    components: Vec<&'a Component>,
}
impl<'a> Nodes<'a> {
    fn document(document: &'a Document) -> Self {
        let mut nodes = Self::default();
        nodes.blocks(&document.blocks);
        nodes
    }
    fn blocks(&mut self, blocks: &'a [Block]) {
        for block in blocks {
            self.blocks.push(block);
            match block {
                Block::Paragraph(content) | Block::Heading { content, .. } => self.inlines(content),
                Block::List { items, .. } => {
                    for item in items {
                        self.blocks(&item.blocks);
                    }
                }
                Block::Quote(blocks) => self.blocks(blocks),
                Block::Table { head, rows, .. } => {
                    for cell in head.iter().chain(rows.iter().flatten()) {
                        self.inlines(cell);
                    }
                }
                Block::Component(component) => self.component(component),
                _ => {}
            }
        }
    }
    fn inlines(&mut self, content: &'a [Inline]) {
        for node in content {
            self.inlines.push(node);
            match node {
                Inline::Strong(content) | Inline::Emphasis(content) | Inline::Delete(content) | Inline::Link { content, .. } => {
                    self.inlines(content)
                }
                Inline::Component(component) => self.component(component),
                _ => {}
            }
        }
    }
    fn component(&mut self, component: &'a Component) {
        self.components.push(component);
        match component {
            Component::CodeTabs { tabs, blocks } => {
                for tab in tabs {
                    self.blocks(&tab.body.blocks);
                }
                self.blocks(blocks);
            }
            Component::Comparison { options, blocks } => {
                for option in options {
                    self.blocks(&option.body.blocks);
                }
                self.blocks(blocks);
            }
            Component::DecisionMatrix { blocks, .. } | Component::Unknown { blocks, .. } => self.blocks(blocks),
            Component::AnnotatedDiff { blocks, notes } => {
                self.blocks(blocks);
                for note in notes {
                    self.blocks(&note.body.blocks);
                }
            }
            Component::Flowchart { details, .. } => {
                for detail in details {
                    self.blocks(&detail.body.blocks);
                }
            }
            Component::StepRail { steps, blocks } => {
                for step in steps {
                    self.blocks(&step.body.blocks);
                }
                self.blocks(blocks);
            }
        }
    }
    fn xrefs(&self) -> Vec<(&str, &str)> {
        self.inlines
            .iter()
            .filter_map(|node| {
                if let Inline::Xref { id, family } = node {
                    Some((id.as_str(), family.as_str()))
                } else {
                    None
                }
            })
            .collect()
    }
    fn docs(&self) -> Vec<&str> {
        self.inlines
            .iter()
            .filter_map(|node| {
                if let Inline::Link { doc: Some(doc), .. } = node {
                    Some(doc.as_str())
                } else {
                    None
                }
            })
            .collect()
    }
}

#[test]
fn dialect_components_are_semantic() {
    let document = mdx(include_str!("../../../fixtures/dialect.spec.mdx"));
    let nodes = Nodes::document(&document);
    let components = &nodes.components;
    assert!(matches!(components[0], Component::Unknown { name, .. } if name == "expression"));
    assert!(
        matches!(components[1], Component::CodeTabs { tabs, .. } if tabs.iter().map(|tab| tab.label.as_str()).collect::<Vec<_>>() == ["TypeScript", "JSON"])
    );
    assert!(
        matches!(components[2], Component::Comparison { options, .. } if options.iter().map(|option| option.name.as_str()).collect::<Vec<_>>() == ["Keep source visible", "Enhance in browser"])
    );
    assert!(matches!(components[3], Component::DecisionMatrix { score, .. } if score == "4"));
    assert!(
        matches!(components[4], Component::AnnotatedDiff { notes, .. } if notes.iter().map(|note| note.line.as_str()).collect::<Vec<_>>() == ["1", "2"])
    );
    assert!(
        matches!(components[5], Component::Flowchart { source, details } if source.contains("Draft[Draft] --> Review[Review]") && details.iter().map(|detail| detail.name.as_str()).collect::<Vec<_>>() == ["Draft", "Accepted"])
    );
    assert!(
        matches!(components[6], Component::StepRail { steps, .. } if steps.iter().map(|step| step.title.as_str()).collect::<Vec<_>>() == ["Author", "Render", "Review"])
    );
    assert!(
        matches!(components[7], Component::Unknown { name, blocks } if name == "FeatureViewer" && matches!(blocks[0], Block::Paragraph(_)))
    );
    assert_eq!(components.len(), 8);
    assert_eq!(document.meta.title, "Spec Dialect Fixture");
    assert_eq!(document.meta.status.as_ref().map(|status| status.name.as_str()), Some("accepted"));
}

#[test]
fn dialect_navigation_and_mermaid() {
    // The checked-in dialect has no markdown document link or duplicate heading.
    let source = format!(
        "{}\n\n# Header\n\n# Appendix\n\n## Repeated\n\n## Repeated\n\n[relative](../other.spec.md#s)\n",
        include_str!("../../../fixtures/dialect.spec.mdx")
    );
    let document = parse(&source, "spec/dialect.spec.mdx");
    let nodes = Nodes::document(&document);
    assert!(nodes.xrefs().contains(&("KD-3.1", "kd")));
    assert!(!nodes.xrefs().contains(&("AC-001", "other")));
    assert!(!nodes.xrefs().contains(&("T-010", "other")));
    assert_eq!(nodes.docs(), ["other.spec.md"]);
    assert_eq!(
        nodes.blocks.iter().filter(|block| matches!(block, Block::Mermaid { .. })).count(),
        2
    );
    assert_eq!(
        document
            .toc
            .iter()
            .map(|entry| (entry.depth, entry.id.as_str()))
            .collect::<Vec<_>>(),
        [
            (2, "1-overview"),
            (3, "11-rendered-table"),
            (2, "2-component-dialect"),
            (2, "3-code-and-diagrams"),
            (3, "31-closing-notes"),
            (1, "appendix"),
            (2, "repeated"),
            (2, "repeated-2")
        ]
    );
}

#[test]
fn malformed_mdx_starts_with_degraded_markdown_fallback() {
    let document = mdx("The prose before the component remains readable.\n\n<StepRail>");
    assert_eq!(document.degraded.as_ref().unwrap().reason, "mdx-parse");
    assert_eq!(document.blocks[0], Block::Degraded);
    assert!(
        matches!(&document.blocks[1], Block::Paragraph(content) if content == &[Inline::Text("The prose before the component remains readable.".into())])
    );
    assert_eq!(document.blocks[2], Block::Raw("<StepRail>".into()));
}

#[test]
fn headings_and_toc_depth_order() {
    let document = render("## Repeated heading\n\n## Repeated heading\n\n### Detail\n\n#### Deep");
    assert_eq!(
        document
            .toc
            .iter()
            .map(|entry| (entry.depth, entry.id.as_str()))
            .collect::<Vec<_>>(),
        [(2, "repeated-heading"), (2, "repeated-heading-2"), (3, "detail")]
    );
    assert!(matches!(document.blocks.last(), Some(Block::Heading { depth: 4, id, .. }) if id == "deep"));
    let document = mdx("## A <FeatureViewer>nested &amp;</FeatureViewer> end");
    assert_eq!(document.toc[0].text, "A nested & end");
    assert_eq!(document.toc[0].id, "a-nested-end");
    assert_eq!(render("## 界").toc[0].id, "section");
}

#[test]
fn component_headings_have_ids_and_toc_in_document_order() {
    let document = mdx("## Before\n\n<StepRail>\n<Step title=\"First\">\n\n## Repeated\n\n</Step>\n</StepRail>\n\n## Repeated");
    assert_eq!(
        document.toc.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(),
        ["before", "repeated", "repeated-2"]
    );
    let document = mdx("## Before\n\n<FeatureViewer>\n\n## Inside\n\n</FeatureViewer>\n\n## After");
    assert_eq!(
        document.toc.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(),
        ["before", "inside", "after"]
    );
}

#[test]
fn xref_families_and_bracketed_subreferences() {
    let document = render("VC-001 `[G-1]` `[NG-1]` `[PI-1]` `[KD-3.1]` `[SO-1]` `[CT-1]` `[VC-1]` `[C-1]` `[OQ-1]` `[EX-1]` `[FR-2]`");
    assert_eq!(
        Nodes::document(&document).xrefs(),
        [
            ("VC-001", "vc"),
            ("G-1", "g"),
            ("NG-1", "ng"),
            ("PI-1", "pi"),
            ("KD-3.1", "kd"),
            ("SO-1", "so"),
            ("CT-1", "ct"),
            ("VC-1", "vc"),
            ("C-1", "c"),
            ("OQ-1", "oq"),
            ("EX-1", "ex"),
            ("FR-2", "other")
        ]
    );
}

#[test]
fn xrefs_protect_code_and_unknown_bare_prefixes() {
    let document = render("```text\nAC-001 VC-1\n```\n\n`VC-1` `before [KD-1] after` ZZ-001");
    assert!(Nodes::document(&document).xrefs().is_empty());
    assert_eq!(
        document.blocks[0],
        Block::Code {
            lang: Some("text".into()),
            source: "AC-001 VC-1\n".into()
        }
    );
}

#[test]
fn xref_word_boundaries_and_limits() {
    let document = render("VC-1 VC-12345 VC-1.2 xVC-2 _VC-3 éVC-4 VC-9x [VC-6] `[ZZ-99.3]` `[KD-1.2.3]`");
    let nodes = Nodes::document(&document);
    assert_eq!(
        nodes.xrefs(),
        [("VC-1", "vc"), ("VC-1", "vc"), ("VC-4", "vc"), ("VC-6", "vc"), ("ZZ-99.3", "other")]
    );
    assert!(nodes.inlines.contains(&&Inline::Code("[KD-1.2.3]".into())));
}

#[test]
fn mermaid_and_mmd_keep_source_and_other_code_keeps_language() {
    for lang in ["mermaid", "mmd"] {
        assert_eq!(
            render(&format!("```{lang}\ngraph TD\n  A --> B\n```")).blocks,
            [Block::Mermaid {
                source: "graph TD\n  A --> B\n".into()
            }]
        );
    }
    for lang in ["ts", "brainfuck"] {
        assert_eq!(
            render(&format!("```{lang}\n+++\n```")).blocks,
            [Block::Code {
                lang: Some(lang.into()),
                source: "+++\n".into()
            }]
        );
    }
    assert_eq!(
        render("```\n```").blocks,
        [Block::Code {
            lang: None,
            source: String::new()
        }]
    );
    assert_eq!(
        render("Plain text").blocks,
        [Block::Paragraph(vec![Inline::Text("Plain text".into())])]
    );
}

#[test]
fn component_defaults_and_paragraph_child_groups() {
    for source in [
        "<CodeTabs>\n\n<CodeTab>a</CodeTab> <CodeTab>b</CodeTab>\n\n</CodeTabs>",
        "<CodeTabs><CodeTab>a</CodeTab> <CodeTab>b</CodeTab></CodeTabs>",
    ] {
        let document = mdx(source);
        assert!(
            matches!(Nodes::document(&document).components[0], Component::CodeTabs { tabs, .. } if tabs.iter().map(|tab| tab.label.as_str()).collect::<Vec<_>>() == ["Tab 1", "Tab 2"])
        );
    }
    let cases = [
        "<Comparison><Option>a</Option></Comparison>",
        "<StepRail><Step>x</Step></StepRail>",
        "<DecisionMatrix>x</DecisionMatrix>",
        "<AnnotatedDiff>x<MarginNote>y</MarginNote></AnnotatedDiff>",
        "<Flowchart>graph TD<NodeDetail>x</NodeDetail></Flowchart>",
    ];
    for (index, source) in cases.into_iter().enumerate() {
        let document = mdx(source);
        let nodes = Nodes::document(&document);
        match (index, nodes.components[0]) {
            (0, Component::Comparison { options, .. }) => assert_eq!(options[0].name, "Option 1"),
            (1, Component::StepRail { steps, .. }) => assert_eq!(steps[0].title, "Step 1"),
            (2, Component::DecisionMatrix { score, .. }) => assert!(score.is_empty()),
            (3, Component::AnnotatedDiff { notes, .. }) => assert_eq!(notes[0].line, "1"),
            (4, Component::Flowchart { details, .. }) => assert_eq!(details[0].name, "Node 1"),
            _ => panic!("wrong component: {:?}", nodes.components),
        }
    }
}

#[test]
fn unknown_components_preserve_inline_and_block_placement() {
    let document = mdx("before <FeatureViewer>child</FeatureViewer> after");
    assert!(
        matches!(&document.blocks[0], Block::Paragraph(content) if matches!(&content[1], Inline::Component(component) if matches!(component.as_ref(), Component::Unknown { name, .. } if name == "FeatureViewer")))
    );
    let document = mdx("<FeatureViewer>\n\nFeature content\n\n</FeatureViewer>");
    assert!(
        matches!(&document.blocks[0], Block::Component(Component::Unknown { name, blocks }) if name == "FeatureViewer" && blocks == &[Block::Paragraph(vec![Inline::Text("Feature content".into())]) ])
    );
}

#[test]
fn esm_and_quoted_expression_placeholders() {
    let document =
        mdx("import Component from 'x'\nexport const config = {\n  value: 1\n}\n\n{someExpression}\n\ninline {\"quoted } and {\"} end");
    assert!(document.degraded.is_none());
    let nodes = Nodes::document(&document);
    assert!(
        matches!(nodes.components[0], Component::Unknown { name, blocks } if name == "module" && blocks == &[Block::Paragraph(vec![Inline::Text("import Component from 'x'\nexport const config = {\n  value: 1\n}".into())])])
    );
    assert!(nodes.components.iter().any(|component| matches!(component, Component::Unknown { name, blocks } if name == "expression" && blocks == &[Block::Paragraph(vec![Inline::Text("\"quoted } and {\"".into())])])));
}

#[test]
fn mdx_inline_code_angles_are_not_components() {
    let document = mdx("`List<String>`");
    assert_eq!(document.blocks, [Block::Paragraph(vec![Inline::Code("List<String>".into())])]);
    for source in ["before <", "before \\<", "&lt;"] {
        let document = mdx(source);
        assert!(document.degraded.is_none());
        let expected = if source == "&lt;" { "<" } else { "before <" };
        assert_eq!(document.blocks, [Block::Paragraph(vec![Inline::Text(expected.into())])]);
    }
}

#[test]
fn markdown_jsx_stays_raw() {
    assert_eq!(
        render("<StepRail>x</StepRail>").blocks,
        [Block::Paragraph(vec![
            Inline::Raw("<StepRail>".into()),
            Inline::Text("x".into()),
            Inline::Raw("</StepRail>".into())
        ])]
    );
}

#[test]
fn panic_and_final_fallback_retain_source() {
    for panic in [false, true] {
        let document = parse_with("source ' < &", "fixture.md", |_, _| {
            if panic {
                panic!("pipeline unavailable");
            }
            Err(Degraded {
                reason: "mdx-parse".into(),
                message: "failed".into(),
                line: 0,
            })
        });
        assert_eq!(
            document.blocks,
            [
                Block::Degraded,
                Block::Code {
                    lang: None,
                    source: "source ' < &".into()
                }
            ]
        );
        assert_eq!(document.degraded.unwrap().reason, "mdx-parse");
        assert!(document.toc.is_empty());
    }
}

#[test]
fn mdx_failure_keeps_native_message_and_location() {
    let document = mdx("Prose\n\n<StepRail>");
    let error = document.degraded.unwrap();
    assert_eq!(error.reason, "mdx-parse");
    assert!(error.message.contains("Expected a closing tag for `<StepRail>`"));
    assert_eq!(error.line, 3);
    assert!(std::panic::catch_unwind(|| mdx("Prose\n\n<StepRail>")).is_ok());
}

#[test]
fn relative_doc_links_resolve_and_reject_escaping_targets() {
    let document = parse(
        "[leaf](document-render.spec.md) [up](../../other/x.spec.mdx#s) [web](https://example.com/a.md)",
        "spec-viewer/spec/viewer.spec.md",
    );
    assert_eq!(
        Nodes::document(&document).docs(),
        ["spec-viewer/spec/document-render.spec.md", "other/x.spec.mdx"]
    );
    let document = parse(
        "[abs](/x.md) [escape](../../x.md) [query](x.md?x=1) [anchor](x.mdx#x) [dot](./a/../b.md) [scheme](ftp://a.md) [encoded](x%20y.md)",
        "spec/fixture.md",
    );
    assert_eq!(Nodes::document(&document).docs(), ["spec/x.mdx", "spec/b.md", "spec/x%20y.md"]);
}

#[test]
fn crlf_text_and_link_line_endings_are_preserved() {
    let document = render("First\r\n second\r\n\r\n`one\r\ntwo` and [one\r\ntwo](x.md)");
    assert_eq!(document.blocks[0], Block::Paragraph(vec![Inline::Text("First\r\nsecond".into())]));
    assert_eq!(
        document.blocks[1],
        Block::Paragraph(vec![
            Inline::Code("one two".into()),
            Inline::Text(" and ".into()),
            Inline::Link {
                href: "x.md".into(),
                doc: Some("x.md".into()),
                content: vec![Inline::Text("one\r\ntwo".into())]
            }
        ])
    );
}

#[test]
fn literal_jsx_links_and_styles_keep_children_without_document_resolution() {
    let document = mdx("before <a href=\"other.spec.md\" title=\"ignored\">VC-1</a> <br /> after");
    let nodes = Nodes::document(&document);
    assert!(nodes.docs().is_empty());
    assert_eq!(nodes.xrefs(), [("VC-1", "vc")]);
    assert!(
        nodes
            .inlines
            .iter()
            .any(|node| matches!(node, Inline::Link { href, .. } if href == "other.spec.md"))
    );
    assert!(nodes.inlines.contains(&&Inline::Break));
}

#[test]
fn gfm_task_and_nested_lists_keep_checked_start_and_looseness() {
    let document = render("- [x] checked\n- [ ] unchecked\n  - nested\n\n3. ordered\n\n   loose\n\n4. next");
    assert!(
        matches!(&document.blocks[0], Block::List { ordered: false, loose: false, items, .. } if items[0].checked == Some(true) && items[1].checked == Some(false) && matches!(items[1].blocks[1], Block::List { .. }))
    );
    assert!(
        matches!(&document.blocks[1], Block::List { ordered: true, start: Some(3), loose: true, items } if items[0].blocks.len() == 2 && items[1].blocks.len() == 1)
    );
}

#[test]
fn table_alignment_and_missing_cells() {
    let document = render("| L | C | R |\n|:--|:-:|--:|\n| a | b | c |\n| d | e |");
    let Block::Table { align, head, rows } = &document.blocks[0] else {
        panic!("missing table");
    };
    assert_eq!(align, &[Align::Left, Align::Center, Align::Right]);
    assert_eq!(
        head,
        &[
            vec![Inline::Text("L".into())],
            vec![Inline::Text("C".into())],
            vec![Inline::Text("R".into())]
        ]
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].len(), 3);
    assert!(rows[1][2].is_empty());
}

#[test]
fn entities_raw_html_hard_breaks_references_and_images() {
    let document = render(
        "~~strike~~ ~one~ https://example.com x@y.com www.example.org\n\n<div>\nRaw VC-1\n</div>\n\nInline <b>VC-2</b> &amp; &copy; &lt;\n\nHard  \nbreak\\\n next\n\n![image & title](<x y.png> \"say 'hi'\") [ref][r]\n\n[r]: ../x.spec.md \"Title & quote\"",
    );
    let nodes = Nodes::document(&document);
    assert_eq!(nodes.inlines.iter().filter(|node| matches!(node, Inline::Delete(_))).count(), 2);
    assert_eq!(
        nodes
            .inlines
            .iter()
            .filter_map(|node| if let Inline::Link { href, .. } = node {
                Some(href.as_str())
            } else {
                None
            })
            .collect::<Vec<_>>(),
        ["https://example.com", "mailto:x@y.com", "http://www.example.org", "../x.spec.md"]
    );
    assert!(document.blocks.contains(&Block::Raw("<div>\nRaw VC-1\n</div>".into())));
    assert_eq!(nodes.inlines.iter().filter(|node| matches!(node, Inline::Break)).count(), 2);
    assert!(nodes.inlines.contains(&&Inline::Image {
        alt: "image & title".into(),
        src: "x%20y.png".into()
    }));
    assert!(nodes.inlines.contains(&&Inline::Text(" & © <".into())));
}

#[test]
fn component_bodies_keep_code_and_navigation() {
    let document = parse(
        "<CodeTabs>\n<CodeTab label=\"TS\">\n\n```ts\nconst value = 1\n```\n\nVC-1 [doc](x.spec.md)\n\n</CodeTab>\n</CodeTabs>",
        "spec/fixture.mdx",
    );
    let nodes = Nodes::document(&document);
    assert!(matches!(nodes.components[0], Component::CodeTabs { tabs, .. } if tabs[0].label == "TS"));
    assert!(nodes.blocks.contains(&&Block::Code {
        lang: Some("ts".into()),
        source: "const value = 1\n".into()
    }));
    assert_eq!(nodes.xrefs(), [("VC-1", "vc")]);
    assert_eq!(nodes.docs(), ["spec/x.spec.md"]);
}

#[test]
fn mdx_soft_breaks_between_child_nodes_are_repaired() {
    let document = mdx("before <FeatureViewer>one\n*two*\nthree\n<ChildViewer>four</ChildViewer>\nfive</FeatureViewer> after");
    let nodes = Nodes::document(&document);
    assert!(nodes.inlines.contains(&&Inline::Text("one\n".into())));
    assert!(nodes.inlines.contains(&&Inline::Text("\nthree\n".into())));
    assert!(nodes.inlines.contains(&&Inline::Text("\nfive".into())));
    let document = mdx("<FeatureViewer>[one\n*two*\nthree](x.md)\nnext</FeatureViewer>");
    let nodes = Nodes::document(&document);
    assert_eq!(nodes.docs(), ["x.md"]);
    assert!(nodes.inlines.contains(&&Inline::Text("\nthree".into())));
}

#[test]
fn lexical_boundary_handles_nested_quotes_comments_and_incomplete_input() {
    for value in [
        "({ value: ['}'] })",
        "\"quoted } and {\"",
        "`text ${value}`",
        "/* } */ ({})",
        "// }\n({})",
    ] {
        assert!(matches!(lexical_boundary(value), markdown::MdxSignal::Ok), "{value}");
    }
    for value in ["({", "'open", "/* open", "[1"] {
        assert!(matches!(lexical_boundary(value), markdown::MdxSignal::Eof(..)), "{value}");
    }
}

#[test]
fn footnotes_keep_visible_numbers_heading_and_return_labels() {
    let document = render("Text[^a] again[^a].\n\n[^a]: Foot *note*");
    let nodes = Nodes::document(&document);
    let labels = nodes
        .inlines
        .iter()
        .filter_map(|node| {
            if let Inline::Link { href, content, .. } = node {
                assert!(href.is_empty());
                Some(plugins::inline_text(content))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(labels, ["1", "1", "↩", "↩2"]);
    assert_eq!(document.toc[0].text, "Footnotes");
}

#[test]
fn vc_5_configured_chips_follow_order_and_first_present_section() {
    use spec_viewer_config::{ChipConfig, ChipKind};
    let config = ParseConfig {
        statuses: vec![],
        chips: vec![
            ChipConfig {
                label: "milestones".into(),
                kind: ChipKind::Headings,
                sections: vec!["missing".into(), "RISKS".into(), "steps".into()],
            },
            ChipConfig {
                label: "R".into(),
                kind: ChipKind::Checklist,
                sections: vec!["risks".into()],
            },
            ChipConfig {
                label: "empty".into(),
                kind: ChipKind::Checklist,
                sections: vec!["implementation".into(), "steps".into()],
            },
        ],
    };
    let document = super::parse(
        "## Steps\n### Not selected\n- [x] task\n## Risks\n### First\n- [x] identified\n### Second\n- [ ] pending\n## Implementation\n",
        ".plan/risks.md",
        &config,
    );
    assert_eq!(
        document.meta.chips,
        [
            Chip {
                label: "milestones".into(),
                value: ChipValue::Count(2)
            },
            Chip {
                label: "R".into(),
                value: ChipValue::Checklist { done: 1, total: 2 }
            },
            Chip {
                label: "empty".into(),
                value: ChipValue::Checklist { done: 0, total: 0 }
            },
        ]
    );
}
