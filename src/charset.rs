//! Unicode to teletext G0 conversion.
//!
//! We transmit the Swedish/Finnish/Hungarian national option (it has Å, Ä, Ö, É, Ü).
//! WST has no Danish/Norwegian option, so Æ, Ø, æ, ø are sent as their Swedish
//! look-alikes (Ä, Ö, ä, ö) for Level 1 decoders, plus an X/26 enhancement that puts the
//! real character from the Latin G2 set in that cell on Level 1.5+ decoders.

/// Header control bits C12, C13, C14 for the Swedish/Finnish/Hungarian option.
pub const NATIONAL_C12_C14: (bool, bool, bool) = (false, true, false);

/// Encode one character: (G0 code, optional G2 code for an X/26 substitution).
pub fn encode_char(c: char) -> (u8, Option<u8>) {
    match c {
        'Å' => (0x5D, None),
        'å' => (0x7D, None),
        'Ä' => (0x5B, None),
        'ä' => (0x7B, None),
        'Ö' => (0x5C, None),
        'ö' => (0x7C, None),
        'É' => (0x40, None),
        'é' => (0x60, None),
        'Ü' => (0x5E, None),
        'ü' => (0x7E, None),
        '¤' => (0x24, None),
        'Æ' => (0x5B, Some(0x61)),
        'æ' => (0x7B, Some(0x71)),
        'Ø' => (0x5C, Some(0x69)),
        'ø' => (0x7C, Some(0x79)),
        // Positions the national option takes over are not available as ASCII.
        '@' | '[' | '\\' | ']' | '^' | '`' | '{' | '|' | '}' | '~' | '$' => (b'?', None),
        c if (' '..='~').contains(&c) => (c as u8, None),
        _ => (b'?', None),
    }
}

/// Decode a G0 code back to a character (for previews).
pub fn decode_g0(b: u8) -> char {
    match b & 0x7F {
        0x24 => '¤',
        0x40 => 'É',
        0x5B => 'Ä',
        0x5C => 'Ö',
        0x5D => 'Å',
        0x5E => 'Ü',
        0x60 => 'é',
        0x7B => 'ä',
        0x7C => 'ö',
        0x7D => 'å',
        0x7E => 'ü',
        0x7F => '■',
        c if c < 0x20 => ' ',
        c => c as char,
    }
}

/// Decode the G2 codes we use.
pub fn decode_g2(b: u8) -> char {
    match b {
        0x61 => 'Æ',
        0x71 => 'æ',
        0x69 => 'Ø',
        0x79 => 'ø',
        _ => '?',
    }
}
