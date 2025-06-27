create table metadata (
    key text not null primary key,
    value jsonb not null
);

create table trace_info (
    time timestamptz not null,
    icao_address int not null,
    callsign text,
    squawk int,
    data_source char
);

create index index_trace_info_time on trace_info (time);
create index index_trace_info_icao_address on trace_info (icao_address);
create index index_trace_info_icao_address_time on trace_info (icao_address, time);
create index index_trace_info_callsign on trace_info (callsign);
create index index_trace_info_callsign_time on trace_info (callsign, time);
create index index_trace_info_squawk on trace_info (squawk);
create index index_trace_info_squawk_time on trace_info (squawk, time);


create table aircraft_registration (
    icao_address int not null primary key,
    registration text,
    model text
);

create index index_aircraft_registration_registration on aircraft_registration (registration);


create table aircraft_model (
    icao_code text not null primary key,
    name text,
    description text,
    wtc char
);


create table aircraft_tag (
    icao_address int not null,
    tag text not null,
    unique (icao_address, tag)
);

create index index_aircraft_tag_icao_address on aircraft_tag (icao_address);
create index index_aircraft_tag_tag on aircraft_tag (tag);
