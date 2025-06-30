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
    source::mode_s::{
        self,
        adsb::{
            self,
            Callsign,
        },
        cpr,
    },
    util::sparse_list::SparseList,
};

#[derive(Debug, Default)]
pub struct State {
    aircraft: SparseList<AircraftState>,
    by_icao_address: HashMap<IcaoAddress, usize>,
    by_callsign: HashMap<Callsign, usize>,
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

    pub fn update_with_mode_s(&mut self, time: DateTime<Utc>, frame: &mode_s::Frame) {
        //tracing::debug!("Mode-S frame: {frame:#?}");

        match frame {
            mode_s::Frame::AllCallReply(mode_s::AllCallReply {
                address_announced, ..
            }) => {
                self.update_aircraft(*address_announced, time);
            }
            mode_s::Frame::ExtendedSquitter(mode_s::ExtendedSquitter {
                address_announced,
                adsb_message,
                ..
            }) => {
                self.update_with_adsb(time, *address_announced, adsb_message);
            }
            mode_s::Frame::ExtendedSquitterNonTransponder(
                mode_s::ExtendedSquitterNonTransponder::AdsbWithIcaoAddress {
                    address_announced,
                    adsb_message,
                    ..
                },
            ) => {
                self.update_with_adsb(time, *address_announced, adsb_message);
            }
            mode_s::Frame::MilitaryExtendedSquitter(mode_s::MilitaryExtendedSquitter::Adsb {
                address_announced,
                adsb_message,
                ..
            }) => {
                self.update_with_adsb(time, *address_announced, adsb_message);
            }
            _ => {}
        }
    }

    pub fn update_with_adsb(
        &mut self,
        time: DateTime<Utc>,
        icao_address: IcaoAddress,
        message: &adsb::Message,
    ) {
        let mut aircraft = self.update_aircraft(icao_address, time);

        match message {
            adsb::Message::AircraftIdentification(aircraft_identification) => {
                aircraft.update_aircraft_identification(aircraft_identification)
            }
            adsb::Message::AirbornePosition(airborne_position) => {
                aircraft.update_airborne_position(airborne_position)
            }
            adsb::Message::AircraftStatus(aircraft_status) => {
                aircraft.update_aircraft_status(aircraft_status)
            }
            _ => {}
        }
    }
}

#[derive(Debug)]
pub struct AircraftState {
    pub icao_address: IcaoAddress,

    pub last_seen: Timestamped<()>,

    pub callsign: Option<Timestamped<Callsign>>,
    pub squawk: Option<Timestamped<Squawk>>,

    pub position: Option<Timestamped<Position>>,

    // in ft
    pub altitude_barometric: Option<Timestamped<i32>>,

    // in m
    pub altitude_gnss: Option<Timestamped<i32>>,

    // in kt
    //pub ground_speed: Option<Timestamped<GroundSpeed>>,
    pub track: Option<Timestamped<f64>>,
    pub vertical_rate: Option<Timestamped<f64>>,

    pub airborne_position_even: Option<Timestamped<cpr::CprPosition>>,
    pub airborne_position_odd: Option<Timestamped<cpr::CprPosition>>,

    pub frames: Vec<mode_s::Frame>,
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
            //ground_speed: Default::default(),
            track: Default::default(),
            vertical_rate: Default::default(),
            airborne_position_even: None,
            airborne_position_odd: None,
            frames: vec![],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Position {
    pub latitude: f64,
    pub longitude: f64,
    pub source: PositionSource,
}

impl From<cpr::Position> for Position {
    fn from(value: cpr::Position) -> Self {
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
    by_callsign: &'a mut HashMap<Callsign, usize>,
    by_squawk: &'a mut HashMap<Squawk, usize>,
    time: DateTime<Utc>,
}

impl<'a> UpdateAircraftState<'a> {
    pub fn update_airborne_position(&mut self, airborne_position: &adsb::AirbornePosition) {
        // the transponder will transmit position data over 2 frames, alternating
        // between even and odd frames. so we need to buffer those.
        match airborne_position.cpr.format {
            cpr::CprFormat::Even => &mut self.state.airborne_position_even,
            cpr::CprFormat::Odd => &mut self.state.airborne_position_odd,
        }
        .update(self.time, airborne_position.cpr.position);

        // if we have both even and odd position frames, we can determine the exact
        // global position
        // todo: if we already know its position, we can also use local decoding (e.g.
        // if global fails)
        if let Some(pair) = self
            .state
            .airborne_position_even
            .as_ref()
            .zip(self.state.airborne_position_odd.as_ref())
        {
            let most_recent = if pair.0.last_update > pair.1.last_update {
                cpr::CprFormat::Even
            }
            else {
                cpr::CprFormat::Odd
            };
            if let Ok(position) =
                cpr::decode_globally_unambigious(pair.0.value, pair.1.value, most_recent)
            {
                self.state.position.update(self.time, position.into());
            }
        }

        // update altitude
        if let Some(altitude) = airborne_position.altitude() {
            match altitude.altitude_type {
                adsb::AltitudeType::Barometric => {
                    self.state
                        .altitude_barometric
                        .update(self.time, altitude.altitude);
                }
                adsb::AltitudeType::Gnss => {
                    self.state
                        .altitude_gnss
                        .update(self.time, altitude.altitude);
                }
            }
        }
    }

    pub fn update_airborne_velocity(&mut self, velocity: &adsb::AirborneVelocity) {
        todo!();
    }

    pub fn update_aircraft_identification(
        &mut self,
        identification: &adsb::AircraftIdentification,
    ) {
        self.update_callsign(identification.callsign.decode_permissive());
    }

    pub fn update_callsign(&mut self, callsign: Callsign) {
        update_timestamped_option_with_index_update::<Callsign, Callsign>(
            &mut self.state.callsign,
            self.time,
            &callsign,
            |old_callsign, new_callsign| {
                if let Some(old_callsign) = old_callsign {
                    self.by_callsign.remove(old_callsign);
                    self.by_callsign.insert(new_callsign.to_owned(), self.index);
                }
            },
        );
    }

    pub fn update_aircraft_status(&mut self, status: &adsb::AircraftStatus) {
        match status {
            adsb::AircraftStatus::EmergencyPriorityStatusAndModeACode(
                adsb::EmergencyPriorityStatusAndModeACode { mode_a_code, .. },
            ) => {
                self.update_squawk(*mode_a_code);
            }
            _ => {}
        }
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
