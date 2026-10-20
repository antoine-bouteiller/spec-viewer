//! Fuzzy title and path search across every tab, shared by the terminal and the web app.
use crate::{DocNode, TabDocs};

/// A document matching every query term, with the matched character indices of its title and path.
#[derive(Clone, Debug)]
pub struct Hit {
    /// An index into `Docs::tabs`.
    pub tab: usize,
    pub node: DocNode,
    pub path: String,
    pub title_matches: Vec<usize>,
    pub path_matches: Vec<usize>,
    score: i64,
}

/// Every document of every tab matching each whitespace-separated term of `query` as a
/// subsequence of its title or path, best first; all documents in tab order when it is blank.
pub fn search(tabs: &[TabDocs], query: &str) -> Vec<Hit> {
    let terms: Vec<_> = query.split_whitespace().map(fold).collect();
    let mut hits = Vec::new();
    for (index, tab) in tabs.iter().enumerate() {
        let mut documents = Vec::new();
        collect(&tab.nodes, &mut documents);
        hits.extend(documents.into_iter().filter_map(|node| hit(index, node, &terms)));
    }
    if terms.is_empty() {
        return hits;
    }
    hits.sort_by(|first, second| {
        second
            .score
            .cmp(&first.score)
            .then(first.path.chars().count().cmp(&second.path.chars().count()))
    });
    hits
}

fn collect<'a>(nodes: &'a [DocNode], documents: &mut Vec<&'a DocNode>) {
    for node in nodes {
        if node.path.is_some() {
            documents.push(node);
        }
        collect(node.children.as_deref().unwrap_or_default(), documents);
    }
}

fn hit(tab: usize, node: &DocNode, terms: &[Vec<char>]) -> Option<Hit> {
    let path = node.path.clone()?;
    let (title, folded_path) = (fold(&node.name), fold(&path));
    let (mut title_matches, mut path_matches, mut score) = (Vec::new(), Vec::new(), 0);
    for term in terms {
        match (fuzzy(term, &title), fuzzy(term, &folded_path)) {
            (Some((title_score, positions)), path) if path.as_ref().is_none_or(|(path_score, _)| title_score >= *path_score) => {
                score += title_score;
                title_matches.extend(positions);
            }
            (_, Some((path_score, positions))) => {
                score += path_score;
                path_matches.extend(positions);
            }
            _ => return None,
        }
    }
    for matches in [&mut title_matches, &mut path_matches] {
        matches.sort_unstable();
        matches.dedup();
    }
    Some(Hit {
        tab,
        node: node.clone(),
        path,
        title_matches,
        path_matches,
        score,
    })
}

/// Lower-cases per character so indices stay aligned with the original text.
fn fold(text: &str) -> Vec<char> {
    text.chars()
        .map(|character| character.to_lowercase().next().unwrap_or(character))
        .collect()
}

/// Scores the shortest window ending at the leftmost complete match of `term` in `text`.
fn fuzzy(term: &[char], text: &[char]) -> Option<(i64, Vec<usize>)> {
    let mut next = 0;
    let end = text.iter().position(|character| {
        if *character == term[next] {
            next += 1;
        }
        next == term.len()
    })?;
    let mut positions = Vec::with_capacity(term.len());
    for index in (0..=end).rev() {
        if text[index] == term[term.len() - 1 - positions.len()] {
            positions.push(index);
            if positions.len() == term.len() {
                break;
            }
        }
    }
    positions.reverse();
    let mut score = -((end + 1 - positions[0] - term.len()) as i64);
    for (rank, &position) in positions.iter().enumerate() {
        score += 16;
        if position == 0 || !text[position - 1].is_alphanumeric() {
            score += 8;
        }
        if rank > 0 && positions[rank - 1] + 1 == position {
            score += 8;
        }
    }
    Some((score, positions))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, path: &str) -> DocNode {
        DocNode {
            name: name.into(),
            path: Some(path.into()),
            kind: "leaf".into(),
            status: None,
            archived: None,
            companion: false,
            children: None,
        }
    }

    fn tabs() -> Vec<TabDocs> {
        vec![
            TabDocs {
                name: "plans".into(),
                nodes: vec![node("Rollout", ".plan/rollout.md")],
            },
            TabDocs {
                name: "specs".into(),
                nodes: vec![DocNode {
                    children: Some(vec![node("Tree leaf", "doc/tree.spec.md")]),
                    ..node("Viewer umbrella", "doc/viewer.spec.md")
                }],
            },
        ]
    }

    fn paths(query: &str) -> Vec<String> {
        search(&tabs(), query).into_iter().map(|hit| hit.path).collect()
    }

    #[test]
    fn blank_query_lists_every_document_in_tab_order() {
        assert_eq!(paths("  "), [".plan/rollout.md", "doc/viewer.spec.md", "doc/tree.spec.md"]);
    }

    #[test]
    fn every_term_must_match_title_or_path_as_a_subsequence() {
        assert_eq!(paths("vwr"), ["doc/viewer.spec.md"]);
        assert_eq!(paths("tree spec"), ["doc/tree.spec.md"]);
        assert!(paths("tree plan").is_empty());
    }

    #[test]
    fn contiguous_and_boundary_matches_rank_first() {
        assert_eq!(paths("u"), ["doc/viewer.spec.md", ".plan/rollout.md"]);
        assert_eq!(paths("re"), ["doc/tree.spec.md", "doc/viewer.spec.md"]);
        let hit = &search(&tabs(), "leaf doc")[0];
        assert_eq!(
            (hit.tab, hit.title_matches.clone(), hit.path_matches.clone()),
            (1, vec![5, 6, 7, 8], vec![0, 1, 2])
        );
    }
}
