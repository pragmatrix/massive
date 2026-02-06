use derive_more::{From, IntoIterator};

#[derive(Debug, From)]
pub enum OrderedInsertion<Parent> {
    Insert(Parent, usize),
    Append(Parent),
}

impl<Parent> OrderedInsertion<Parent> {
    pub fn parent(&self) -> &Parent {
        match self {
            OrderedInsertion::Insert(parent, _) => parent,
            OrderedInsertion::Append(parent) => parent,
        }
    }

    pub fn map<NT>(self, f: impl Fn(Parent) -> NT) -> OrderedInsertion<NT> {
        match self {
            Self::Insert(p, i) => OrderedInsertion::Insert(f(p), i),
            Self::Append(p) => OrderedInsertion::Append(f(p)),
        }
    }
}

#[derive(Debug, IntoIterator, From)]
pub struct Transaction<Command> {
    commands: Vec<Command>,
}

impl<Command> Transaction<Command> {
    pub fn map<Cmd2>(self, f: impl Fn(Command) -> Cmd2) -> Transaction<Cmd2> {
        self.commands.into_iter().map(f).collect::<Vec<_>>().into()
    }
}
