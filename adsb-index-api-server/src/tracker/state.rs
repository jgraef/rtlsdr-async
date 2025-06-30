use std::{
    collections::{
        HashMap,
        hash_map,
    },
    ops::{
        Deref,
        DerefMut,
    },
};

use adsb_index_api_types::{
    IcaoAddress,
    Squawk,
};
use chrono::{
    DateTime,
    Utc,
};

use crate::{
    source::adsb_deku as adsb,
    util::sparse_list::SparseList,
};

#[derive(Debug, Default)]
pub struct State {
    aircraft: SparseList<AircraftState>,
    by_icao_address: HashMap<IcaoAddress, usize>,
    by_callsign: HashMap<String, usize>,
    by_squawk: HashMap<Squawk, usize>,
}

impl State {
    pub fn update_aircraft(
        &mut self,
        icao_address: IcaoAddress,
        time: DateTime<Utc>,
    ) -> UpdateAircraftState<'_> {
        let (index, aircraft) = match self.by_icao_address.entry(icao_address) {
            hash_map::Entry::Occupied(occupied) => {
                let index = *occupied.get();
                let aircraft = &mut self.aircraft[index];
                aircraft.last_seen.update(time, ());
                (index, aircraft)
            }
            hash_map::Entry::Vacant(vacant) => {
                let (index, aircraft) = self
                    .aircraft
                    .insert_and_get_mut(AircraftState::new(icao_address, time));
                vacant.insert(index);
                (index, aircraft)
            }
        };

        UpdateAircraftState {
            index,
            state: aircraft,
            by_callsign: &mut self.by_callsign,
            by_squawk: &mut self.by_squawk,
            time,
        }
    }

    pub fn update_mlat_position(
        &mut self,
        icao_address: IcaoAddress,
        time: DateTime<Utc>,
        position: Position,
    ) {
        let mut aircraft = self.update_aircraft(icao_address, time);
        aircraft.position.update(time, position);
    }

    pub fn update_with_modes_frame(&mut self, time: DateTime<Utc>, frame: &adsb::Frame) {
        tracing::debug!("Mode-S frame: {frame:#?}");

        match &frame.df {
            adsb::DF::ADSB(adsb) => {
                let icao_address = adsb.icao.into();
                let mut aircraft = self.update_aircraft(icao_address, time);

                aircraft.mode_s_packets.push(frame.df.clone());

                match &adsb.me {
                    adsb::adsb::ME::AirbornePositionBaroAltitude { altitude, .. } => {
                        aircraft.update_airborne_position(altitude, AltitudeType::Barometric);
                    }
                    adsb::adsb::ME::AirborneVelocity(airborne_velocity) => {
                        aircraft.update_airborne_velocity(time, airborne_velocity);
                    }
                    adsb::adsb::ME::AircraftIdentification { identification, .. } => {
                        aircraft.update_aircraft_identification(identification);
                    }
                    adsb::adsb::ME::SurfacePosition { id, surface } => todo!(),
                    adsb::adsb::ME::AirbornePositionGNSSAltitude { id, altitude } => {
                        aircraft.update_airborne_position(altitude, AltitudeType::Gnss);
                    }
                    adsb::adsb::ME::Reserved0(_) => todo!(),
                    adsb::adsb::ME::SurfaceSystemStatus(_) => todo!(),
                    adsb::adsb::ME::Reserved1 { id, slice } => todo!(),
                    adsb::adsb::ME::AircraftStatus(aircraft_status) => {
                        aircraft.update_aircraft_status(aircraft_status);
                    }
                    adsb::adsb::ME::TargetStateAndStatusInformation(
                        target_state_and_status_information,
                    ) => todo!(),
                    adsb::adsb::ME::AircraftOperationalCoordination(_) => todo!(),
                    adsb::adsb::ME::AircraftOperationStatus(operation_status) => todo!(),
                    _ => {
                        tracing::debug!("unhandled ads-b frame: {frame:?}");
                    }
                }
            }
            adsb::DF::AllCallReply {
                capability,
                icao,
                p_icao,
            } => {
                self.update_aircraft((*icao).into(), time);
            }
            adsb::DF::ShortAirAirSurveillance {
                vs,
                cc,
                unused,
                sl,
                unused1,
                ri,
                unused2,
                altitude,
                parity,
            } => todo!(),
            adsb::DF::SurveillanceAltitudeReply { fs, dr, um, ac, ap } => todo!(),
            adsb::DF::SurveillanceIdentityReply { fs, dr, um, id, ap } => todo!(),
            adsb::DF::LongAirAir {
                vs,
                spare1,
                sl,
                spare2,
                ri,
                spare3,
                altitude,
                mv,
                parity,
            } => todo!(),
            adsb::DF::TisB { cf, pi } => todo!(),
            adsb::DF::ExtendedQuitterMilitaryApplication { af } => todo!(),
            adsb::DF::ModeSExtendedSquitter {
                df,
                capability,
                icao,
                type_code,
                adsb_data,
                parity,
            } => todo!("parse: {frame:?}"),
            _ => {}
        }
    }
}

#[derive(Debug)]
pub struct AircraftState {
    pub icao_address: IcaoAddress,

    pub last_seen: Timestamped<()>,

    pub callsign: Option<Timestamped<String>>,
    pub squawk: Option<Timestamped<Squawk>>,

    pub position: Option<Timestamped<Position>>,
    pub altitude_barometric: Option<Timestamped<u16>>,
    pub altitude_gnss: Option<Timestamped<u16>>,
    pub ground_speed: Option<Timestamped<GroundSpeed>>,
    pub track: Option<Timestamped<f64>>,
    pub vertical_rate: Option<Timestamped<f64>>,

    pub airborne_position_even: Option<adsb::Altitude>,
    pub airborne_position_odd: Option<adsb::Altitude>,

    pub mode_s_packets: Vec<adsb::DF>,
}

impl AircraftState {
    pub fn new(icao_address: IcaoAddress, time: DateTime<Utc>) -> Self {
        Self {
            icao_address,
            last_seen: Timestamped {
                last_update: time,
                value: (),
            },
            callsign: Default::default(),
            squawk: Default::default(),
            position: Default::default(),
            altitude_barometric: Default::default(),
            altitude_gnss: Default::default(),
            ground_speed: Default::default(),
            track: Default::default(),
            vertical_rate: Default::default(),
            airborne_position_even: None,
            airborne_position_odd: None,
            mode_s_packets: vec![],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Position {
    pub latitude: f64,
    pub longitude: f64,
    pub source: PositionSource,
}

impl From<adsb::cpr::Position> for Position {
    fn from(value: adsb::cpr::Position) -> Self {
        Self {
            latitude: value.latitude,
            longitude: value.longitude,
            source: PositionSource::Adbs,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PositionSource {
    Adbs,
    Mlat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AltitudeType {
    Barometric,
    Gnss,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroundSpeed {}

#[derive(Clone, Copy, Debug)]
pub struct Timestamped<T> {
    pub last_update: DateTime<Utc>,
    pub value: T,
}

trait UpdateTimestamped<T> {
    fn update_with(&mut self, time: DateTime<Utc>, value: impl FnOnce() -> T) -> bool;

    fn update(&mut self, time: DateTime<Utc>, value: T) -> bool {
        self.update_with(time, move || value)
    }
}

impl<T> UpdateTimestamped<T> for Timestamped<T> {
    fn update_with(&mut self, time: DateTime<Utc>, value: impl FnOnce() -> T) -> bool {
        if self.last_update < time {
            self.value = value();
            true
        }
        else {
            false
        }
    }
}

impl<T> UpdateTimestamped<T> for Option<Timestamped<T>> {
    fn update_with(&mut self, time: DateTime<Utc>, value: impl FnOnce() -> T) -> bool {
        if let Some(current) = self {
            current.update_with(time, value)
        }
        else {
            *self = Some(Timestamped {
                last_update: time,
                value: value(),
            });
            true
        }
    }
}

#[derive(Debug)]
pub struct UpdateAircraftState<'a> {
    index: usize,
    state: &'a mut AircraftState,
    by_callsign: &'a mut HashMap<String, usize>,
    by_squawk: &'a mut HashMap<Squawk, usize>,
    time: DateTime<Utc>,
}

impl<'a> UpdateAircraftState<'a> {
    pub fn update_airborne_position(
        &mut self,
        position: &adsb::Altitude,
        altitude_type: AltitudeType,
    ) {
        // the transponder will transmit position data over 2 frames, alternating
        // between even and odd frames. so we need to buffer those.
        match position.odd_flag {
            adsb::CPRFormat::Even => self.state.airborne_position_even = Some(*position),
            adsb::CPRFormat::Odd => self.state.airborne_position_odd = Some(*position),
        }

        // if we have both even and odd position frames, we can determine the exact
        // global position
        if let Some(pair) = self
            .state
            .airborne_position_even
            .as_ref()
            .zip(self.state.airborne_position_odd.as_ref())
        {
            self.state.position.update(
                self.time,
                adsb::cpr::get_position(pair)
                    .expect("cpr decoding failed unexpectedly")
                    .into(),
            );
        }

        // update altitude
        if let Some(altitude) = position.alt {
            match altitude_type {
                AltitudeType::Barometric => {
                    self.state.altitude_barometric.update(self.time, altitude);
                }
                AltitudeType::Gnss => {
                    self.state.altitude_gnss.update(self.time, altitude);
                }
            }
        }
    }

    pub fn update_airborne_velocity(
        &mut self,
        time: DateTime<Utc>,
        velocity: &adsb::adsb::AirborneVelocity,
    ) {
        todo!();
    }

    pub fn update_aircraft_identification(&mut self, identification: &adsb::adsb::Identification) {
        self.update_callsign(&identification.cn);
    }

    pub fn update_callsign(&mut self, callsign: &str) {
        update_timestamped_option_with_index_update::<String, str>(
            &mut self.state.callsign,
            self.time,
            callsign,
            |old_callsign, new_callsign| {
                if let Some(old_callsign) = old_callsign {
                    self.by_callsign.remove(old_callsign);
                    self.by_callsign.insert(new_callsign.to_owned(), self.index);
                }
            },
        );
    }

    pub fn update_aircraft_status(&mut self, status: &adsb::adsb::AircraftStatus) {
        // note: i verified this by playing back a plane in adbs.lol's webinterface.
        let squawk =
            Squawk::from_u16_hex(status.squawk.try_into().expect("squawk more than 16 bits"));
        self.update_squawk(squawk);
    }

    pub fn update_squawk(&mut self, squawk: Squawk) {
        update_timestamped_option_with_index_update::<Squawk, Squawk>(
            &mut self.state.squawk,
            self.time,
            &squawk,
            |old_squawk, new_squawk| {
                if let Some(old_squawk) = old_squawk {
                    self.by_squawk.remove(old_squawk);
                    self.by_squawk.insert(*new_squawk, self.index);
                }
            },
        );
    }
}

impl<'a> Deref for UpdateAircraftState<'a> {
    type Target = AircraftState;

    fn deref(&self) -> &Self::Target {
        &*self.state
    }
}

impl<'a> DerefMut for UpdateAircraftState<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.state
    }
}

/// helper to update a `Option<Timestamped<T>>` state value that also checks if
/// the value actually changed. if it did, it calls a callback with old and new
/// value.
fn update_timestamped_option_with_index_update<T, U>(
    state_value: &mut Option<Timestamped<T>>,
    time: DateTime<Utc>,
    new_value: &U,
    update_index: impl FnOnce(Option<&T>, &U),
) where
    T: PartialEq<U>,
    U: ?Sized,
    U: ToOwned<Owned = T>,
{
    if let Some(old_value) = state_value {
        if &old_value.value != new_value {
            update_index(Some(&old_value.value), new_value);
            old_value.value = new_value.to_owned();
        }
        old_value.last_update = time;
    }
    else {
        update_index(None, new_value);
        *state_value = Some(Timestamped {
            last_update: time,
            value: new_value.to_owned(),
        });
    }
}
