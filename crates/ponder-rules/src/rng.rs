//! A small seeded generator (xoshiro256**): deterministic samples without a dependency.

pub struct Rng([u64; 4]);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut s = seed;
        let mut next = || {
            s = s.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        Rng([next(), next(), next(), next()])
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.0;
        let out = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        out
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// An index drawn with probability proportional to its weight; `None` if all are zero.
    pub fn pick(&mut self, weights: impl Iterator<Item = f64> + Clone) -> Option<usize> {
        let total: f64 = weights.clone().sum();
        if total <= 0.0 {
            return None;
        }
        let mut x = self.unit() * total;
        let mut last = None;
        for (i, w) in weights.enumerate() {
            if w > 0.0 {
                last = Some(i);
                if x < w {
                    return Some(i);
                }
                x -= w;
            }
        }
        last
    }
}
