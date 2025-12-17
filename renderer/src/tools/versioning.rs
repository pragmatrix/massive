use std::ops::Deref;

use crate::Version;

#[derive(Debug)]
pub struct Versioned<T> {
    value: T,
    pub updated_at: Version,
}

impl<T> Deref for Versioned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> Versioned<T> {
    pub fn new(value: T, version: Version) -> Self {
        Self {
            value,
            updated_at: version,
        }
    }

    pub fn resolve(&mut self, head_version: Version, mut resolver: impl FnMut() -> T) -> &T {
        assert!(head_version >= self.updated_at);
        if self.updated_at < head_version {
            self.update(resolver(), head_version);
        }
        &self.value
    }

    fn update(&mut self, value: T, version: Version) {
        assert!(version > self.updated_at);
        self.value = value;
        self.updated_at = version;
    }
}

#[derive(Debug, Default)]
pub struct Computed<V> {
    /// This is last the time the `max_deps_version` and computed value was validated to be
    /// consistent with its dependencies.
    ///
    /// If `validated_at` is less than the latest version, `max_deps_version` and `value` may be
    /// outdated.
    pub validated_at: Version,
    /// The maximum version of all its dependencies. May be outdated if `validated_at` does not
    /// equal the latest version.
    ///
    /// This is also equivalent to the updated_at in the [`Versioned`], because it describes that
    /// version the value was last updated.
    ///
    pub max_deps_version: Version,
    /// The value computed value at `max_deps_version`. This value is computed on demand and may not
    /// be up to date. but it always represents the result of a computation matching the
    /// dependencies at `max_deps_version`.
    ///
    /// Idea: Use Versioned here (max_deps_version is equivalent to updated_at).
    pub value: V,
}

impl<T> Deref for Computed<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}
