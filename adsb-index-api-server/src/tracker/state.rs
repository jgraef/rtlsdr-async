use std::collections::{
    HashMap,
    hash_map,
};

use adsb_deku::adsb::ADSB;
use adsb_index_api_types::{
    IcaoAddress,
    Squawk,
};
use chrono::{
    DateTime,
    Utc,
};

use crate::util::sparse_list::SparseList;

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
    ) -> &mut AircraftState {
        match self.by_icao_address.entry(icao_address) {
            hash_map::Entry::Occupied(occupied) => {
                let aircraft = &mut self.aircraft[*occupied.get()];
                aircraft.last_seen.update(time, ());
                aircraft
            }
            hash_map::Entry::Vacant(vacant) => {
                let (index, aircraft) = self
                    .aircraft
                    .insert_and_get_mut(AircraftState::new(icao_address, time));
                vacant.insert(index);
                aircraft
            }
        }
    }

    pub fn update_mlat_position(
        &mut self,
        icao_address: IcaoAddress,
        time: DateTime<Utc>,
        position: Position,
    ) {
        let aircraft = self.update_aircraft(icao_address, time);
        aircraft.position.update(time, position);
    }

    pub fn update_adsb_data(&mut self, time: DateTime<Utc>, packet: &ADSB) {
        let icao_address = packet.icao.into();
        let aircraft = self.update_aircraft(icao_address, time);
        todo!("update state from adsb packet");
    }
}

#[derive(Debug)]
pub struct AircraftState {
    pub icao_address: IcaoAddress,

    pub last_seen: Timestamped<()>,

    pub callsign: Option<Timestamped<String>>,
    pub squawk: Option<Timestamped<Squawk>>,

    pub position: Option<Timestamped<Position>>,
    pub altitude: Option<Timestamped<f32>>,
    pub ground_speed: Option<Timestamped<f32>>,
    pub track: Option<Timestamped<f32>>,
    pub vertical_rate: Option<Timestamped<f32>>,
}

impl AircraftState {
    pub fn new(icao_address: IcaoAddress, time: DateTime<Utc>) -> Self {
        Self {
            icao_address,
            last_seen: Timestamped { time, value: () },
            callsign: Default::default(),
            squawk: Default::default(),
            position: Default::default(),
            altitude: Default::default(),
            ground_speed: Default::default(),
            track: Default::default(),
            vertical_rate: Default::default(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Position {
    pub latitude: f32,
    pub longitude: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Timestamped<T> {
    pub time: DateTime<Utc>,
    pub value: T,
}

trait UpdateTimestamped<T> {
    fn update(&mut self, time: DateTime<Utc>, value: T) -> bool;
}

impl<T> UpdateTimestamped<T> for Timestamped<T> {
    fn update(&mut self, time: DateTime<Utc>, value: T) -> bool {
        if self.time < time {
            self.value = value;
            true
        }
        else {
            false
        }
    }
}

impl<T> UpdateTimestamped<T> for Option<Timestamped<T>> {
    fn update(&mut self, time: DateTime<Utc>, value: T) -> bool {
        if let Some(current) = self {
            current.update(time, value)
        }
        else {
            *self = Some(Timestamped { time, value });
            true
        }
    }
}
