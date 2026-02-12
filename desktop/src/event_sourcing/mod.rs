use std::{
    ops::{self},
    vec,
};

use derive_more::{From, IntoIterator};

#[derive(Debug, IntoIterator, From)]
pub struct Transaction<Command> {
    commands: Vec<Command>,
}

impl<Command> Transaction<Command> {
    pub fn map<Cmd2>(self, f: impl Fn(Command) -> Cmd2) -> Transaction<Cmd2> {
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

impl<T> From<Cmd<T>> for Transaction<T> {
    fn from(value: Cmd<T>) -> Self {
        value.0.into()
    }
}

// Performance: Most often we just handle a single command, so I guess an enum with One(T),
// Any(Vec<T>) could prevent heap allocations here.
#[must_use]
#[derive(Debug, PartialEq)]
pub struct Cmd<T>(Vec<T>);

impl<T> Cmd<T> {
    #[allow(non_upper_case_globals)]
    pub const None: Self = Self(Vec::new());

    pub fn is_none(&self) -> bool {
        self.0.is_empty()
    }
}

impl<T> From<T> for Cmd<T> {
    fn from(value: T) -> Self {
        Cmd(vec![value])
    }
}

impl<T> ops::AddAssign<T> for Cmd<T> {
    fn add_assign(&mut self, rhs: T) {
        self.0.push(rhs);
    }
}

impl<T> ops::AddAssign<Cmd<T>> for Cmd<T> {
    fn add_assign(&mut self, rhs: Self) {
        self.0.extend(rhs.0);
    }
}

impl<T> IntoIterator for Cmd<T> {
    type Item = T;
    type IntoIter = vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
