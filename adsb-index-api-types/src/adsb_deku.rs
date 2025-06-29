use crate::IcaoAddress;

impl From<adsb_deku::ICAO> for IcaoAddress {
    fn from(value: adsb_deku::ICAO) -> Self {
        Self::from_bytes(value.0)
    }
}

impl From<IcaoAddress> for adsb_deku::ICAO {
    fn from(value: IcaoAddress) -> Self {
        Self(value.as_bytes())
    }
}
