use std::cmp::Ordering;
use std::collections::HashMap;

use super::js::name_order;
use super::sort::adaptive_sort;
use super::{DocMeta, DocNode, TabConfig};

struct Node {
    wire: DocNode,
    children: Vec<usize>,
    parent: Option<usize>,
    directory: bool,
    order: isize,
}

struct Tree<'a> {
    tab: &'a TabConfig,
    nodes: Vec<Node>,
    directories: Vec<(String, usize)>,
    directory_ids: HashMap<String, usize>,
    documents: HashMap<String, usize>,
    metadata: Vec<(usize, &'a DocMeta)>,
    represented: HashMap<usize, usize>,
}

fn kind_of(doc: &DocMeta, tab: &TabConfig) -> String {
    if let Some(kind) = &doc.kind
        && !kind.is_empty()
    {
        return kind.clone();
    }
    tab.companion(&doc.path).unwrap_or("leaf").to_owned()
}
fn plain(node: &Node) -> bool {
    node.directory && node.wire.path.is_none()
}
fn compare(tab: &TabConfig, first: &Node, second: &Node) -> Ordering {
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
        && tab.stem(left) == tab.stem(right)
        && (first.wire.companion || second.wire.companion)
    {
        let rank = |path: &str| {
            tab.companion(path)
                .and_then(|kind| tab.companions.iter().position(|c| c == kind))
                .map_or(0, |i| i + 1)
        };
        let order = rank(left).cmp(&rank(right));
        if order != Ordering::Equal {
            return order;
        }
    }
    name_order(&first.wire.name.to_lowercase(), &second.wire.name.to_lowercase()).then(first.order.cmp(&second.order))
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
                companion: false,
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
            let relative = doc
                .path
                .strip_prefix(self.tab.root())
                .map_or(doc.path.as_str(), |rest| rest.trim_start_matches('/'));
            let segments: Vec<_> = relative.split('/').filter(|segment| !segment.is_empty()).collect();
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
                    kind: kind_of(doc, self.tab),
                    status: doc.status.clone(),
                    archived: doc.archived,
                    companion: self.tab.companion(&doc.path).is_some(),
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
    /// The document for `<stem>[.<topic>…].<companion>.<ext>`, preferring the longest stem.
    fn owner(&self, path: &str) -> Option<usize> {
        let mut stem = self.tab.stem(path);
        loop {
            let found = self
                .tab
                .documents(stem)
                .iter()
                .find_map(|candidate| self.documents.get(candidate).copied());
            if found.is_some() {
                return found;
            }
            let name_start = stem.rfind('/').map_or(0, |index| index + 1);
            stem = &stem[..name_start + stem[name_start..].rfind('.')?];
        }
    }
    fn resolves_to_umbrella(&self, child: usize, umbrella: usize) -> bool {
        let (Some(child_doc), Some(umbrella_doc)) = (self.meta(child), self.meta(umbrella)) else {
            return false;
        };
        if child_doc.parent_spec.as_deref() == Some(&umbrella_doc.path) {
            return true;
        }
        self.nodes[child].wire.companion
            && self
                .owner(&child_doc.path)
                .and_then(|id| self.meta(id))
                .is_some_and(|doc| doc.parent_spec.as_deref() == Some(&umbrella_doc.path))
    }
    /// The umbrella document a directory already represents.
    fn representative(&self, directory: usize) -> Option<usize> {
        self.represented
            .iter()
            .find(|&(_, &id)| id == directory)
            .map(|(&umbrella, _)| umbrella)
    }
    fn attach_umbrellas(&mut self) {
        let mut directories = self.directories.clone();
        directories.sort_by_key(|(path, _)| std::cmp::Reverse(path.matches('/').count() + usize::from(!path.is_empty())));
        for (path, directory) in directories {
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
                name_order(
                    self.nodes[left].wire.path.as_deref().unwrap_or(""),
                    self.nodes[right].wire.path.as_deref().unwrap_or(""),
                )
            });
            if let Some(&umbrella) = umbrellas.first() {
                let covers = if self.nodes[umbrella].wire.kind == "umbrella" {
                    children.iter().all(|&id| {
                        id == umbrella
                            || if self.nodes[id].directory {
                                self.representative(id).is_some_and(|sub| self.resolves_to_umbrella(sub, umbrella))
                            } else {
                                self.resolves_to_umbrella(id, umbrella)
                            }
                    })
                } else {
                    !nested
                };
                if !path.is_empty() && covers {
                    let name = if self.nodes[umbrella].wire.kind == "umbrella" {
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
            if self.nodes[node].wire.companion
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
    fn sort(&mut self, node: usize) {
        let mut children = self.nodes[node].children.clone();
        adaptive_sort(&mut children, |left, right| {
            compare(self.tab, &self.nodes[left], &self.nodes[right]).is_lt()
        });
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

fn directory_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(directory, _)| directory)
}

/// Fills each spec's missing `parent_spec` from its position: a leaf's parent is the only
/// umbrella in its directory, a sub-umbrella's the only umbrella in the parent directory.
fn with_positional_parents(docs: &[DocMeta], tab: &TabConfig) -> Vec<DocMeta> {
    let mut umbrellas: HashMap<&str, Option<&str>> = HashMap::new();
    for doc in docs.iter().filter(|doc| doc.kind.as_deref() == Some("umbrella")) {
        umbrellas
            .entry(directory_of(&doc.path))
            .and_modify(|unique| *unique = None)
            .or_insert(Some(&doc.path));
    }
    docs.iter()
        .map(|doc| {
            let mut doc = doc.clone();
            let positional = doc.parent_spec.as_deref().is_none_or(str::is_empty)
                && tab.companion(&doc.path).is_none()
                && !matches!(doc.kind.as_deref(), Some("plan" | "phase"));
            if positional {
                let directory = directory_of(&doc.path);
                let lookup = if doc.kind.as_deref() == Some("umbrella") {
                    (!directory.is_empty()).then(|| directory_of(directory))
                } else {
                    Some(directory)
                };
                doc.parent_spec = lookup
                    .and_then(|directory| umbrellas.get(directory).copied().flatten())
                    .filter(|&parent| parent != doc.path)
                    .map(str::to_owned);
            }
            doc
        })
        .collect()
}

/// A path tree of `docs`, rooted at the tab's folder.
pub fn build_tree(docs: &[DocMeta], tab: &TabConfig) -> Vec<DocNode> {
    let docs = with_positional_parents(docs, tab);
    let mut tree = Tree {
        tab,
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
    tree.add_documents(&docs);
    tree.attach_umbrellas();
    tree.attach_companions();
    tree.attach_parents();
    for child in tree.nodes[0].children.clone() {
        tree.compact(child);
    }
    tree.sort(0);
    tree.nodes[0].children.iter().map(|&id| tree.wire(id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn specs() -> TabConfig {
        crate::ParseConfig::default().tabs[1].clone()
    }
    fn plans() -> TabConfig {
        crate::ParseConfig::default().tabs[0].clone()
    }
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
            &specs(),
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
        let tree = build_tree(
            &[umbrella, parent(doc("area/child.spec.md", "Child"), "area/area.spec.md")],
            &specs(),
        );
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
    fn positional_leaves_nest_under_their_umbrella() {
        let tree = build_tree(
            &[
                kind(doc("area/area.spec.md", "Area overview"), "umbrella"),
                doc("area/a.spec.md", "A"),
                doc("area/b.spec.md", "B"),
            ],
            &specs(),
        );
        assert_eq!(tree.len(), 1);
        assert_eq!((&*tree[0].kind, tree[0].path.as_deref()), ("umbrella", Some("area/area.spec.md")));
        assert_eq!(paths(children(&tree[0])), ["area/a.spec.md", "area/b.spec.md"]);
    }
    #[test]
    fn does_not_absorb_with_sibling_declaring_another_parent() {
        let tree = build_tree(
            &[
                kind(doc("area/area.spec.md", "Area overview"), "umbrella"),
                doc("area/linked.spec.md", "Linked"),
                parent(doc("area/elsewhere.spec.md", "Elsewhere"), "missing.spec.md"),
            ],
            &specs(),
        );
        assert_eq!(tree.len(), 1);
        assert_eq!((&*tree[0].kind, &*tree[0].name, tree[0].path.as_deref()), ("dir", "area", None));
        assert_eq!(paths(children(child(&tree[0], "Area overview"))), ["area/linked.spec.md"]);
        assert_eq!(child(&tree[0], "Elsewhere").path.as_deref(), Some("area/elsewhere.spec.md"));
    }
    #[test]
    fn two_umbrellas_in_a_directory_derive_no_parent() {
        let tree = build_tree(
            &[
                kind(doc("area/a.spec.md", "A"), "umbrella"),
                kind(doc("area/b.spec.md", "B"), "umbrella"),
                doc("area/leaf.spec.md", "Leaf"),
            ],
            &specs(),
        );
        assert_eq!((&*tree[0].kind, &*tree[0].name), ("dir", "area"));
        assert_eq!(paths(children(&tree[0])), ["area/a.spec.md", "area/b.spec.md", "area/leaf.spec.md"]);
    }
    #[test]
    fn absorbs_with_companion_of_linking_sibling() {
        let tree = build_tree(
            &[
                kind(doc("area/area.spec.md", "Area overview"), "umbrella"),
                parent(doc("area/child.spec.md", "Child"), "area/area.spec.md"),
                doc("area/child.discovery.md", "Child discovery"),
            ],
            &specs(),
        );
        assert_eq!(tree.len(), 1);
        assert_eq!((&*tree[0].kind, tree[0].path.as_deref()), ("umbrella", Some("area/area.spec.md")));
        assert_eq!(paths(children(child(&tree[0], "Child"))), ["area/child.discovery.md"]);
    }
    #[test]
    fn sub_umbrella_directory_nests_under_parent_umbrella() {
        let tree = build_tree(
            &[
                kind(doc("settings/settings.spec.md", "Settings"), "umbrella"),
                doc("settings/general.spec.md", "General"),
                kind(doc("settings/custom-fields/custom-fields.spec.md", "Custom Fields"), "umbrella"),
                doc("settings/custom-fields/types.spec.md", "Types"),
            ],
            &specs(),
        );
        assert_eq!(tree.len(), 1);
        assert_eq!(
            (&*tree[0].kind, tree[0].path.as_deref()),
            ("umbrella", Some("settings/settings.spec.md"))
        );
        assert_eq!(
            paths(children(&tree[0])),
            ["settings/custom-fields/custom-fields.spec.md", "settings/general.spec.md"]
        );
        assert_eq!(
            paths(children(child(&tree[0], "Custom Fields"))),
            ["settings/custom-fields/types.spec.md"]
        );
    }
    #[test]
    fn does_not_absorb_directory_with_plain_subdirectory() {
        let tree = build_tree(
            &[
                kind(doc("settings/settings.spec.md", "Settings"), "umbrella"),
                doc("settings/notes/loose.spec.md", "Loose"),
            ],
            &specs(),
        );
        assert_eq!((&*tree[0].kind, &*tree[0].name, tree[0].path.as_deref()), ("dir", "settings", None));
        assert_eq!(paths(children(&tree[0])), ["", "settings/settings.spec.md"]);
    }
    #[test]
    fn first_umbrella_by_path_represents_directory() {
        let tree = build_tree(
            &[
                parent(kind(doc("area/z.spec.md", "Later umbrella"), "umbrella"), "area/a.spec.md"),
                kind(doc("area/a.spec.md", "First umbrella"), "umbrella"),
            ],
            &specs(),
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
            &specs(),
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
    fn topic_companion_attaches_to_spec_of_shorter_stem() {
        let tree = build_tree(
            &[
                doc("graph/labels.spec.mdx", "Labels"),
                doc("graph/labels.KD-1-ownership.discovery.mdx", "Ownership discovery"),
            ],
            &specs(),
        );
        let spec = &children(&tree[0])[0];
        assert_eq!(spec.path.as_deref(), Some("graph/labels.spec.mdx"));
        assert_eq!(paths(children(spec)), ["graph/labels.KD-1-ownership.discovery.mdx"]);
    }
    #[test]
    fn plan_examples_attach_to_their_plan() {
        let tree = build_tree(
            &[
                kind(doc(".plan/guide.md", "Guide"), "plan"),
                kind(doc(".plan/guide.examples.md", "Guide examples"), "examples"),
            ],
            &plans(),
        );
        assert_eq!(tree.len(), 1);
        assert_eq!((&*tree[0].kind, tree[0].path.as_deref()), ("plan", Some(".plan/guide.md")));
        assert_eq!(paths(children(&tree[0])), [".plan/guide.examples.md"]);
    }
    #[test]
    fn orphan_companion_remains_in_directory() {
        let tree = build_tree(&[doc("guides/orphan.examples.md", "Orphan examples")], &specs());
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
            &specs(),
        );
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].path.as_deref(), Some("parent.spec.md"));
        assert_eq!(paths(children(&tree[0])), ["child.spec.md"]);
    }
    #[test]
    fn leaves_unlisted_parent_in_place() {
        let tree = build_tree(&[parent(doc("guides/child.spec.md", "Child"), "missing.spec.md")], &specs());
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
            &specs(),
        );
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].path.as_deref(), Some("b.spec.md"));
        assert_eq!(paths(children(&tree[0])), ["a.spec.md"]);
    }
    #[test]
    fn compacts_single_child_directory_chain() {
        let tree = build_tree(&[doc("docs/architecture/specs/guide.spec.md", "Guide")], &specs());
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
            &specs(),
        );
        assert_eq!(tree.len(), 1);
        assert_eq!(
            (&*tree[0].kind, &*tree[0].name, tree[0].path.as_deref()),
            ("plan", "docs/area/plan", Some("docs/area/plan/index.md"))
        );
        assert_eq!(paths(children(&tree[0])), ["docs/area/plan/a.spec.md", "docs/area/plan/b.spec.md"]);
    }
    #[test]
    fn tab_tree_is_rooted_at_its_folder() {
        let tree = build_tree(
            &[
                kind(doc(".plan/area/plan/index.md", "Plan"), "plan"),
                kind(doc(".plan/area/plan/a.md", "A"), "phase"),
                kind(doc(".plan/loose.md", "Loose"), "plan"),
            ],
            &plans(),
        );
        assert_eq!(tree.iter().map(|node| &*node.name).collect::<Vec<_>>(), ["area/plan", "Loose"]);
        assert_eq!(paths(&tree), [".plan/area/plan/index.md", ".plan/loose.md"]);
        assert_eq!(paths(children(&tree[0])), [".plan/area/plan/a.md"]);
    }
    #[test]
    fn archived_umbrella_directories_sort_last() {
        let tree = build_tree(
            &[
                archived(kind(doc("archived/archived.spec.md", "Archived"), "umbrella")),
                kind(doc("active/active.spec.md", "Active"), "umbrella"),
            ],
            &specs(),
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
            &specs(),
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
        let mut explicit_false = doc("16.spec.md", "中");
        explicit_false.archived = Some(false);
        let tree = build_tree(
            &[
                doc("11.spec.md", "éclair"),
                explicit_false,
                doc("14.spec.md", "2 title"),
                doc("15.spec.md", "Zulu"),
            ],
            &specs(),
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
            &plans(),
        );
        assert_eq!(tree[0].kind, "unexpected");
        assert_eq!(
            children(&tree[1]).iter().map(|node| &*node.name).collect::<Vec<_>>(),
            ["Zulu", "Alpha"]
        );
    }
}
