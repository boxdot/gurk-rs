mod forgetful;
mod json;
mod memcache;
mod storage;

pub use forgetful::ForgetfulStorage;
pub use json::JsonStorage;
pub use memcache::MemCache;
pub use storage::{MessageId, Storage};
