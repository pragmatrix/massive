use uuid::Uuid;

mod application_context;
mod persistence;
mod persistence_context;
mod presence;
mod presence_builder;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
struct PersistenceId(Uuid);
