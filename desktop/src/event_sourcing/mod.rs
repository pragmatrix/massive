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
