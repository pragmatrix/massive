use std::ops::Deref;

pub type Version = u64;

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

    #[allow(unused)]
    pub fn update(&mut self, value: T, version: Version) {
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
    /// If `validated_at` is less than the latest tick, `max_deps_version` and `value` may be
    /// outdated.
    pub validated_at: Version,
    /// The maximum version of all its dependencies. May be outdated if `validated_at` does not
    /// equal the latest version.
    pub max_deps_version: Version,
    /// The value computed value at `max_deps_version`. This value is computed on demand and may not
    /// be up to date. but it always represents the result of a computation matching the
    /// dependencies at `max_deps_version`.
    pub value: V,
}

impl<T> Deref for Computed<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}
