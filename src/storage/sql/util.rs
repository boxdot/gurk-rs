use std::fmt;

pub(super) trait ResultExt<T, E> {
    fn ok_logged(self) -> Option<T>;
}

impl<T, E: fmt::Display> ResultExt<T, E> for Result<T, E> {
    fn ok_logged(self) -> Option<T> {
        match self {
            Ok(v) => Some(v),
            Err(error) => {
                tracing::error!(%error);
                None
            }
        }
    }
}
