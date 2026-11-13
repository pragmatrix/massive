//! The effect scheduler: runs the work that a transaction's changes make necessary (ADR 0043).
//!
//! An effect is a value without data that names work still to do. Code marks effects through a
//! task-local that [`scope`] installs for one transaction. [`run`] then handles them in the order
//! that the kinds' declared reads give, one rank batch at a time. This module is generic over the
//! effect type, so the desktop's effects and the tests' effects share it.

use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::Hash;
use std::{fmt, mem};

use anyhow::Result;
use tokio::task::LocalKey;

/// A value that names work still to do: its kind and, for most kinds, a subject.
///
/// Equal effects are one piece of work, so an effect carries no data that its handling needs from
/// whoever marked it (ADR 0043).
pub trait Effect: Eq + Hash + Clone + fmt::Debug + 'static {
    type Kind: EffectKind;

    fn kind(&self) -> Self::Kind;

    /// The one effect of `kind`, or `None` when the kind has a subject. Kinds without a subject
    /// are marked automatically after a kind they read was handled.
    fn without_subject(kind: Self::Kind) -> Option<Self>;
}

/// The kind of an effect, which declares the kinds whose results its handling reads.
pub trait EffectKind: Copy + Eq + fmt::Debug + 'static {
    /// Every kind, in index order.
    const ALL: &'static [Self];

    /// The kind's position in [`Self::ALL`].
    fn index(self) -> usize;

    /// The kinds whose results the handling of this kind reads. A kind may read itself.
    fn reads(self) -> &'static [Self];
}

/// Handles the pending effects, one rank batch at a time, until none are pending. Returns the
/// first error a handling returns.
pub fn run<E: Effect>(
    pending: &'static LocalKey<RefCell<PendingEffects<E>>>,
    mut handle: impl FnMut(E) -> Result<()>,
) -> Result<()> {
    pending.with(|cell| {
        // Handling marks only its own rank or later ones, so the ranks below the one just handled
        // stay empty (ADR 0043).
        let mut lowest_rank = 0;
        // One set serves every rank batch, so its allocation is reused.
        let mut seen = HashSet::new();
        let mut pending = cell.borrow_mut();
        let graph = pending.graph;
        while let Some((rank, batch)) = pending.take_rank_batch(lowest_rank, &mut seen) {
            lowest_rank = rank;
            let kind = graph.kinds[rank];
            // The marks made while handling are checked against the kind being handled.
            pending.current_kind = Some(kind);
            // Handling marks further effects through the task-local, so the borrow is released
            // while the batch is handled.
            drop(pending);
            for effect in batch {
                handle(effect)?;
            }
            pending = cell.borrow_mut();
            pending.mark_automatic(kind);
        }
        pending.current_kind = None;
        Ok(())
    })
}

/// Marks `effect` in the current transaction. Panics outside [`scope`], so that a mark never
/// waits silently for a later transaction.
pub fn mark<E: Effect>(pending: &'static LocalKey<RefCell<PendingEffects<E>>>, effect: E) {
    pending
        .try_with(|pending| pending.borrow_mut().mark(effect))
        .expect("Effects are marked only inside a transaction (ADR 0043)");
}

/// Drops the pending effects that `keep` rejects.
pub fn retain<E: Effect>(
    pending: &'static LocalKey<RefCell<PendingEffects<E>>>,
    mut keep: impl FnMut(&E) -> bool,
) {
    pending.with(|pending| {
        for effects in &mut pending.borrow_mut().by_rank {
            effects.retain(&mut keep);
        }
    });
}

/// Runs `f` with fresh pending effects installed in `pending`, the scope of one transaction.
pub fn scope<E: Effect, R>(
    pending: &'static LocalKey<RefCell<PendingEffects<E>>>,
    graph: &'static EffectGraph<E>,
    f: impl FnOnce() -> R,
) -> R {
    pending.sync_scope(RefCell::new(PendingEffects::new(graph)), f)
}

/// The effects marked in the current transaction and not yet handled, one `Vec` per rank.
///
/// Lives in a task-local that [`scope`] installs.
#[derive(Debug)]
pub struct PendingEffects<E: Effect> {
    graph: &'static EffectGraph<E>,
    by_rank: Vec<Vec<E>>,
    /// The kind of the rank batch being handled; `None` while no batch is handled, which is when
    /// changes mark.
    current_kind: Option<E::Kind>,
}

impl<E: Effect> PendingEffects<E> {
    fn new(graph: &'static EffectGraph<E>) -> Self {
        Self {
            graph,
            by_rank: vec![Vec::new(); graph.ranks.len()],
            current_kind: None,
        }
    }

    /// Appends `effect` to its rank. Never sorts or searches, because marking is far more frequent
    /// than handling (ADR 0043).
    fn mark(&mut self, effect: E) {
        let kind = effect.kind();
        // A mark that no read declares would let an effect run before the results it reads are
        // final.
        debug_assert!(
            self.current_kind
                .is_none_or(|current| kind == current || kind.reads().contains(&current)),
            "{effect:?} was marked while handling {:?}, but {kind:?} does not read it",
            self.current_kind
        );
        let rank = self.graph.rank(kind);
        // Rank batches are taken from the rank being handled upward, so an effect marked below it
        // would never be handled.
        assert!(
            self.current_kind
                .is_none_or(|current| rank >= self.graph.rank(current)),
            "{effect:?} was marked at a rank that was already handled"
        );
        self.by_rank[rank].push(effect);
    }

    /// Takes the pending effects of the lowest rank from `from_rank` upward, without duplicates and
    /// in the order they were first marked. Returns their rank with them, or `None` when nothing is
    /// pending there.
    ///
    /// `seen` is scratch space for recognizing duplicates, passed in so its allocation is reused.
    fn take_rank_batch(
        &mut self,
        from_rank: usize,
        seen: &mut HashSet<E>,
    ) -> Option<(usize, Vec<E>)> {
        // A later rank batch of the same rank handles a re-marked effect again.
        seen.clear();
        let rank = (from_rank..self.by_rank.len()).find(|&rank| !self.by_rank[rank].is_empty())?;
        let mut batch = mem::take(&mut self.by_rank[rank]);
        // The set only answers "marked before?", so its iteration order never shows.
        batch.retain(|effect| seen.insert(effect.clone()));
        Some((rank, batch))
    }

    /// Marks the kinds without a subject that read the `handled` kind.
    fn mark_automatic(&mut self, handled: E::Kind) {
        for effect in &self.graph.automatic[handled.index()] {
            self.by_rank[self.graph.rank(effect.kind())].push(effect.clone());
        }
    }
}

/// The ranks of the effect kinds and the automatic marks that follow each kind, computed once from
/// the declared reads.
#[derive(Debug)]
pub struct EffectGraph<E: Effect> {
    /// The rank of each kind, by kind index.
    ranks: Vec<usize>,
    /// The kind of each rank, by rank. Every kind has a rank of its own.
    kinds: Vec<E::Kind>,
    /// The kinds without a subject that read each kind, by kind index, as their effects.
    automatic: Vec<Vec<E>>,
}

impl<E: Effect> EffectGraph<E> {
    /// Sorts the kinds topologically by their reads. Panics on a cycle between different kinds,
    /// because no order could satisfy it.
    pub fn new() -> Self {
        let kinds = <E::Kind as EffectKind>::ALL;
        for (index, kind) in kinds.iter().enumerate() {
            assert_eq!(kind.index(), index, "EffectKind::ALL is in index order");
        }

        let mut ranks = vec![None; kinds.len()];
        let mut kinds_by_rank = Vec::with_capacity(kinds.len());
        let mut next_rank = 0;
        // Each pass ranks every kind whose other reads are ranked, in the order of `ALL`, so ties
        // are broken deterministically.
        while next_rank < kinds.len() {
            let ready: Vec<E::Kind> = kinds
                .iter()
                .copied()
                .filter(|kind| ranks[kind.index()].is_none())
                .filter(|kind| {
                    kind.reads()
                        .iter()
                        .all(|read| read == kind || ranks[read.index()].is_some())
                })
                .collect();
            assert!(
                !ready.is_empty(),
                "Effect kinds read each other in a cycle: {:?}",
                kinds
                    .iter()
                    .filter(|kind| ranks[kind.index()].is_none())
                    .collect::<Vec<_>>()
            );
            for kind in ready {
                ranks[kind.index()] = Some(next_rank);
                kinds_by_rank.push(kind);
                next_rank += 1;
            }
        }

        let mut automatic = vec![Vec::new(); kinds.len()];
        for &reader in kinds {
            let Some(effect) = E::without_subject(reader) else {
                continue;
            };
            // A kind that reads itself would otherwise mark itself after every handling.
            for read in reader.reads().iter().filter(|read| **read != reader) {
                automatic[read.index()].push(effect.clone());
            }
        }

        Self {
            ranks: ranks.into_iter().map(Option::unwrap).collect(),
            kinds: kinds_by_rank,
            automatic,
        }
    }

    fn rank(&self, kind: E::Kind) -> usize {
        self.ranks[kind.index()]
    }
}

impl<E: Effect> Default for EffectGraph<E> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use tokio::task_local;

    use super::*;

    #[test]
    fn kinds_are_handled_in_the_order_of_their_reads_not_their_declaration() {
        let handled = transaction(|| {
            mark_test(TestEffect::Second);
            mark_test(TestEffect::First);
        });

        assert_eq!(handled, [TestEffect::First, TestEffect::Second]);
    }

    #[test]
    fn duplicates_are_handled_once_and_a_self_mark_runs_in_a_later_rank_batch() {
        let handled = transaction(|| {
            mark_test(TestEffect::Climb(1));
            mark_test(TestEffect::Climb(2));
            mark_test(TestEffect::Climb(2));
        });

        // `Climb(2)` marks `Climb(1)` again, the way a child's `Measure` marks its parent's, so
        // `Climb(1)` is handled once more after it.
        assert_eq!(
            handled,
            [
                TestEffect::Climb(1),
                TestEffect::Climb(2),
                TestEffect::Climb(1),
                TestEffect::Reader
            ]
        );
    }

    #[test]
    fn a_kind_without_a_subject_runs_only_after_a_kind_it_reads() {
        assert_eq!(
            transaction(|| mark_test(TestEffect::First)),
            [TestEffect::First, TestEffect::Second]
        );
        assert_eq!(
            transaction(|| mark_test(TestEffect::Climb(0))),
            [TestEffect::Climb(0), TestEffect::Reader]
        );
    }

    #[test]
    #[should_panic(expected = "does not read")]
    #[cfg(debug_assertions)]
    fn a_mark_that_no_read_declares_panics() {
        transaction(|| mark_test(TestEffect::Climb(9)));
    }

    #[test]
    #[should_panic(expected = "cycle")]
    fn kinds_that_read_each_other_panic() {
        EffectGraph::<CyclicEffect>::new();
    }

    #[test]
    #[should_panic(expected = "only inside a transaction")]
    fn a_mark_outside_a_transaction_panics() {
        mark_test(TestEffect::First);
    }

    task_local! {
        static PENDING: RefCell<PendingEffects<TestEffect>>;
    }

    static GRAPH: LazyLock<EffectGraph<TestEffect>> = LazyLock::new(EffectGraph::new);

    /// Runs `mark` as a transaction's changes and returns the effects in the order handled.
    fn transaction(mark: impl FnOnce()) -> Vec<TestEffect> {
        scope(&PENDING, &GRAPH, || {
            mark();
            let mut handled = Vec::new();
            run(&PENDING, |effect| {
                handled.push(effect.clone());
                match effect {
                    TestEffect::Climb(2) => mark_test(TestEffect::Climb(1)),
                    // A mark that `First` does not declare as read.
                    TestEffect::Climb(9) => mark_test(TestEffect::First),
                    _ => {}
                }
                Ok(())
            })
            .unwrap();
            handled
        })
    }

    fn mark_test(effect: TestEffect) {
        mark(&PENDING, effect);
    }

    /// `Second` is declared first but reads `First`. `Climb` reads itself, and `Reader` reads
    /// `Climb`. `Second` and `Reader` have no subject.
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    enum TestEffect {
        Second,
        First,
        Climb(u32),
        Reader,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestKind {
        Second,
        First,
        Climb,
        Reader,
    }

    impl Effect for TestEffect {
        type Kind = TestKind;

        fn kind(&self) -> TestKind {
            match self {
                Self::Second => TestKind::Second,
                Self::First => TestKind::First,
                Self::Climb(_) => TestKind::Climb,
                Self::Reader => TestKind::Reader,
            }
        }

        fn without_subject(kind: TestKind) -> Option<Self> {
            match kind {
                TestKind::Second => Some(Self::Second),
                TestKind::Reader => Some(Self::Reader),
                TestKind::First | TestKind::Climb => None,
            }
        }
    }

    impl EffectKind for TestKind {
        const ALL: &'static [Self] = &[Self::Second, Self::First, Self::Climb, Self::Reader];

        fn index(self) -> usize {
            self as usize
        }

        fn reads(self) -> &'static [Self] {
            match self {
                Self::Second => &[Self::First],
                Self::First => &[],
                Self::Climb => &[Self::Climb],
                Self::Reader => &[Self::Climb],
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    #[expect(
        dead_code,
        reason = "the cycle test only builds the graph of the kinds"
    )]
    enum CyclicEffect {
        A,
        B,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CyclicKind {
        A,
        B,
    }

    impl Effect for CyclicEffect {
        type Kind = CyclicKind;

        fn kind(&self) -> CyclicKind {
            match self {
                Self::A => CyclicKind::A,
                Self::B => CyclicKind::B,
            }
        }

        fn without_subject(_kind: CyclicKind) -> Option<Self> {
            None
        }
    }

    impl EffectKind for CyclicKind {
        const ALL: &'static [Self] = &[Self::A, Self::B];

        fn index(self) -> usize {
            self as usize
        }

        fn reads(self) -> &'static [Self] {
            match self {
                Self::A => &[Self::B],
                Self::B => &[Self::A],
            }
        }
    }
}
