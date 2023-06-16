//! Encoding/decoding of types to/from Sql

use serde::de::DeserializeOwned;
use serde::Serialize;
use sqlx::database::HasArguments;
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::sqlite::SqliteValueRef;
use sqlx::{Database, Decode, Encode, Sqlite};
use uuid::Uuid;

use crate::data::ChannelId;

use super::util::ResultExt;

impl Decode<'_, Sqlite> for ChannelId {
    fn decode(value: SqliteValueRef<'_>) -> Result<Self, BoxDynError> {
        let bytes: &[u8] = Decode::decode(value)?;
        if let Ok(uuid) = Uuid::from_slice(bytes) {
            Ok(uuid.into())
        } else {
            Ok(bytes.try_into()?)
        }
    }
}

impl<'q> Encode<'q, Sqlite> for &'q ChannelId {
    fn encode_by_ref(&self, buf: &mut <Sqlite as HasArguments<'q>>::ArgumentBuffer) -> IsNull {
        match self {
            ChannelId::User(uuid) => uuid.encode(buf),
            ChannelId::Group(bytes) => bytes.encode(buf),
        }
    }
}

impl sqlx::Type<Sqlite> for ChannelId {
    fn type_info() -> <Sqlite as Database>::TypeInfo {
        <&[u8] as sqlx::Type<Sqlite>>::type_info()
    }
}

/// All data wrapped as BlobData is encoded/decoded with postcard via serde
pub(super) struct BlobData<T>(pub T);

impl<T> BlobData<T> {
    pub(super) fn into_inner(self) -> T {
        self.0
    }
}

impl<T: DeserializeOwned> Decode<'_, Sqlite> for BlobData<T> {
    fn decode(value: SqliteValueRef<'_>) -> Result<Self, BoxDynError> {
        let bytes: &[u8] = Decode::decode(value)?;
        Ok(BlobData(postcard::from_bytes(bytes)?))
    }
}

impl<'q, T: Serialize> Encode<'q, Sqlite> for BlobData<T> {
    fn encode_by_ref(&self, buf: &mut <Sqlite as HasArguments<'q>>::ArgumentBuffer) -> IsNull {
        if let Some(bytes) = postcard::to_allocvec(&self.0).ok_logged() {
            bytes.encode(buf)
        } else {
            IsNull::Yes
        }
    }
}

impl<T> sqlx::Type<Sqlite> for BlobData<T> {
    fn type_info() -> <Sqlite as Database>::TypeInfo {
        <&[u8] as sqlx::Type<Sqlite>>::type_info()
    }
}
