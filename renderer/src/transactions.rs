pub type Version = u64;

/// The "TransactionManager" manages the current version of scene updates.
///
/// Introduced to be able to pass the current version to the various renderer.
#[derive(Debug, Default)]
pub struct TransactionManager {
    current_version: Version,
}

impl TransactionManager {
    pub fn new_transaction(&mut self) -> Transaction {
        self.current_version += 1;
        self.current_transaction()
    }

    pub fn current_transaction(&mut self) -> Transaction {
        Transaction {
            current_version: self.current_version,
        }
    }

    pub fn current(&self) -> Version {
        self.current_version
    }
}

#[derive(Debug)]
pub struct Transaction {
    current_version: Version,
}

impl Transaction {
    pub fn current_version(&self) -> Version {
        self.current_version
    }
}
