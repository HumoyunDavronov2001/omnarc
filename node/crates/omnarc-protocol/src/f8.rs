//! Hidden states as 8-bit floats on the wire (SPEC EN-19): E4M3 (the OCP "FN" variant: 4 exponent bits, 3 mantissa
//! bits, no infinities, 448 at most) with one f32 scale per token, so each token's largest value lands on 448. A
//! frame then carries a quarter of f32's bytes and half of f16's; the precision lost is about one part in sixteen
//! per value, against a per-token scale.

/// The largest finite E4M3 value.
pub const MAX: f32 = 448.0;

/// The value an E4M3 byte stands for.
#[must_use]
pub fn decode(byte: u8) -> f32 {
    let sign = if byte & 0x80 == 0 { 1.0 } else { -1.0 };
    let exp = i32::from((byte >> 3) & 0x0f);
    let mant = f32::from(byte & 0x07);
    let magnitude = if exp == 0 {
        // subnormal: steps of 2^-9
        mant * (2.0_f32).powi(-9)
    } else {
        (1.0 + mant / 8.0) * (2.0_f32).powi(exp - 7)
    };
    sign * magnitude
}

/// The nearest E4M3 byte to `value`, saturating at ±448.
#[must_use]
pub fn encode(value: f32) -> u8 {
    let sign: u8 = if value.is_sign_negative() { 0x80 } else { 0 };
    let a = value.abs();
    if !a.is_finite() {
        return sign | 0x7e;
    }
    if a >= MAX {
        return sign | 0x7e;
    }
    // below the smallest normal (2^-6) the steps are 2^-9 apart
    if a < (2.0_f32).powi(-6) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let m = (a / (2.0_f32).powi(-9)).round_ties_even() as u8;
        // eight steps of 2^-9 are the smallest normal, which the same bits spell
        return sign | m.min(8);
    }
    #[allow(clippy::cast_possible_truncation)]
    let mut e = a.log2().floor() as i32;
    let mut mant = (a / (2.0_f32).powi(e) - 1.0) * 8.0;
    mant = mant.round_ties_even();
    if mant >= 8.0 {
        mant = 0.0;
        e += 1;
    }
    if e > 8 || (e == 8 && mant > 6.0) {
        return sign | 0x7e;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bits = ((e + 7) as u8) << 3 | mant as u8;
    sign | bits
}

/// `tokens` rows of `hidden` f32 values as the wire carries them: one f32 scale per token, then a byte per value.
#[must_use]
pub fn pack(values: &[f32], hidden: usize) -> Vec<u8> {
    let rows = values.chunks_exact(hidden.max(1));
    let tokens = rows.len();
    let mut out = Vec::with_capacity(tokens * 4 + values.len());
    let mut scales = Vec::with_capacity(tokens);
    for row in values.chunks_exact(hidden.max(1)) {
        let peak = row.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        let scale = if peak > 0.0 && peak.is_finite() { peak / MAX } else { 1.0 };
        scales.push(scale);
        out.extend_from_slice(&scale.to_le_bytes());
    }
    for (row, scale) in values.chunks_exact(hidden.max(1)).zip(scales) {
        out.extend(row.iter().map(|v| encode(v / scale)));
    }
    out
}

/// The f32 values of a payload made by [`pack`]; `None` when its length does not fit `hidden`.
#[must_use]
pub fn unpack(payload: &[u8], hidden: usize) -> Option<Vec<f32>> {
    let row = 4 + hidden;
    if hidden == 0 || !payload.len().is_multiple_of(row) {
        return None;
    }
    let tokens = payload.len() / row;
    let (scales, bytes) = payload.split_at(tokens * 4);
    let scales: Vec<f32> = scales.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    Some(bytes.chunks_exact(hidden).zip(scales).flat_map(|(row, s)| row.iter().map(move |b| decode(*b) * s)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_but_nan_survives_a_round_trip() {
        for byte in 0..=255u8 {
            if byte & 0x7f == 0x7f {
                continue; // NaN
            }
            let value = decode(byte);
            // zero has two signs; both mean zero
            if value == 0.0 {
                assert_eq!(encode(value) & 0x7f, 0);
            } else {
                assert_eq!(encode(value), byte, "{byte:#04x} = {value}");
            }
        }
        assert_eq!(decode(0x7e), 448.0);
        assert_eq!(encode(1e9), 0x7e, "saturates");
        assert_eq!(encode(-1e9), 0xfe);
    }

    #[test]
    fn a_hidden_state_comes_back_within_a_sixteenth_of_its_largest_value() {
        let hidden = 64;
        let values: Vec<f32> = (0..hidden * 3)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let x = i as f32;
                (x * 0.37).sin() * (1.0 + (x / 17.0).cos()) * if i < hidden { 1e-3 } else { 40.0 }
            })
            .collect();
        let payload = pack(&values, hidden);
        assert_eq!(payload.len(), 3 * (4 + hidden), "a scale and a byte per value");
        let back = unpack(&payload, hidden).unwrap();
        for (row, original) in back.chunks(hidden).zip(values.chunks(hidden)) {
            let peak = original.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
            for (b, o) in row.iter().zip(original) {
                // three mantissa bits: within 1/16 of a value, and tiny values within a step of the per-token scale
                assert!((b - o).abs() <= o.abs() / 16.0 + peak / MAX * (2.0_f32).powi(-9), "{o} came back as {b}");
            }
        }
        assert!(unpack(&payload[1..], hidden).is_none());
    }
}
