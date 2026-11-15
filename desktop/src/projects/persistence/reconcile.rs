//! The reconcile of ADR 0037: it carries the comments and formatting of the file on disk over to
//! the fresh document derived from the aggregate.
//!
//! The result reads as exactly the fresh document, so the aggregate in memory decides every value.
//! Only comments and formatting come from the file. Matched nodes keep theirs, removed nodes take
//! the comments above them along, and inserted nodes bring the autoformatting of the fresh
//! document.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::mem;

use kdl::{KdlDocument, KdlNode};

use super::kdl_format::{slot_name, slot_position};

/// Reconciles `fresh`, the document derived from the aggregate, into `existing`, the document parsed
/// from the file. The returned document follows the order of `fresh`.
pub fn reconcile(mut existing: KdlDocument, fresh: &KdlDocument) -> KdlDocument {
    keep_file_header(&mut existing);
    reconcile_nodes(existing.nodes_mut(), fresh.nodes());
    existing
}

/// Moves the comments at the top of the file into the document's own leading text, when a blank
/// line separates them from the first node. The parser attaches them to the first node, so they
/// would otherwise move or vanish with whichever node happens to come first.
///
/// The header ends at the last blank line before which the text parses on its own. A blank line
/// inside a block comment therefore does not split it.
fn keep_file_header(document: &mut KdlDocument) {
    let Some(first) = document.nodes_mut().first_mut() else {
        return;
    };
    let leading = &mut first
        .format_mut()
        .expect("A parsed node has a format")
        .leading;
    let Some(end) = leading
        .rmatch_indices("\n\n")
        .map(|(index, _)| index + 2)
        .find(|&end| KdlDocument::parse(&leading[..end]).is_ok())
    else {
        return;
    };
    let header: String = leading.drain(..end).collect();
    document
        .format_mut()
        .expect("A parsed document has a format")
        .leading
        .push_str(&header);
}

/// Reconciles the children of one node, or the top-level nodes, and recurses into every matched
/// pair, so the same rules apply to slots, spawn parameters, and nested param objects.
fn reconcile_nodes(existing: &mut Vec<KdlNode>, fresh: &[KdlNode]) {
    let pairs = pair_nodes(existing, fresh);
    let mut existing_nodes: Vec<Option<KdlNode>> =
        mem::take(existing).into_iter().map(Some).collect();
    *existing = fresh
        .iter()
        .zip(pairs)
        .map(|(fresh_node, pair)| match pair {
            Some(index) => {
                let mut node = existing_nodes[index]
                    .take()
                    .expect("Each existing node pairs at most once");
                reconcile_node(&mut node, fresh_node);
                node
            }
            None => fresh_node.clone(),
        })
        .collect();
    terminate_nodes(existing);
}

/// Pairs each fresh node with at most one existing node, as the index of the existing node.
///
/// A node without a position, such as `startup`, `mcp`, or a spawn parameter, pairs by its node
/// name alone in the first pass. Slot nodes pair in three passes: by kind, position, and name, then by
/// kind and name, then by kind and position. A moved slot, even one swapped with another, so keeps
/// its comments and, for a project, its subtree, and a slot renamed in place keeps them too. Nodes
/// with equal keys pair in document order.
fn pair_nodes(existing: &[KdlNode], fresh: &[KdlNode]) -> Vec<Option<usize>> {
    let mut pairs = vec![None; fresh.len()];
    let mut paired = vec![false; existing.len()];
    pair_by(
        existing,
        fresh,
        |node| Some(full_key(node)),
        &mut pairs,
        &mut paired,
    );
    pair_by(existing, fresh, name_key, &mut pairs, &mut paired);
    pair_by(existing, fresh, position_key, &mut pairs, &mut paired);
    pairs
}

/// One pass of [`pair_nodes`]: pairs the fresh nodes not yet paired with the existing nodes not yet
/// paired that have the same key. A node whose key is `None` takes no part in the pass.
fn pair_by<'a, K: Eq + Hash>(
    existing: &'a [KdlNode],
    fresh: &'a [KdlNode],
    key: impl Fn(&'a KdlNode) -> Option<K>,
    pairs: &mut [Option<usize>],
    paired: &mut [bool],
) {
    let mut candidates: HashMap<K, VecDeque<usize>> = HashMap::new();
    for (index, node) in existing.iter().enumerate() {
        if !paired[index]
            && let Some(key) = key(node)
        {
            candidates.entry(key).or_default().push_back(index);
        }
    }
    for (pair, node) in pairs.iter_mut().zip(fresh) {
        if pair.is_some() {
            continue;
        }
        if let Some(index) = key(node)
            .and_then(|key| candidates.get_mut(&key))
            .and_then(VecDeque::pop_front)
        {
            *pair = Some(index);
            paired[index] = true;
        }
    }
}

/// The kind, position, and name of a slot node, or the node name alone of any other node. The name of
/// a node without a position stays out, because a param's argument is its value, which may change.
fn full_key(node: &KdlNode) -> (&str, Option<(u32, u32)>, Option<&str>) {
    let position = slot_position(node);
    (node.name().value(), position, position.and(slot_name(node)))
}

/// The kind and name of a slot node. A node without a position is not a slot, so it takes no part
/// in this pass.
fn name_key(node: &KdlNode) -> Option<(&str, &str)> {
    slot_position(node)?;
    Some((node.name().value(), slot_name(node)?))
}

/// The kind and position of a slot node.
fn position_key(node: &KdlNode) -> Option<(&str, (u32, u32))> {
    Some((node.name().value(), slot_position(node)?))
}

/// Gives a matched existing node the values and children of its fresh counterpart, and keeps its
/// own comments and formatting where they still apply.
fn reconcile_node(existing: &mut KdlNode, fresh: &KdlNode) {
    // `KdlEntry::set_value` keeps printing the old value's text, so changed entries are replaced
    // as a whole, with the fresh formatting.
    if !same_values(existing, fresh) {
        *existing.entries_mut() = fresh.entries().to_vec();
    }
    match (existing.children_mut(), fresh.children()) {
        (Some(existing_children), Some(fresh_children)) => {
            reconcile_nodes(existing_children.nodes_mut(), fresh_children.nodes());
        }
        (None, Some(fresh_children)) => {
            let before_children = fresh
                .format()
                .expect("A parsed node has a format")
                .before_children
                .clone();
            existing
                .format_mut()
                .expect("A parsed node has a format")
                .before_children = before_children;
            existing.set_children(fresh_children.clone());
        }
        (Some(_), None) => existing.clear_children(),
        (None, None) => {}
    }
}

/// Whether two nodes hold the same entries, compared by key and value and not by their text.
fn same_values(existing: &KdlNode, fresh: &KdlNode) -> bool {
    existing.entries().len() == fresh.entries().len()
        && existing
            .entries()
            .iter()
            .zip(fresh.entries())
            .all(|(existing, fresh)| {
                existing.name().map(|name| name.value()) == fresh.name().map(|name| name.value())
                    && existing.value() == fresh.value()
            })
}

/// Ends every node but the last with a line break when it has no terminator. A node can lack one
/// as the last node of a block written on one line, or of a file without a final line break, and a
/// node placed after it would otherwise continue its line.
fn terminate_nodes(nodes: &mut [KdlNode]) {
    let count = nodes.len();
    for node in nodes.iter_mut().take(count.saturating_sub(1)) {
        let format = node.format_mut().expect("A parsed node has a format");
        if format.terminator.is_empty() {
            // The space that stood before the closing brace would end the line otherwise.
            format
                .before_terminator
                .truncate(format.before_terminator.trim_end().len());
            format.terminator = "\n".into();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::projects::persistence::kdl_format;

    #[test]
    fn a_comment_above_a_matched_node_survives() {
        let existing = "// The shell.\nlauncher shell at=\"0,0\"\n";
        let fresh = "launcher shell at=\"0,0\"\nlauncher build at=\"1,0\"\n";
        assert_eq!(
            reconciled(existing, fresh),
            "// The shell.\nlauncher shell at=\"0,0\"\nlauncher build at=\"1,0\"\n"
        );
    }

    #[test]
    fn a_moved_slot_keeps_its_comments_and_subtree() {
        let existing = "\
// The labs.
project labs at=\"0,0\" {
    // The build.
    launcher build at=\"0,0\"
}
";
        let fresh = "\
launcher shell at=\"0,0\"
project labs at=\"2,0\" {
    launcher build at=\"0,0\"
}
";
        assert_eq!(
            reconciled(existing, fresh),
            "\
launcher shell at=\"0,0\"
// The labs.
project labs at=\"2,0\" {
    // The build.
    launcher build at=\"0,0\"
}
"
        );
    }

    #[test]
    fn a_removed_node_takes_its_comments_along() {
        let existing = "\
// The shell.
launcher shell at=\"0,0\"
// The build.
launcher build at=\"1,0\"
";
        let fresh = "launcher build at=\"1,0\"\n";
        assert_eq!(
            reconciled(existing, fresh),
            "// The build.\nlauncher build at=\"1,0\"\n"
        );
    }

    /// A slot that stays in place but changes its name pairs by position, so it keeps its comment
    /// and carries the fresh values.
    #[test]
    fn a_changed_node_carries_the_fresh_values() {
        let existing = "\
// The shell.
launcher shell at=\"0,0\" {
    // How to start it.
    command \"zsh\"
}
";
        let fresh = "\
launcher login at=\"0,0\" full_screen_mode=fullscreen {
    command bash
}
";
        assert_eq!(
            reconciled(existing, fresh),
            "\
// The shell.
launcher login at=\"0,0\" full_screen_mode=fullscreen {
    // How to start it.
    command bash
}
"
        );
    }

    /// When two slots swap their positions, each keeps its own comments.
    #[test]
    fn swapped_slots_keep_their_comments() {
        let existing = "\
// The shell.
launcher shell at=\"0,0\" {
    // A login shell.
    command \"zsh -l\"
}
// The build.
launcher build at=\"1,0\"
";
        let fresh = "\
launcher build at=\"0,0\"
launcher shell at=\"1,0\" {
    command \"zsh -l\"
}
";
        assert_eq!(
            reconciled(existing, fresh),
            "\
// The build.
launcher build at=\"0,0\"
// The shell.
launcher shell at=\"1,0\" {
    // A login shell.
    command \"zsh -l\"
}
"
        );
    }

    /// A blank line inside a block comment does not end the file header, so removing the first node
    /// keeps the comment whole.
    #[test]
    fn a_block_comment_with_a_blank_line_stays_whole() {
        let existing = "/* One.\n\nTwo. */\nlauncher shell at=\"0,0\"\nlauncher build at=\"1,0\"\n";
        let fresh = "launcher build at=\"1,0\"\n";
        assert_eq!(reconciled(existing, fresh), "launcher build at=\"1,0\"\n");
    }

    /// The comments at the top of the file stay there, even when the node below them goes away.
    #[test]
    fn the_file_header_stays_when_the_first_node_goes() {
        let existing = "// Header.\n\nstartup \"/shell\"\nlauncher shell at=\"0,0\"\n";
        let fresh = "launcher shell at=\"0,0\"\n";
        assert_eq!(
            reconciled(existing, fresh),
            "// Header.\n\nlauncher shell at=\"0,0\"\n"
        );
    }

    /// A node inserted after the last node of a block written on one line starts a line of its
    /// own, instead of continuing the line of the node before it.
    #[test]
    fn a_node_inserted_after_an_unterminated_node_starts_its_own_line() {
        let existing = "launcher shell at=\"0,0\" { command just }";
        let fresh = "\
launcher shell at=\"0,0\" {
    command just
    term xterm
}
launcher build at=\"1,0\"
";
        let text = reconciled(existing, fresh);
        assert!(text.contains("command just\n"), "{text}");
    }

    /// Reconciles the fresh text into the existing text, and checks that the result reads as
    /// exactly the fresh document.
    fn reconciled(existing: &str, fresh: &str) -> String {
        let path = Path::new("desktop.kdl");
        let existing = kdl_format::parse_document(path, existing).unwrap();
        let fresh =
            kdl_format::write_document(&kdl_format::read_configuration(path, fresh).unwrap());
        let text = reconcile(existing, &fresh).to_string();
        assert_eq!(
            kdl_format::read_configuration(path, &text).unwrap(),
            kdl_format::read_document(path, &fresh.to_string(), &fresh).unwrap(),
            "{text}"
        );
        text
    }
}
