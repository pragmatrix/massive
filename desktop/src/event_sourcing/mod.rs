use std::ops;

use derive_more::{From, IntoIterator};

/// An ordered batch of commands.
///
/// A freshly produced command list (from input/event processing) and a batch assembled for
/// application are the same value: an ordered list of commands. The transaction boundary is the
/// `transact` call that applies them, not a separate type.
//
// Performance: Most often we just handle a single command, so an enum with One(T), Any(Vec<T>)
// could prevent heap allocations here.
#[must_use]
#[derive(Debug, PartialEq, IntoIterator, From)]
pub struct Transaction<Command> {
    commands: Vec<Command>,
}

impl<Command> From<Command> for Transaction<Command> {
    fn from(value: Command) -> Self {
        vec![value].into()
    }
}

impl<Command, const LEN: usize> From<[Command; LEN]> for Transaction<Command> {
    fn from(value: [Command; LEN]) -> Self {
        let v: Vec<Command> = value.into();
        v.into()
    }
}

impl<Command> Transaction<Command> {
    #[allow(non_upper_case_globals)]
    pub const None: Self = Self {
        commands: Vec::new(),
    };

    pub fn is_none(&self) -> bool {
        self.commands.is_empty()
    }

    pub fn map<Mapped>(self, f: impl Fn(Command) -> Mapped) -> Transaction<Mapped> {
        self.commands.into_iter().map(f).collect::<Vec<_>>().into()
    }
}

impl<T> ops::Add for Transaction<T> {
    type Output = Transaction<T>;

    fn add(mut self, rhs: Self) -> Self::Output {
        self.commands.extend(rhs);
        self
    }
}

impl<T> ops::AddAssign<T> for Transaction<T> {
    fn add_assign(&mut self, rhs: T) {
        self.commands.push(rhs);
    }
}

impl<T> ops::AddAssign<Transaction<T>> for Transaction<T> {
    fn add_assign(&mut self, rhs: Self) {
        self.commands.extend(rhs.commands);
    }
}
