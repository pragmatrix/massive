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

#[derive(Debug)]
pub struct Computed<V> {
    /// This is last the time the `max_deps_version` and computed value was validated to be
    /// consistent with its dependencies.
    ///
    /// If `validated_at` is less than the latest version, `max_deps_version` and `value` may be
    /// outdated.
    pub validated_at: Version,

    /// The computed value at `max_deps_version`. This value is computed on demand and may not be up
    /// to date. but it always represents the result of a computation matching the dependencies at
    /// `max_deps_version`.
    ///
    /// The `updated_at` version is equivalent to the maximum version of all the dependencies
    /// (including transitives).

    /// The `updated_at` and the value may be outdated if `validated_at` does not equal the latest
    /// version.
    pub versioned: Versioned<V>,
}

impl<T> Deref for Computed<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.versioned.deref()
    }
}

impl<T> Default for Computed<T>
where
    Versioned<T>: Default,
{
    fn default() -> Self {
        Self {
            validated_at: 0,
            versioned: Versioned::default(),
        }
    }
}
