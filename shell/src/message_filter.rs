use std::{collections::HashSet, hash::Hash, mem};

/// Filters messages to keep only the latest occurrence of each message type,
/// preserving the original order of the remaining messages.
///
/// Additionally `selector` can be used to specifically select the Msgs that should be filtered.
pub fn keep_last_per_variant<Msg>(
    messages: Vec<Msg>,
    mut selector: impl FnMut(&Msg) -> bool,
) -> Vec<Msg> {
    keep_last_per_key(messages, |msg| {
        selector(msg).then(|| mem::discriminant(msg))
    })
}

pub fn keep_last_per_key<Msg, Key: Eq + Hash>(
    messages: Vec<Msg>,
    mut get_key: impl FnMut(&Msg) -> Option<Key>,
) -> Vec<Msg> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(messages.len());
    for msg in messages.into_iter().rev() {
        match get_key(&msg) {
            Some(key) => {
                if seen.insert(key) {
                    out.push(msg);
                }
            }
            None => out.push(msg),
        }
    }
    out.reverse(); // restore original order
    out
}
