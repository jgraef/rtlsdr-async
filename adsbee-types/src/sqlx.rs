use sqlx::{
    Database,
    Decode,
    Encode,
    Type,
    encode::IsNull,
    error::BoxDynError,
    postgres::{
        PgHasArrayType,
        PgTypeInfo,
    },
};

use crate::{
    IcaoAddress,
    Squawk,
};

impl<DB: Database> Type<DB> for IcaoAddress
where
    i32: Type<DB>,
{
    fn type_info() -> DB::TypeInfo {
        <i32 as Type<DB>>::type_info()
    }
}

impl<'q, DB: Database> Encode<'q, DB> for IcaoAddress
where
    i32: Encode<'q, DB>,
{
    fn encode_by_ref(
        &self,
        buf: &mut <DB as Database>::ArgumentBuffer<'q>,
    ) -> Result<IsNull, BoxDynError> {
        let mut address = self.address as i32;
        if self.non_icao {
            address = -address;
        }
        <i32 as Encode<DB>>::encode_by_ref(&(address as i32), buf)
    }
}

impl<'r, DB: Database> Decode<'r, DB> for IcaoAddress
where
    i32: Decode<'r, DB>,
{
    fn decode(value: <DB as Database>::ValueRef<'r>) -> Result<Self, BoxDynError> {
        let address = <i32 as Decode<DB>>::decode(value)?;
        let non_icao = address < 0;
        let address = address.abs() as u32;
        Ok(Self { address, non_icao })
    }
}

impl PgHasArrayType for IcaoAddress {
    fn array_type_info() -> PgTypeInfo {
        i32::array_type_info()
    }
}

impl<DB: Database> Type<DB> for Squawk
where
    i16: Type<DB>,
{
    fn type_info() -> DB::TypeInfo {
        <i16 as Type<DB>>::type_info()
    }
}

impl<'q, DB: Database> Encode<'q, DB> for Squawk
where
    i16: Encode<'q, DB>,
{
    fn encode_by_ref(
        &self,
        buf: &mut <DB as Database>::ArgumentBuffer<'q>,
    ) -> Result<IsNull, BoxDynError> {
        <i16 as Encode<DB>>::encode_by_ref(&(self.code as i16), buf)
    }
}

impl<'r, DB: Database> Decode<'r, DB> for Squawk
where
    i16: Decode<'r, DB>,
{
    fn decode(value: <DB as Database>::ValueRef<'r>) -> Result<Self, BoxDynError> {
        let code = <i16 as Decode<DB>>::decode(value)?;
        Ok(Self::from_u16_unchecked(code as u16))
    }
}

impl PgHasArrayType for Squawk {
    fn array_type_info() -> sqlx::postgres::PgTypeInfo {
        i32::array_type_info()
    }
}
