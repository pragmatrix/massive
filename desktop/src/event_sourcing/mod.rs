use std::ops;

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
