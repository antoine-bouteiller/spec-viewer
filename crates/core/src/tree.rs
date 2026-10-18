use std::cmp::Ordering;
use std::collections::HashMap;

use super::js::{locale_compare, spec_stem};
use super::sort::adaptive_sort;
use super::{DocMeta, DocNode};

struct Node {
    wire: DocNode,
    children: Vec<usize>,
    parent: Option<usize>,
    directory: bool,
    order: isize,
}

struct Tree<'a> {
    nodes: Vec<Node>,
    directories: Vec<(String, usize)>,
    directory_ids: HashMap<String, usize>,
    documents: HashMap<String, usize>,
    metadata: Vec<(usize, &'a DocMeta)>,
    represented: HashMap<usize, usize>,
}

fn companion(kind: &str) -> bool {
    matches!(kind, "examples" | "discovery")
}
fn companion_order(kind: &str) -> u8 {
    match kind {
        "examples" => 1,
        "discovery" => 2,
        _ => 0,
    }
}
fn kind_of(doc: &DocMeta) -> String {
    if let Some(kind) = &doc.kind
        && !kind.is_empty()
    {
        return kind.clone();
    }
    if doc.path.ends_with(".discovery.md") || doc.path.ends_with(".discovery.mdx") {
        "discovery"
    } else if doc.path.ends_with(".examples.md") || doc.path.ends_with(".examples.mdx") {
        "examples"
    } else {
        "leaf"
    }
    .to_owned()
}
fn plain(node: &Node) -> bool {
    node.directory && node.wire.path.is_none()
}
fn compare(first: &Node, second: &Node) -> Ordering {
    if plain(first) != plain(second) {
        return if plain(first) { Ordering::Less } else { Ordering::Greater };
    }
    if first.wire.path.is_some() && second.wire.path.is_some() && first.wire.archived != second.wire.archived {
        return if first.wire.archived == Some(true) {
            Ordering::Greater
        } else {
            Ordering::Less
        };
    }
    if first.wire.kind == "phase" && second.wire.kind == "phase" {
        return first.order.cmp(&second.order);
    }
    if let (Some(left), Some(right)) = (&first.wire.path, &second.wire.path)
        && spec_stem(left) == spec_stem(right)
        && (companion(&first.wire.kind) || companion(&second.wire.kind))
    {
        let order = companion_order(&first.wire.kind).cmp(&companion_order(&second.wire.kind));
        if order != Ordering::Equal {
            return order;
        }
    }
    locale_compare(&first.wire.name.to_lowercase(), &second.wire.name.to_lowercase()).then(first.order.cmp(&second.order))
}

impl<'a> Tree<'a> {
    fn directory(&mut self, name: &str, order: isize) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            wire: DocNode {
                name: name.into(),
                path: None,
                kind: "dir".into(),
                status: None,
                archived: None,
                children: None,
            },
            children: vec![],
            parent: None,
            directory: true,
            order,
        });
        id
    }
    fn detach(&mut self, node: usize) {
        if let Some(parent) = self.nodes[node].parent
            && let Some(index) = self.nodes[parent].children.iter().position(|&id| id == node)
        {
            self.nodes[parent].children.remove(index);
            self.nodes[node].parent = None;
        }
    }
    fn append(&mut self, parent: usize, node: usize) {
        self.detach(node);
        self.nodes[parent].children.push(node);
        self.nodes[node].parent = Some(parent);
    }
    fn attach_document(&mut self, directory: usize, document: usize) {
        let name = self.nodes[directory].wire.name.clone();
        self.nodes[directory].wire = self.nodes[document].wire.clone();
        self.nodes[directory].wire.name = name;
        self.nodes[directory].order = self.nodes[document].order;
        self.detach(document);
    }
    fn add_documents(&mut self, docs: &'a [DocMeta]) {
        for (order, doc) in docs.iter().enumerate() {
            let segments: Vec<_> = doc.path.split('/').filter(|segment| !segment.is_empty()).collect();
            if segments.is_empty() {
                continue;
            }
            let mut parent = 0;
            let mut directory_path = String::new();
            for segment in &segments[..segments.len() - 1] {
                if !directory_path.is_empty() {
                    directory_path.push('/');
                }
                directory_path.push_str(segment);
                let directory = if let Some(&id) = self.directory_ids.get(&directory_path) {
                    id
                } else {
                    let id = self.directory(segment, order as isize);
                    self.append(parent, id);
                    self.directory_ids.insert(directory_path.clone(), id);
                    self.directories.push((directory_path.clone(), id));
                    id
                };
                parent = directory;
            }
            let id = self.nodes.len();
            self.nodes.push(Node {
                wire: DocNode {
                    name: doc.title.clone(),
                    path: Some(doc.path.clone()),
                    kind: kind_of(doc),
                    status: doc.status.clone(),
                    archived: doc.archived,
                    children: None,
                },
                children: vec![],
                parent: None,
                directory: false,
                order: order as isize,
            });
            self.append(parent, id);
            self.documents.insert(doc.path.clone(), id);
            self.metadata.push((id, doc));
        }
    }
    fn meta(&self, node: usize) -> Option<&DocMeta> {
        self.metadata.iter().find(|(id, _)| *id == node).map(|(_, doc)| *doc)
    }
    fn owner(&self, path: &str) -> Option<usize> {
        [".spec.md", ".spec.mdx", ".md"]
            .iter()
            .find_map(|suffix| self.documents.get(&format!("{}{suffix}", spec_stem(path))).copied())
    }
    fn resolves_to_umbrella(&self, child: usize, umbrella: usize) -> bool {
        let (Some(child_doc), Some(umbrella_doc)) = (self.meta(child), self.meta(umbrella)) else {
            return false;
        };
        if child_doc.parent_spec.as_deref() == Some(&umbrella_doc.path) {
            return true;
        }
        companion(&self.nodes[child].wire.kind)
            && self
                .owner(&child_doc.path)
                .and_then(|id| self.meta(id))
                .is_some_and(|doc| doc.parent_spec.as_deref() == Some(&umbrella_doc.path))
    }
    fn attach_umbrellas(&mut self, flat: bool) {
        for (path, directory) in self.directories.clone() {
            let children = &self.nodes[directory].children;
            let nested = children.iter().any(|&id| self.nodes[id].directory);
            let mut umbrellas: Vec<_> = children
                .iter()
                .copied()
                .filter(|&id| {
                    self.meta(id).is_some_and(|doc| {
                        doc.kind.as_deref() == Some("umbrella")
                            || (doc.kind.as_deref() == Some("plan") && (doc.path == "index.md" || doc.path.ends_with("/index.md")))
                    })
                })
                .collect();
            umbrellas.sort_by(|&left, &right| {
                locale_compare(
                    self.nodes[left].wire.path.as_deref().unwrap_or(""),
                    self.nodes[right].wire.path.as_deref().unwrap_or(""),
                )
            });
            if let Some(&umbrella) = umbrellas.first() {
                let covers = self.nodes[umbrella].wire.kind != "umbrella"
                    || children.iter().all(|&id| id == umbrella || self.resolves_to_umbrella(id, umbrella));
                if !path.is_empty() && !nested && covers {
                    let name = if flat || self.nodes[umbrella].wire.kind == "umbrella" {
                        self.nodes[umbrella].wire.name.clone()
                    } else {
                        self.nodes[directory].wire.name.clone()
                    };
                    self.attach_document(directory, umbrella);
                    self.nodes[directory].wire.name = name;
                    self.represented.insert(umbrella, directory);
                }
            }
        }
    }
    fn represented(&self, node: usize) -> usize {
        self.represented.get(&node).copied().unwrap_or(node)
    }
    fn attach_companions(&mut self) {
        for (node, doc) in self.metadata.clone() {
            if companion(&self.nodes[node].wire.kind)
                && let Some(owner) = self.owner(&doc.path)
            {
                self.append(self.represented(owner), node);
            }
        }
    }
    fn contains(&self, node: usize, wanted: usize) -> bool {
        node == wanted || self.nodes[node].children.iter().any(|&id| self.contains(id, wanted))
    }
    fn attach_parents(&mut self) {
        for (node, doc) in self.metadata.clone() {
            if let Some(parent) = doc
                .parent_spec
                .as_ref()
                .filter(|segment| !segment.is_empty())
                .and_then(|path| self.documents.get(path))
                .copied()
            {
                let child = self.represented(node);
                let target = self.represented(parent);
                if child != target && !self.contains(child, target) {
                    self.append(target, child);
                }
            }
        }
    }
    fn compact(&mut self, node: usize) {
        for child in self.nodes[node].children.clone() {
            self.compact(child);
        }
        while self.nodes[node].wire.path.is_none()
            && self.nodes[node].directory
            && self.nodes[node].children.len() == 1
            && self.nodes[self.nodes[node].children[0]].directory
        {
            let child = self.nodes[node].children[0];
            self.nodes[node].wire.name = format!("{}/{}", self.nodes[node].wire.name, self.nodes[child].wire.name);
            if self.nodes[child].wire.path.is_some() {
                self.attach_document(node, child);
            }
            self.nodes[node].children = self.nodes[child].children.clone();
            for grandchild in self.nodes[node].children.clone() {
                self.nodes[grandchild].parent = Some(node);
            }
        }
    }
    fn hoisted(&self, nodes: &[usize]) -> Vec<usize> {
        nodes
            .iter()
            .flat_map(|&id| {
                if plain(&self.nodes[id]) {
                    self.hoisted(&self.nodes[id].children)
                } else {
                    vec![id]
                }
            })
            .collect()
    }
    fn sort(&mut self, node: usize) {
        let mut children = self.nodes[node].children.clone();
        adaptive_sort(&mut children, |left, right| compare(&self.nodes[left], &self.nodes[right]).is_lt());
        self.nodes[node].children = children.clone();
        for child in children {
            self.sort(child);
        }
    }
    fn wire(&self, node: usize) -> DocNode {
        let mut wire = self.nodes[node].wire.clone();
        wire.children = if self.nodes[node].children.is_empty() {
            None
        } else {
            Some(self.nodes[node].children.iter().map(|&id| self.wire(id)).collect())
        };
        wire
    }
}

pub fn build_tree(docs: &[DocMeta], flat: bool) -> Vec<DocNode> {
    let mut tree = Tree {
        nodes: vec![],
        directories: vec![],
        directory_ids: HashMap::new(),
        documents: HashMap::new(),
        metadata: vec![],
        represented: HashMap::new(),
    };
    tree.directory("", -1);
    tree.directories.push((String::new(), 0));
    tree.directory_ids.insert(String::new(), 0);
    tree.add_documents(docs);
    tree.attach_umbrellas(flat);
    tree.attach_companions();
    tree.attach_parents();
    if flat {
        tree.nodes[0].children = tree.hoisted(&tree.nodes[0].children);
        for child in tree.nodes[0].children.clone() {
            tree.nodes[child].parent = Some(0);
        }
    } else {
        for child in tree.nodes[0].children.clone() {
            tree.compact(child);
        }
    }
    tree.sort(0);
    tree.nodes[0].children.iter().map(|&id| tree.wire(id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn doc(path: &str, title: &str) -> DocMeta {
        DocMeta {
            path: path.into(),
            title: title.into(),
            ..Default::default()
        }
    }
    fn children(node: &DocNode) -> &[DocNode] {
        node.children.as_deref().unwrap_or_default()
    }
    fn child<'a>(node: &'a DocNode, name: &str) -> &'a DocNode {
        children(node).iter().find(|node| node.name == name).unwrap()
    }
    #[test]
    fn nests_path_segments() {
        let tree = build_tree(
            &[
                doc("docs/guides/intro.spec.md", "Introduction"),
                doc("docs/reference/api.spec.md", "API"),
            ],
            false,
        );
        assert_eq!(tree.len(), 1);
        assert_eq!((&*tree[0].name, &*tree[0].kind), ("docs", "dir"));
        assert_eq!(
            children(&tree[0]).iter().map(|node| (&*node.name, &*node.kind)).collect::<Vec<_>>(),
            [("guides", "dir"), ("reference", "dir")]
        );
        let intro = &children(child(&tree[0], "guides"))[0];
        assert_eq!(
            (&*intro.kind, &*intro.name, intro.path.as_deref()),
            ("leaf", "Introduction", Some("docs/guides/intro.spec.md"))
        );
    }

    fn kind(mut doc: DocMeta, kind: &str) -> DocMeta {
        doc.kind = Some(kind.into());
        doc
    }
    fn parent(mut doc: DocMeta, path: &str) -> DocMeta {
        doc.parent_spec = Some(path.into());
        doc
    }
    fn archived(mut doc: DocMeta) -> DocMeta {
        doc.archived = Some(true);
        doc
    }
    fn paths(nodes: &[DocNode]) -> Vec<&str> {
        nodes.iter().map(|node| node.path.as_deref().unwrap_or("")).collect()
    }
    #[test]
    fn absorbs_umbrella_with_linking_siblings() {
        let mut umbrella = kind(doc("area/area.spec.md", "Area overview"), "umbrella");
        umbrella.status = Some(crate::Status {
            name: "active".into(),
            known: false,
        });
        let tree = build_tree(&[umbrella, parent(doc("area/child.spec.md", "Child"), "area/area.spec.md")], false);
        assert_eq!(tree.len(), 1);
        assert_eq!(
            (
                &*tree[0].kind,
                &*tree[0].name,
                tree[0].path.as_deref(),
                tree[0].status.as_ref().map(|status| status.name.as_str())
            ),
            ("umbrella", "Area overview", Some("area/area.spec.md"), Some("active"))
        );
        assert_eq!(paths(children(&tree[0])), ["area/child.spec.md"]);
    }
    #[test]
    fn does_not_absorb_with_unlinked_sibling() {
        let tree = build_tree(
            &[
                kind(doc("area/area.spec.md", "Area overview"), "umbrella"),
                parent(doc("area/linked.spec.md", "Linked"), "area/area.spec.md"),
                doc("area/unlinked.spec.md", "Unlinked"),
            ],
            false,
        );
        assert_eq!(tree.len(), 1);
        assert_eq!((&*tree[0].kind, &*tree[0].name, tree[0].path.as_deref()), ("dir", "area", None));
        assert_eq!(paths(children(child(&tree[0], "Area overview"))), ["area/linked.spec.md"]);
        assert_eq!(child(&tree[0], "Unlinked").path.as_deref(), Some("area/unlinked.spec.md"));
    }
    #[test]
    fn absorbs_with_companion_of_linking_sibling() {
        let tree = build_tree(
            &[
                kind(doc("area/area.spec.md", "Area overview"), "umbrella"),
                parent(doc("area/child.spec.md", "Child"), "area/area.spec.md"),
                doc("area/child.discovery.md", "Child discovery"),
            ],
            false,
        );
        assert_eq!(tree.len(), 1);
        assert_eq!((&*tree[0].kind, tree[0].path.as_deref()), ("umbrella", Some("area/area.spec.md")));
        assert_eq!(paths(children(child(&tree[0], "Child"))), ["area/child.discovery.md"]);
    }
    #[test]
    fn does_not_absorb_directory_with_subdirectories() {
        let tree = build_tree(
            &[
                kind(doc("settings/settings.spec.md", "Settings"), "umbrella"),
                kind(doc("settings/custom-fields/custom-fields.spec.md", "Custom Fields"), "umbrella"),
            ],
            false,
        );
        assert_eq!((&*tree[0].kind, &*tree[0].name, tree[0].path.as_deref()), ("dir", "settings", None));
        assert_eq!(
            children(&tree[0])
                .iter()
                .map(|node| (&*node.name, node.path.as_deref()))
                .collect::<Vec<_>>(),
            [
                ("Custom Fields", Some("settings/custom-fields/custom-fields.spec.md")),
                ("Settings", Some("settings/settings.spec.md"))
            ]
        );
    }
    #[test]
    fn first_umbrella_by_path_represents_directory() {
        let tree = build_tree(
            &[
                parent(kind(doc("area/z.spec.md", "Later umbrella"), "umbrella"), "area/a.spec.md"),
                kind(doc("area/a.spec.md", "First umbrella"), "umbrella"),
            ],
            false,
        );
        assert_eq!(tree[0].path.as_deref(), Some("area/a.spec.md"));
        assert_eq!(paths(children(&tree[0])), ["area/z.spec.md"]);
        assert_eq!(children(&tree[0])[0].kind, "umbrella");
    }
    #[test]
    fn companions_attach_examples_then_discovery() {
        let tree = build_tree(
            &[
                doc("guide.spec.md", "Guide"),
                doc("guide.discovery.md", "Guide discovery"),
                doc("guide.examples.md", "Guide examples"),
            ],
            false,
        );
        assert_eq!(tree.len(), 1);
        assert_eq!(
            (&*tree[0].name, &*tree[0].kind, tree[0].path.as_deref()),
            ("Guide", "leaf", Some("guide.spec.md"))
        );
        assert_eq!(
            children(&tree[0])
                .iter()
                .map(|node| (&*node.kind, &*node.name, node.path.as_deref()))
                .collect::<Vec<_>>(),
            [
                ("examples", "Guide examples", Some("guide.examples.md")),
                ("discovery", "Guide discovery", Some("guide.discovery.md"))
            ]
        );
    }
    #[test]
    fn plan_examples_attach_in_flat_tree() {
        let tree = build_tree(
            &[
                kind(doc(".plan/guide.md", "Guide"), "plan"),
                kind(doc(".plan/guide.examples.md", "Guide examples"), "examples"),
            ],
            true,
        );
        assert_eq!(tree.len(), 1);
        assert_eq!((&*tree[0].kind, tree[0].path.as_deref()), ("plan", Some(".plan/guide.md")));
        assert_eq!(paths(children(&tree[0])), [".plan/guide.examples.md"]);
    }
    #[test]
    fn orphan_companion_remains_in_directory() {
        let tree = build_tree(&[doc("guides/orphan.examples.md", "Orphan examples")], false);
        assert_eq!(tree.len(), 1);
        assert_eq!((&*tree[0].kind, &*tree[0].name), ("dir", "guides"));
        assert_eq!(
            (&*children(&tree[0])[0].kind, children(&tree[0])[0].path.as_deref()),
            ("examples", Some("guides/orphan.examples.md"))
        );
    }
    #[test]
    fn reparents_when_parent_is_listed() {
        let tree = build_tree(
            &[
                doc("parent.spec.md", "Parent"),
                parent(doc("child.spec.md", "Child"), "parent.spec.md"),
            ],
            false,
        );
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].path.as_deref(), Some("parent.spec.md"));
        assert_eq!(paths(children(&tree[0])), ["child.spec.md"]);
    }
    #[test]
    fn leaves_unlisted_parent_in_place() {
        let tree = build_tree(&[parent(doc("guides/child.spec.md", "Child"), "missing.spec.md")], false);
        assert_eq!((&*tree[0].kind, &*tree[0].name), ("dir", "guides"));
        assert_eq!(paths(children(&tree[0])), ["guides/child.spec.md"]);
    }
    #[test]
    fn skips_ancestor_cycle() {
        let tree = build_tree(
            &[
                parent(doc("a.spec.md", "A"), "b.spec.md"),
                parent(doc("b.spec.md", "B"), "a.spec.md"),
            ],
            false,
        );
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].path.as_deref(), Some("b.spec.md"));
        assert_eq!(paths(children(&tree[0])), ["a.spec.md"]);
    }
    #[test]
    fn compacts_single_child_directory_chain() {
        let tree = build_tree(&[doc("docs/architecture/specs/guide.spec.md", "Guide")], false);
        assert_eq!(tree.len(), 1);
        assert_eq!((&*tree[0].kind, &*tree[0].name), ("dir", "docs/architecture/specs"));
        assert_eq!(paths(children(&tree[0])), ["docs/architecture/specs/guide.spec.md"]);
    }
    #[test]
    fn compaction_preserves_children_of_plan_index() {
        let tree = build_tree(
            &[
                kind(doc("docs/area/plan/index.md", "Plan"), "plan"),
                doc("docs/area/plan/a.spec.md", "A"),
                doc("docs/area/plan/b.spec.md", "B"),
            ],
            false,
        );
        assert_eq!(tree.len(), 1);
        assert_eq!(
            (&*tree[0].kind, &*tree[0].name, tree[0].path.as_deref()),
            ("plan", "docs/area/plan", Some("docs/area/plan/index.md"))
        );
        assert_eq!(paths(children(&tree[0])), ["docs/area/plan/a.spec.md", "docs/area/plan/b.spec.md"]);
    }
    #[test]
    fn flat_hoists_plan_index_with_phases() {
        let tree = build_tree(
            &[
                kind(doc(".plan/area/plan/index.md", "Plan"), "plan"),
                kind(doc(".plan/area/plan/a.md", "A"), "phase"),
                kind(doc(".plan/loose.md", "Loose"), "plan"),
            ],
            true,
        );
        assert_eq!(tree.iter().map(|node| &*node.name).collect::<Vec<_>>(), ["Loose", "Plan"]);
        assert_eq!(paths(&tree), [".plan/loose.md", ".plan/area/plan/index.md"]);
        assert_eq!(paths(children(&tree[1])), [".plan/area/plan/a.md"]);
    }
    #[test]
    fn archived_umbrella_directories_sort_last() {
        let tree = build_tree(
            &[
                archived(kind(doc("archived/archived.spec.md", "Archived"), "umbrella")),
                kind(doc("active/active.spec.md", "Active"), "umbrella"),
            ],
            false,
        );
        assert_eq!(paths(&tree), ["active/active.spec.md", "archived/archived.spec.md"]);
    }
    #[test]
    fn sort_directories_case_insensitive_documents_and_archived_last() {
        let tree = build_tree(
            &[
                doc("zeta/item.spec.md", "Item"),
                doc("bravo.spec.md", "bravo"),
                doc("alpha.spec.md", "Alpha"),
                archived(doc("old.spec.md", "Aardvark")),
            ],
            false,
        );
        assert_eq!(
            tree.iter()
                .map(|node| (&*node.name, &*node.kind, node.archived))
                .collect::<Vec<_>>(),
            [
                ("zeta", "dir", None),
                ("Alpha", "leaf", None),
                ("bravo", "leaf", None),
                ("Aardvark", "leaf", Some(true))
            ]
        );
    }
    #[test]
    fn non_total_archived_order_preserves_established_order() {
        let mut explicit_false = doc("other/16.spec.md", "中");
        explicit_false.archived = Some(false);
        let tree = build_tree(
            &[
                doc("other/11.spec.md", "éclair"),
                doc("dir/14.spec.md", "2 title"),
                doc("dir/15.spec.md", "Zulu"),
                explicit_false,
            ],
            true,
        );
        assert_eq!(
            tree.iter().map(|node| node.name.as_str()).collect::<Vec<_>>(),
            ["2 title", "Zulu", "中", "éclair"]
        );
    }

    #[test]
    fn phase_order_and_unknown_kind_are_preserved() {
        let tree = build_tree(
            &[
                kind(doc(".plan/p/index.md", "Plan"), "plan"),
                kind(doc(".plan/p/1.md", "Zulu"), "phase"),
                kind(doc(".plan/p/2.md", "Alpha"), "phase"),
                kind(doc("custom.spec.md", "Custom"), "unexpected"),
            ],
            true,
        );
        assert_eq!(tree[0].kind, "unexpected");
        assert_eq!(
            children(&tree[1]).iter().map(|node| &*node.name).collect::<Vec<_>>(),
            ["Zulu", "Alpha"]
        );
    }
}
