use super::*;

fn render(source: &str) -> RenderResult {
    to_html(&parse(source, "fixture.md"))
}
fn mdx(source: &str) -> RenderResult {
    to_html(&parse(source, "fixture.mdx"))
}

#[test]
fn headings() {
    let result = render("## Repeated heading\n\n## Repeated heading");
    assert!(result.html.contains("id=\"repeated-heading\""));
    assert!(result.html.contains("id=\"repeated-heading-2\""));
}

#[test]
fn gfm_table() {
    assert!(
        render("| Name | Status |\n| --- | --- |\n| T-003 | Done |")
            .html
            .contains("<table>")
    );
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
fn h4_id_without_toc() {
    let result = render("#### Deep section");
    assert!(result.html.contains("id=\"deep-section\""));
    assert!(result.toc.is_empty());
}
#[test]
fn bare_xref_chip() {
    assert!(render("See VC-001 for acceptance criteria.").html.contains(
        "<button class=\"xref-chip\" data-kind=\"vc\" data-ref=\"VC-001\" title=\"Copy VC-001\" type=\"button\">VC-001</button>"
    ));
}
#[test]
fn xref_families() {
    let html = render("`[G-1]` `[NG-1]` `[PI-1]` `[KD-1]` `[SO-1]` `[CT-1]` `[VC-1]` `[C-1]` `[OQ-1]` `[EX-1]`").html;
    let kinds = html
        .split("data-kind=\"")
        .skip(1)
        .map(|fragment| fragment.split('"').next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(kinds, vec!["g", "ng", "pi", "kd", "so", "ct", "vc", "c", "oq", "ex"]);
}
#[test]
fn bracketed_xref() {
    let html = render("See `[KD-3.1]` for details.").html;
    for fragment in ["class=\"xref-chip\"", "data-kind=\"kd\"", "data-ref=\"KD-3.1\""] {
        assert!(html.contains(fragment));
    }
}
#[test]
fn xrefs_protect_code() {
    let html = render("```text\nAC-001\n```\n\n`VC-1` `before [KD-1] after`").html;
    assert!(html.contains("AC-001"));
    assert!(!html.contains("xref-chip"));
}
#[test]
fn highlight_shape_and_colors() {
    let html = render("```ts\n// Answer\nconst answer: number = 42\n```").html;
    for fragment in [
        "class=\"shiki shiki-themes catppuccin-latte catppuccin-mocha\"",
        "background-color:var(--code-bg)",
        "--shiki-dark-bg:var(--code-bg)",
        "color:var(--accent)",
        "--shiki-dark:var(--accent)",
        "color:var(--muted)",
        "--shiki-dark:var(--muted)",
        "tabindex=\"0\"",
        "<span class=\"line\">",
    ] {
        assert!(html.contains(fragment), "missing {fragment} in {html}");
    }
}
#[test]
fn mermaid_fence() {
    let result = render("```mermaid\ngraph TD\n  A --> B\n```");
    assert!(result.has_mermaid);
    assert!(result.html.contains("<pre class=\"mermaid\">graph TD\n  A --> B\n</pre>"));
}
#[test]
fn mmd_fence() {
    let result = render("```mmd\ngraph LR\n  A --> B\n```");
    assert!(result.has_mermaid);
    assert!(result.html.contains("<pre class=\"mermaid\">graph LR\n  A --> B\n</pre>"));
}
#[test]
fn typescript_has_no_mermaid() {
    let result = render("```ts\nconst answer: number = 42\n```");
    assert!(!result.has_mermaid);
    assert!(result.html.contains("class=\"shiki"));
}
#[test]
fn mermaid_state_is_local() {
    render("```mermaid\ngraph TD\n  A --> B\n```");
    assert!(!render("Plain text").has_mermaid);
}
#[test]
fn unknown_language() {
    let html = render("```brainfuck\n+++.\n```").html;
    assert_eq!(html, "<pre><code class=\"language-brainfuck\">+++.\n</code></pre>");
}
#[test]
fn escape_fence_script() {
    let html = render("```brainfuck\n<script>alert(1)</script>\n```").html;
    assert!(html.contains("&#x3C;script>alert(1)&#x3C;/script>"));
    assert!(!html.contains("<script>"));
}
#[test]
fn code_tabs() {
    let html = mdx("<CodeTabs><CodeTab label=\"TypeScript\">Tab content</CodeTab></CodeTabs>").html;
    for fragment in [
        "<div class=\"md-codetabs-wrapper\">",
        "<section class=\"md-codetab-panel\" data-label=\"TypeScript\">",
        "Tab content",
    ] {
        assert!(html.contains(fragment));
    }
}
#[test]
fn comparison() {
    let html = mdx("<Comparison><Option name=\"First\">Option content</Option></Comparison>").html;
    for fragment in [
        "<section class=\"md-comparison\">",
        "<div class=\"md-comparison-grid\">",
        "Option content",
    ] {
        assert!(html.contains(fragment));
    }
}
#[test]
fn decision_matrix() {
    let html = mdx("<DecisionMatrix score=\"5\">Matrix content</DecisionMatrix>").html;
    for fragment in ["<table class=\"md-matrix\">", "<td data-score=\"5\">", "Matrix content"] {
        assert!(html.contains(fragment));
    }
}
#[test]
fn annotated_diff() {
    let html = mdx("<AnnotatedDiff>Diff content<MarginNote line=\"3\">Note content</MarginNote></AnnotatedDiff>").html;
    for fragment in [
        "<figure class=\"md-annotated-diff\">",
        "<ol class=\"md-margin-notes\">",
        "Diff content",
        "Note content",
    ] {
        assert!(html.contains(fragment));
    }
}
#[test]
fn flowchart() {
    let result = mdx("<Flowchart>graph TD\nA --> B<NodeDetail name=\"A\">Node content</NodeDetail></Flowchart>");
    assert!(result.has_mermaid);
    for fragment in [
        "<div class=\"md-flowchart\">",
        "<pre class=\"mermaid\">graph TD\nA --&gt; B</pre>",
        "<div class=\"md-node-details\">",
        "Node content",
    ] {
        assert!(result.html.contains(fragment), "{}", result.html);
    }
}
#[test]
fn step_rail() {
    let html = mdx("<StepRail><Step title=\"First\">Step content</Step></StepRail>").html;
    for fragment in [
        "<ol class=\"md-steprail\">",
        "<li class=\"md-step\" data-title=\"First\">",
        "Step content",
    ] {
        assert!(html.contains(fragment));
    }
}
#[test]
fn unknown_component() {
    let html = mdx("<FeatureViewer>\n\nFeature content\n\n</FeatureViewer>").html;
    assert!(html.contains("<div class=\"md-unknown\" data-component=\"FeatureViewer\">"));
    assert!(html.contains("Feature content"));
}
#[test]
fn unknown_inline_component() {
    assert_eq!(
        mdx("before <FeatureViewer>child</FeatureViewer> after").html,
        "<p>before <span class=\"md-unknown\" data-component=\"FeatureViewer\">child</span> after</p>"
    );
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
    let ids = result
        .html
        .split("id=\"")
        .skip(1)
        .map(|fragment| fragment.split('"').next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["before", "repeated", "repeated-2"]);
}
#[test]
fn expression_placeholder() {
    assert!(
        mdx("{someExpression}")
            .html
            .contains("<span class=\"md-unknown\" data-component=\"expression\">someExpression</span>")
    );
}
#[test]
fn mdx_inline_code_angles() {
    let html = mdx("`List<String>`").html;
    assert!(html.contains("<code>List&#x3C;String></code>"));
    assert!(!html.contains("md-unknown"));
}
#[test]
fn markdown_jsx_stays_raw() {
    let html = render("<StepRail>x</StepRail>").html;
    assert!(html.contains("<StepRail>x</StepRail>"));
    assert!(!html.contains("md-steprail"));
    assert!(!html.contains("md-unknown"));
}
#[test]
fn degraded_mdx_keeps_prose() {
    let result = mdx("The prose before the component remains readable.\n\n<StepRail>");
    assert_eq!(result.degraded.as_ref().unwrap().reason, "mdx-parse");
    assert!(result.html.contains("The prose before the component remains readable."));
    assert!(result.html.contains("class=\"md-degraded\""));
}
#[test]
fn mdx_failure_does_not_panic() {
    assert!(std::panic::catch_unwind(|| mdx("Prose\n\n<StepRail>")).is_ok());
}
#[test]
fn final_fallback_escaped_source() {
    let parsed = parse_with("<script>alert(\"failure\")</script>", "fixture.md", |_, _| {
        panic!("pipeline unavailable")
    });
    let result = to_html(&parsed);
    assert!(result.html.contains("class=\"md-degraded\""));
    assert!(result.html.contains("&lt;script&gt;alert(&quot;failure&quot;)&lt;/script&gt;"));
    assert_eq!(result.degraded.unwrap().reason, "mdx-parse");
    assert!(!result.has_mermaid);
    assert!(result.toc.is_empty());
}
#[test]
fn relative_doc_links() {
    let html = to_html(&parse(
        "[leaf](document-render.spec.md) [up](../../other/x.spec.mdx#s) [web](https://example.com/a.md)",
        "plan-viewer/spec/viewer.spec.md",
    ))
    .html;
    assert!(html.contains("data-doc=\"plan-viewer/spec/document-render.spec.md\""));
    assert!(html.contains("data-doc=\"other/x.spec.mdx\""));
    assert!(!html.contains("data-doc=\"https"));
}
#[test]
fn unknown_bare_prefix() {
    let html = render("ZZ-001").html;
    assert!(html.contains("ZZ-001"));
    assert!(!html.contains("xref-chip"));
}
#[test]
fn bracketed_other_prefix() {
    let html = render("`[FR-2]`").html;
    assert!(html.contains("class=\"xref-chip\""));
    assert!(html.contains("data-kind=\"other\""));
    assert!(html.contains("data-ref=\"FR-2\""));
}

#[test]
fn footnote_boolean_attributes() {
    let result = render("Text[^a] again[^a].\n\n[^a]: Foot *note*");
    assert!(result.html.contains("data-footnote-ref aria-describedby=\"footnote-label\""));
    assert!(
        result
            .html
            .contains("<section data-footnotes class=\"footnotes\"><h2 class=\"sr-only\" id=\"footnotes\">Footnotes</h2>")
    );
    assert!(
        result
            .html
            .contains("data-footnote-backref=\"\" aria-label=\"Back to reference 1-2\"")
    );
    assert!(result.html.contains("↩<sup>2</sup>"));
}
#[test]
fn crlf_text_preserved() {
    assert_eq!(
        render("First\r\n second\r\n\r\n`one\r\ntwo` and [one\r\ntwo](x.md)").html,
        "<p>First\r\nsecond</p>\n<p><code>one two</code> and <a href=\"x.md\" data-doc=\"x.md\">one\r\ntwo</a></p>"
    );
}
#[test]
fn shiki_plaintext_names() {
    for lang in ["text", "txt", "plain", "plaintext"] {
        let result = render(&format!("```{lang}\nVC-1 < > &\n```"));
        assert!(result.html.contains("class=\"shiki shiki-themes"));
        assert!(!result.html.contains("xref-chip"));
    }
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
    assert!(matches!(parsed.children.last(),Some(tree::Node::Element(element)) if element.tag == "pre"));
    assert_eq!(
        to_html(&parsed).html,
        "<div class=\"md-degraded\">MDX parsing failed; this document is shown as plain markdown.</div><pre>source &#39; &lt; &amp;</pre>"
    );
}

#[test]
fn literal_jsx_attributes_and_escaping() {
    let result = mdx("before <a href=\"other.spec.md\" title=\"a &amp; &quot; ' b\" empty disabled data-x={ignored}>VC-1</a> <br /> after");
    assert_eq!(
        result.html,
        "<p>before <a href=\"other.spec.md\" title=\"a &amp; &quot; &#x27; b\"><button class=\"xref-chip\" data-kind=\"vc\" data-ref=\"VC-1\" title=\"Copy VC-1\" type=\"button\">VC-1</button></a> <br></br> after</p>"
    );
    assert!(!result.html.contains("data-doc"));
    assert_eq!(result.html.matches("xref-chip").count(), 1);
}
#[test]
fn esm_and_quoted_expression_placeholders() {
    let result =
        mdx("import Component from 'x'\nexport const config = {\n  value: 1\n}\n\n{someExpression}\n\ninline {\"quoted } and {\"} end");
    assert!(result.degraded.is_none());
    assert!(result.html.contains(
        "<span class=\"md-unknown\" data-component=\"module\">import Component from &#x27;x&#x27;\nexport const config = {\n  value: 1\n}</span>"
    ));
    assert!(
        result
            .html
            .contains("data-component=\"expression\">&quot;quoted } and {&quot;</span>")
    );
}
#[test]
fn component_defaults_and_paragraph_child_groups() {
    assert_eq!(
        mdx("<CodeTabs>\n\n<CodeTab>a</CodeTab> <CodeTab>b</CodeTab>\n\n</CodeTabs>").html,
        "<div class=\"md-codetabs-wrapper\"><section class=\"md-codetab-panel\" data-label=\"Tab 1\"><h3>Tab 1</h3>a</section><section class=\"md-codetab-panel\" data-label=\"Tab 2\"><h3>Tab 2</h3>b</section></div>"
    );
    for (source, expected) in [
        (
            "<CodeTabs><CodeTab>a</CodeTab> <CodeTab>b</CodeTab></CodeTabs>",
            "<p><div class=\"md-codetabs-wrapper\"><section class=\"md-codetab-panel\" data-label=\"Tab 1\"><h3>Tab 1</h3>a</section><section class=\"md-codetab-panel\" data-label=\"Tab 2\"><h3>Tab 2</h3>b</section> </div></p>",
        ),
        (
            "<Comparison><Option>a</Option></Comparison>",
            "<p><section class=\"md-comparison\"><div class=\"md-comparison-grid\"><article class=\"md-comparison-option\" data-name=\"Option 1\"><h3>Option 1</h3>a</article></div></section></p>",
        ),
        (
            "<StepRail><Step>x</Step></StepRail>",
            "<p><ol class=\"md-steprail\"><li class=\"md-step\" data-title=\"Step 1\"><h3>Step 1</h3>x</li></ol></p>",
        ),
        (
            "<DecisionMatrix>x</DecisionMatrix>",
            "<p><table class=\"md-matrix\"><tbody><tr><td data-score=\"\">x</td></tr></tbody></table></p>",
        ),
        (
            "<AnnotatedDiff>x<MarginNote>y</MarginNote></AnnotatedDiff>",
            "<p><figure class=\"md-annotated-diff\"><div class=\"md-annotated-diff-content\">x</div><ol class=\"md-margin-notes\"><li class=\"md-margin-note\" data-line=\"1\">y</li></ol></figure></p>",
        ),
        (
            "<Flowchart>graph TD<NodeDetail>x</NodeDetail></Flowchart>",
            "<p><div class=\"md-flowchart\"><pre class=\"mermaid\">graph TD</pre><div class=\"md-node-details\"><section class=\"md-node-detail\" data-name=\"Node 1\"><h3>Node 1</h3>x</section></div></div></p>",
        ),
    ] {
        assert_eq!(mdx(source).html, expected);
    }
}
#[test]
fn gfm_task_and_nested_lists() {
    assert_eq!(
        render("- [x] checked\n- [ ] unchecked\n  - nested\n\n3. ordered\n\n   loose\n\n4. next").html,
        "<ul class=\"contains-task-list\">\n<li class=\"task-list-item\"><input type=\"checkbox\" checked disabled> checked</li>\n<li class=\"task-list-item\"><input type=\"checkbox\" disabled> unchecked\n<ul>\n<li>nested</li>\n</ul>\n</li>\n</ul>\n<ol start=\"3\">\n<li>\n<p>ordered</p>\n<p>loose</p>\n</li>\n<li>\n<p>next</p>\n</li>\n</ol>"
    );
}
#[test]
fn table_alignment_and_missing_cells() {
    let html = render("| L | C | R |\n|:--|:-:|--:|\n| a | b | c |\n| d | e |").html;
    assert!(html.contains("<th align=\"left\">L</th>\n<th align=\"center\">C</th>\n<th align=\"right\">R</th>"));
    assert!(html.contains("<td align=\"left\">d</td>\n<td align=\"center\">e</td>\n<td align=\"right\"></td>"));
}
#[test]
fn entities_raw_html_hard_breaks_and_references() {
    let html = render("~~strike~~ ~one~ https://example.com x@y.com www.example.org\n\n<div>\nRaw VC-1\n</div>\n\nInline <b>VC-2</b> &amp; &copy; &lt;\n\nHard  \nbreak\\\n next\n\n![image & title](<x y.png> \"say 'hi'\") [ref][r]\n\n[r]: ../x.spec.md \"Title & quote\"").html;
    for fragment in [
        "<del>strike</del> <del>one</del>",
        "<a href=\"https://example.com\">https://example.com</a>",
        "<a href=\"mailto:x@y.com\">x@y.com</a>",
        "<a href=\"http://www.example.org\">www.example.org</a>",
        "<div>\nRaw VC-1\n</div>",
        "<p>Hard<br>\nbreak<br>\nnext</p>",
        "&#x26; © &#x3C;",
        "<img src=\"x%20y.png\" alt=\"image &#x26; title\" title=\"say &#x27;hi&#x27;\">",
        "<a href=\"../x.spec.md\" title=\"Title &#x26; quote\">ref</a>",
    ] {
        assert!(html.contains(fragment), "missing {fragment}");
    }
}
#[test]
fn xref_word_boundaries_and_limits() {
    let html = render("VC-1 VC-12345 VC-1.2 xVC-2 _VC-3 éVC-4 VC-9x [VC-6] `[ZZ-99.3]` `[KD-1.2.3]`").html;
    assert_eq!(html.matches("xref-chip").count(), 5);
    for fragment in [
        "VC-12345",
        "xVC-2",
        "_VC-3",
        "VC-9x",
        "<code>[KD-1.2.3]</code>",
        "data-kind=\"other\" data-ref=\"ZZ-99.3\"",
    ] {
        assert!(html.contains(fragment));
    }
}
#[test]
fn doc_links_reject_nonrelative_and_escaping_targets() {
    let html = to_html(&parse(
        "[abs](/x.md) [escape](../../x.md) [query](x.md?x=1) [anchor](x.mdx#x) [dot](./a/../b.md) [scheme](ftp://a.md) [encoded](x%20y.md)",
        "spec/fixture.md",
    ))
    .html;
    assert_eq!(html.matches("data-doc").count(), 3);
    for target in ["spec/x.mdx", "spec/b.md", "spec/x%20y.md"] {
        assert!(html.contains(&format!("data-doc=\"{target}\"")));
    }
}
#[test]
fn shared_tree_retains_components_code_and_navigation() {
    fn elements(nodes: &[tree::Node], output: &mut Vec<tree::Element>) {
        for node in nodes {
            match node {
                tree::Node::Element(element) => {
                    output.push(element.clone());
                    elements(&element.children, output);
                }
                tree::Node::Bridge(children) => elements(children, output),
                _ => {}
            }
        }
    }
    let parsed = parse(
        "<CodeTabs>\n<CodeTab label=\"TS\">\n\n```ts\nconst value = 1\n```\n\nVC-1 [doc](x.spec.md)\n\n</CodeTab>\n</CodeTabs>",
        "spec/fixture.mdx",
    );
    let mut found = Vec::new();
    elements(&parsed.children, &mut found);
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
fn highlight_aliases_and_terminal_tokens() {
    let source = "const value = \"hello\"\n\n// comment\n";
    for language in [
        "ts",
        "tsx",
        "js",
        "json",
        "yaml",
        "bash",
        "java",
        "kotlin",
        "sql",
        "diff",
        "md",
        "typescript",
        "javascript",
        "cts",
        "mts",
        "cjs",
        "mjs",
        "sh",
        "shell",
        "shellscript",
        "zsh",
        "yml",
        "markdown",
        "kt",
        "kts",
    ] {
        for theme in [highlight::Theme::Light, highlight::Theme::Dark] {
            let lines = highlight::tokens(source, language, theme).unwrap();
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line.iter().map(|token| token.text.as_str()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n"),
                source
            );
        }
    }
    assert!(highlight::tokens(source, "brainfuck", highlight::Theme::Light).is_none());
    assert!(highlight::tokens(source, "rust", highlight::Theme::Light).is_none());
}

#[test]
fn mdx_soft_breaks_between_child_nodes() {
    assert_eq!(
        mdx("before <FeatureViewer>one\n*two*\nthree\n<ChildViewer>four</ChildViewer>\nfive</FeatureViewer> after").html,
        "<p>before <span class=\"md-unknown\" data-component=\"FeatureViewer\">one\n\n<em>two</em>\n\nthree\n\n<span class=\"md-unknown\" data-component=\"ChildViewer\">four</span>\n\nfive</span> after</p>"
    );
    assert_eq!(
        mdx("<FeatureViewer>[one\n*two*\nthree](x.md)\nnext</FeatureViewer>").html,
        "<p><span class=\"md-unknown\" data-component=\"FeatureViewer\"><a href=\"x.md\" data-doc=\"x.md\">one\n<em>two</em>\nthree</a>\n\nnext</span></p>"
    );
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
