//! Encoding/decoding of types to/from Sql

use sqlx::database::HasArguments;
use sqlx::error::BoxDynError;
use sqlx::sqlite::SqliteValueRef;
use sqlx::{Database, Decode, Encode, Sqlite};
use uuid::Uuid;

use crate::data::ChannelId;

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
    fn encode_by_ref(
        &self,
        buf: &mut <Sqlite as HasArguments<'q>>::ArgumentBuffer,
    ) -> sqlx::encode::IsNull {
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

pub(super) trait BlobData {
    type Error: std::error::Error;

    fn encode(&self) -> Result<Vec<u8>, Self::Error>;
    fn decode(bytes: &[u8]) -> Result<Self, Self::Error>
    where
        Self: Sized;
}

impl BlobData for Vec<Uuid> {
    type Error = postcard::Error;

    fn encode(&self) -> Result<Vec<u8>, Self::Error> {
        postcard::to_allocvec(self)
    }

    fn decode(bytes: &[u8]) -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        postcard::from_bytes(bytes)
    }
}
