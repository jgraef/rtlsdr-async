//! Gillham code
//!
//! We could implement the gillham code once, but depending on where it is used,
//! the data bits are still jumbled. So instead of bit-shuffling twice, we just
//! do it once, with a function for each specific case.
//!
//! <https://en.wikipedia.org/wiki/Gillham_code>

/// Decodes 13-bit identity code as is used in DF5, DF21, and Mode A?? (not
/// ADSB)
///
/// ```plain
/// input:  C1 A1 C2 A2 C4 A4 ID B1 D1 B2 D2 B4 D4
/// output: A4 A2 A1 B4 B2 B1 C4 C2 C1 D4 D2 D1
/// ```
///
/// the ID bit in the input is ignored (this is the IDENT flag)
pub fn decode_gillham_id13(code: u16) -> u16 {
    let mut value = 0;

    // C1
    if code & 0b1000000000000 != 0 {
        value |= 0b000_000_001_000;
    }
    // A1
    if code & 0b0100000000000 != 0 {
        value |= 0b001_000_000_000;
    }
    // C2
    if code & 0b0010000000000 != 0 {
        value |= 0b000_000_010_000;
    }
    // A2
    if code & 0b0001000000000 != 0 {
        value |= 0b010_000_000_000;
    }
    // C4
    if code & 0b0000100000000 != 0 {
        value |= 0b000_000_100_000;
    }
    // A4
    if code & 0b0000010000000 != 0 {
        value |= 0b100_000_000_000;
    }
    // B1
    if code & 0b0000000100000 != 0 {
        value |= 0b000_001_000_000;
    }
    // D1
    if code & 0b0000000010000 != 0 {
        value |= 0b000_000_000_001;
    }
    // B2
    if code & 0b0000000001000 != 0 {
        value |= 0b000_010_000_000;
    }
    // D2
    if code & 0b0000000000100 != 0 {
        value |= 0b000_000_000_010;
    }
    // B4
    if code & 0b0000000000010 != 0 {
        value |= 0b000_100_000_000;
    }
    // D4
    if code & 0b0000000000001 != 0 {
        value |= 0b000_000_000_100;
    }

    value
}

/// Decodes 13-bit altitude code as is used in DF0, DF4, DF16, DF20, and Mode
/// C?? (not ADSB)
///
/// ```plain
/// bit:     0  1  2  3  4  5  6  7  8  9 10 11 12
/// input:  C1 A1 C2 A2 C4 A4  M B1  Q B2 D2 B4 D4
/// output: D2 D4 A1 A2 A4 B1 B2 B4 C1 C2 C4
/// ```
pub fn decode_gillham_ac13(code: u16) -> u16 {
    let mut value = 0;

    // C1
    if code & 0b1000000000000 != 0 {
        value |= 0b00000000100;
    }
    // A1
    if code & 0b0100000000000 != 0 {
        value |= 0b00100000000;
    }
    // C2
    if code & 0b0010000000000 != 0 {
        value |= 0b00000000010;
    }
    // A2
    if code & 0b0001000000000 != 0 {
        value |= 0b00010000000;
    }
    // C4
    if code & 0b0000100000000 != 0 {
        value |= 0b00000000001;
    }
    // A4
    if code & 0b0000010000000 != 0 {
        value |= 0b00001000000;
    }
    // B1
    if code & 0b0000000100000 != 0 {
        value |= 0b00000100000;
    }
    // B2
    if code & 0b0000000001000 != 0 {
        value |= 0b00000010000;
    }
    // D2
    if code & 0b0000000000100 != 0 {
        value |= 0b00000000001;
    }
    // B4
    if code & 0b0000000000010 != 0 {
        value |= 0b00000001000;
    }
    // D4
    if code & 0b0000000000001 != 0 {
        value |= 0b01000000000;
    }

    value
}

/// Decodes 12-bit altitude code as is used in ADSB-B AirbornePosition frames.
///
/// Page 59
///
/// ```plain
/// bit:     0  1  2  3  4  5  6  7  8  9 10 11
/// input:  C1 A1 C2 A2 C4 A4 B1  Q B2 D2 B4 D4
/// output: D2 D4 A1 A2 A4 B1 B2 B4 C1 C2 C4
/// ```
pub fn decode_gillham_ac12(code: u16) -> u16 {
    let mut value = 0;

    // C1
    if code & 0b100000000000 != 0 {
        value |= 0b00000000100;
    }
    // A1
    if code & 0b010000000000 != 0 {
        value |= 0b00100000000;
    }
    // C2
    if code & 0b001000000000 != 0 {
        value |= 0b00000000010;
    }
    // A2
    if code & 0b000100000000 != 0 {
        value |= 0b00010000000;
    }
    // C4
    if code & 0b000010000000 != 0 {
        value |= 0b00000000001;
    }
    // A4
    if code & 0b000001000000 != 0 {
        value |= 0b00001000000;
    }
    // B1
    if code & 0b000000100000 != 0 {
        value |= 0b00000100000;
    }
    // B2
    if code & 0b000000001000 != 0 {
        value |= 0b00000010000;
    }
    // D2
    if code & 0b000000000100 != 0 {
        value |= 0b00000000001;
    }
    // B4
    if code & 0b000000000010 != 0 {
        value |= 0b00000001000;
    }
    // D4
    if code & 0b000000000001 != 0 {
        value |= 0b01000000000;
    }

    value
}
#[cfg(test)]
mod tests {
    use crate::source::mode_s::util::gillham::decode_gillham_id13;

    #[test]
    fn it_decodes_id13() {
        assert_eq!(decode_gillham_id13(2214), 2882); // squawk 5502
        assert_eq!(decode_gillham_id13(2048), 512); // squawk 1000
        assert_eq!(decode_gillham_id13(5147), 413); // squawk 0635        
    }

    //#[test]
    //fn it_decodes_ac13() {
    //    todo!();
    //}
}
