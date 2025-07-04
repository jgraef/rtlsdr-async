use sqlx::{
    Database,
    Decode,
    Encode,
    Type,
    encode::IsNull,
    error::BoxDynError,
};

use crate::Wtc;

impl<DB: Database> Type<DB> for Wtc
where
    i8: Type<DB>,
{
    fn type_info() -> DB::TypeInfo {
        <i8 as Type<DB>>::type_info()
    }
}

impl<'q, DB: Database> Encode<'q, DB> for Wtc
where
    i8: Encode<'q, DB>,
{
    fn encode_by_ref(
        &self,
        buf: &mut <DB as Database>::ArgumentBuffer<'q>,
    ) -> Result<IsNull, BoxDynError> {
        <i8 as Encode<DB>>::encode_by_ref(&(self.as_char() as i8), buf)
    }
}

impl<'r, DB: Database> Decode<'r, DB> for Wtc
where
    i8: Decode<'r, DB>,
{
    fn decode(value: <DB as Database>::ValueRef<'r>) -> Result<Self, BoxDynError> {
        let c = <i8 as Decode<DB>>::decode(value)?;
        Ok(Self::from_char(c as u8 as char).unwrap_or_else(|| panic!("invalid wtc: {c}")))
    }
}
