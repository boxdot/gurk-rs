mod json;
mod memcache;
mod storage;
#[cfg(test)]
mod test;

pub use json::JsonStorage;
pub use memcache::MemCache;
pub use storage::{MessageId, Storage};
#[cfg(test)]
pub use test::ForgetfulStorage;
