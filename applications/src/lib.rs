use uuid::Uuid;

mod application_context;
mod instance;
mod instance_context;
mod view;
mod view_builder;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
struct InstanceId(Uuid);
