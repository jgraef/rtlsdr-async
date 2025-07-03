use std::collections::{
    HashMap,
    hash_map,
};

use adsb_index_api_types::{
    IcaoAddress,
    Squawk,
};
use chrono::{
    DateTime,
    TimeDelta,
    Utc,
};

use crate::{
    source::mode_s::{
        self,
        VerticalStatus,
        adsb::{
            self,
            Callsign,
            cpr::{
                self,
                Decoder,
            },
        },
    },
    util::sparse_list::SparseList,
};

#[derive(Debug, Default)]
pub struct State {
    aircraft: SparseList<AircraftState>,
    indices: AircraftIndices,
}

impl State {
    pub fn update_aircraft(
        &mut self,
        icao_address: IcaoAddress,
        time: DateTime<Utc>,
    ) -> UpdateAircraftState<'_> {
        let (index, state) = match self.indices.by_icao_address.entry(icao_address) {
            hash_map::Entry::Occupied(occupied) => {
                let index = *occupied.get();
                let state = &mut self.aircraft[index];
                state.last_seen.update(time, ());
                (index, state)
            }
            hash_map::Entry::Vacant(vacant) => {
                let (index, state) = self
                    .aircraft
                    .insert_and_get_mut(AircraftState::new(icao_address, time));
                vacant.insert(index);
                (index, state)
            }
        };

        UpdateAircraftState {
            index,
            state,
            indices: &mut self.indices,
            time,
        }
    }

    pub fn iter_aircraft(&self) -> impl Iterator<Item = &AircraftState> {
        self.aircraft.iter()
    }

    pub fn update_mlat_position(
        &mut self,
        icao_address: IcaoAddress,
        time: DateTime<Utc>,
        position: Position,
    ) {
        let aircraft = self.update_aircraft(icao_address, time);
        aircraft.state.position.update(time, position);
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
            mode_s::Frame::MilitaryExtendedSquitter(_military_extended_squitter) => {
                todo!("military: {frame:#?}");

                //self.update_with_adsb(time, *address_announced,
                // adsb_message);
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
            adsb::Message::SurfacePosition(surface_position) => {
                aircraft.update_surface_position(surface_position)
            }
            adsb::Message::AirbornePosition(airborne_position) => {
                aircraft.update_airborne_position(airborne_position)
            }
            adsb::Message::AirborneVelocity(airborne_velocity) => {
                aircraft.update_airborne_velocity(airborne_velocity)
            }
            adsb::Message::AircraftStatus(aircraft_status) => {
                aircraft.update_aircraft_status(aircraft_status)
            }
            _ => {}
        }
    }
}

#[derive(Debug, Default)]
struct AircraftIndices {
    by_icao_address: HashMap<IcaoAddress, usize>,
    by_callsign: HashMap<Callsign, usize>,
    by_squawk: HashMap<Squawk, usize>,
}

#[derive(Debug)]
pub struct AircraftState {
    pub icao_address: IcaoAddress,

    pub last_seen: Timestamped<()>,

    pub callsign: Option<Timestamped<Callsign>>,
    pub squawk: Option<Timestamped<Squawk>>,

    // latitude and longitude
    pub position: Option<Timestamped<Position>>,

    // in ft
    pub altitude_barometric: Option<Timestamped<i32>>,

    // in m
    pub altitude_gnss: Option<Timestamped<i32>>,

    // in kt
    pub ground_speed: Option<Timestamped<f64>>,
    pub airspeed: Option<Timestamped<f64>>,

    // in radians, clockwise
    pub track: Option<Timestamped<f64>>,
    pub magnetic_heading: Option<Timestamped<f64>>,

    pub vertical_status: Option<VerticalStatus>,

    pub cpr_decoder: Decoder<DateTime<Utc>>,
}

impl AircraftState {
    pub fn new(icao_address: IcaoAddress, time: DateTime<Utc>) -> Self {
        Self {
            icao_address,
            last_seen: Timestamped {
                last_update: time,
                value: (),
            },
            callsign: None,
            squawk: None,
            position: None,
            altitude_barometric: None,
            altitude_gnss: None,
            ground_speed: None,
            airspeed: None,
            track: None,
            magnetic_heading: None,
            vertical_status: None,
            cpr_decoder: Default::default(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Position {
    pub latitude: f64,
    pub longitude: f64,
    pub source: PositionSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PositionSource {
    Gnss,
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
    indices: &'a mut AircraftIndices,
    time: DateTime<Utc>,
}

impl<'a> UpdateAircraftState<'a> {
    pub fn update_surface_position(&mut self, surface_position: &adsb::SurfacePosition) {
        self.update_position(&surface_position.cpr, VerticalStatus::Ground);

        if let Some(speed) = surface_position.movement.decode() {
            self.state.ground_speed.update(self.time, speed);
        }

        if let Some(track) = surface_position
            .ground_track
            .map(|track| track.as_radians())
        {
            self.state.track.update(self.time, track);
        }

        self.state.vertical_status = Some(VerticalStatus::Ground);
    }

    pub fn update_airborne_position(&mut self, airborne_position: &adsb::AirbornePosition) {
        // update position
        if let Some(cpr) = &airborne_position.cpr {
            self.update_position(cpr, VerticalStatus::Ground);
        }

        // update altitude
        if let Some(altitude) = airborne_position.altitude() {
            match altitude {
                adsb::Altitude::Barometric(altitude) => {
                    self.state.altitude_barometric.update(self.time, altitude);
                }
                adsb::Altitude::Gnss(altitude) => {
                    self.state.altitude_gnss.update(self.time, altitude);
                }
            }
        }

        self.state.vertical_status = Some(VerticalStatus::Airborne);
    }

    pub fn update_airborne_velocity(&mut self, velocity: &adsb::AirborneVelocity) {
        match velocity.velocity_type {
            adsb::VelocityType::GroundSpeed(ground_speed) => {
                if let Some([vx, vy]) = ground_speed.components(velocity.supersonic) {
                    let vx = vx as f64;
                    let vy = vy as f64;
                    let ground_speed = vx.hypot(vy);
                    let track = vx.atan2(vy);
                    self.state.ground_speed.update(self.time, ground_speed);
                    self.state.track.update(self.time, track);
                }
            }
            adsb::VelocityType::Airspeed(airspeed) => {
                if let Some(magnetic_heading) = &airspeed.magnetic_heading {
                    self.state
                        .magnetic_heading
                        .update(self.time, magnetic_heading.as_radians());

                    if let Some(airspeed) = &airspeed.airspeed_value {
                        self.state
                            .airspeed
                            .update(self.time, airspeed.as_knots(velocity.supersonic) as f64);
                    }
                }
            }
        }

        self.state.vertical_status = Some(VerticalStatus::Airborne);
    }

    pub fn update_aircraft_identification(
        &mut self,
        identification: &adsb::AircraftIdentification,
    ) {
        self.update_callsign(identification.callsign.decode_permissive());
    }

    pub fn update_position(&mut self, cpr: &cpr::Cpr, vertical_status: VerticalStatus) {
        let reference = self.state.position.as_ref().and_then(|reference| {
            // from MOPS appendix
            let max_age = match vertical_status {
                VerticalStatus::Airborne => TimeDelta::seconds(10),
                VerticalStatus::Ground => TimeDelta::seconds(50),
            };
            (self.time.signed_duration_since(reference.last_update) <= max_age).then_some(
                cpr::Position {
                    latitude: reference.value.latitude,
                    longitude: reference.value.longitude,
                },
            )
        });

        if let Some(position) =
            self.state
                .cpr_decoder
                .push(*cpr, vertical_status, self.time, reference)
        {
            self.state.position.update(
                self.time,
                Position {
                    latitude: position.latitude,
                    longitude: position.longitude,
                    source: PositionSource::Gnss,
                },
            );
        }
    }

    pub fn update_callsign(&mut self, callsign: Callsign) {
        update_timestamped_option_with_index_update::<Callsign, Callsign>(
            &mut self.state.callsign,
            self.time,
            &callsign,
            |old_callsign, new_callsign| {
                if let Some(old_callsign) = old_callsign {
                    self.indices.by_callsign.remove(old_callsign);
                    self.indices
                        .by_callsign
                        .insert(new_callsign.to_owned(), self.index);
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
                    self.indices.by_squawk.remove(old_squawk);
                    self.indices.by_squawk.insert(*new_squawk, self.index);
                }
            },
        );
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
