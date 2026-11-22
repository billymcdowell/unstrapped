//! Tiny stateless PRNG helpers: everything is a pure function of `(seed, tag, key)`, so synthetic
//! physiology can be regenerated for any timestamp without stored state.

/// One SplitMix64 round.
pub fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Hash `(seed, tag, key)` to 64 bits.
pub fn hash(seed: u64, tag: u64, key: i64) -> u64 {
    splitmix(splitmix(seed ^ tag.wrapping_mul(0xD6E8_FEB8_6659_FD93)) ^ (key as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// Uniform in `[0, 1)`.
pub fn u01(seed: u64, tag: u64, key: i64) -> f64 {
    (hash(seed, tag, key) >> 11) as f64 / (1u64 << 53) as f64
}

/// Standard normal (Box-Muller).
pub fn gauss(seed: u64, tag: u64, key: i64) -> f64 {
    let h1 = hash(seed, tag, key);
    let h2 = splitmix(h1 ^ 0xA5A5_A5A5_5A5A_5A5A);
    let u1 = (((h1 >> 11) as f64) + 1.0) / ((1u64 << 53) as f64 + 1.0);
    let u2 = (h2 >> 11) as f64 / (1u64 << 53) as f64;
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

/// Random unit vector.
pub fn unit_vec(seed: u64, tag: u64, key: i64) -> [f64; 3] {
    let v = [gauss(seed, tag, key * 3), gauss(seed, tag, key * 3 + 1), gauss(seed, tag, key * 3 + 2)];
    normalize(v)
}

pub fn normalize(v: [f64; 3]) -> [f64; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / n, v[1] / n, v[2] / n]
}

pub fn smoothstep(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}
