/// A generic type that describes progress over time in form of a stream of varying values that
/// can be cancelled or committed at the end.
#[derive(Debug, Clone, Copy)]
pub enum Progress<T> {
    Proceed(T),
    /// Commit at the previous T sent with Proceed. This is the last progress event.
    Commit,
    /// Cancel. This is the last progress event.
    Cancel,
}

impl<T> Progress<T> {
    pub fn try_map<R>(self, mut f: impl FnMut(T) -> Option<R>) -> Option<Progress<R>> {
        match self {
            Progress::Proceed(v) => f(v).map(Progress::Proceed),
            Progress::Commit => Some(Progress::Commit),
            Progress::Cancel => Some(Progress::Cancel),
        }
    }

    pub fn try_map_or_cancel<R>(self, f: impl FnMut(T) -> Option<R>) -> Progress<R> {
        self.try_map_or(f, Progress::Cancel)
    }

    pub fn try_map_or_commit<R>(self, f: impl FnMut(T) -> Option<R>) -> Progress<R> {
        self.try_map_or(f, Progress::Commit)
    }

    pub fn try_map_or<R>(self, mut f: impl FnMut(T) -> Option<R>, or: Progress<R>) -> Progress<R> {
        match self {
            Progress::Proceed(v) => match f(v) {
                Some(r) => Progress::Proceed(r),
                None => or,
            },
            Progress::Commit => Progress::Commit,
            Progress::Cancel => Progress::Cancel,
        }
    }

    pub fn map<R>(self, mut f: impl FnMut(T) -> R) -> Progress<R> {
        match self {
            Progress::Proceed(v) => Progress::Proceed(f(v)),
            Progress::Commit => Progress::Commit,
            Progress::Cancel => Progress::Cancel,
        }
    }

    pub fn ends(&self) -> bool {
        self.proceeds().is_none()
    }

    pub fn proceeds(&self) -> Option<&T> {
        match self {
            Progress::Proceed(v) => Some(v),
            Progress::Commit => None,
            Progress::Cancel => None,
        }
    }
}
