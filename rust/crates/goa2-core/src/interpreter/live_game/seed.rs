//! CPython's seed and bounded draw for rules-v9 setup compatibility.
use sha2::{Digest, Sha512};

const N: usize = 624;

pub(super) struct SetupRng {
    state: [u32; N],
    index: usize,
}

impl SetupRng {
    pub(super) fn integer(seed: u64) -> Self {
        let words = [seed as u32, (seed >> 32) as u32];
        Self::from_words(&words[..if words[1] == 0 { 1 } else { 2 }])
    }

    pub(super) fn text(seed: &str) -> Self {
        // random.Random.seed uses int.from_bytes(seed + sha512(seed), "big").
        let hash = Sha512::digest(seed.as_bytes());
        let bytes = [seed.as_bytes(), hash.as_slice()].concat();
        let words = bytes
            .rchunks(4)
            .map(|chunk| {
                let mut word = [0; 4];
                word[4 - chunk.len()..].copy_from_slice(chunk);
                u32::from_be_bytes(word)
            })
            .collect::<Vec<_>>();
        Self::from_words(&words)
    }

    fn from_words(key: &[u32]) -> Self {
        let mut state = [0u32; N];
        state[0] = 19_650_218;
        for i in 1..N {
            let previous = state[i - 1];
            state[i] = (previous ^ (previous >> 30))
                .wrapping_mul(1_812_433_253)
                .wrapping_add(i as u32);
        }
        let mut i = 1;
        for j in (0..key.len()).cycle().take(N.max(key.len())) {
            let previous = state[i - 1];
            state[i] = (state[i] ^ (previous ^ (previous >> 30)).wrapping_mul(1_664_525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            if i == N {
                state[0] = state[N - 1];
                i = 1;
            }
        }
        for _ in 1..N {
            let previous = state[i - 1];
            state[i] = (state[i] ^ (previous ^ (previous >> 30)).wrapping_mul(1_566_083_941))
                .wrapping_sub(i as u32);
            i += 1;
            if i == N {
                state[0] = state[N - 1];
                i = 1;
            }
        }
        state[0] = 0x8000_0000;
        Self { state, index: N }
    }

    fn next(&mut self) -> u32 {
        if self.index == N {
            for i in 0..N {
                let word = (self.state[i] & 0x8000_0000) | (self.state[(i + 1) % N] & 0x7fff_ffff);
                self.state[i] = self.state[(i + 397) % N]
                    ^ (word >> 1)
                    ^ if word & 1 == 0 { 0 } else { 0x9908_b0df };
            }
            self.index = 0;
        }
        let mut word = self.state[self.index];
        self.index += 1;
        word ^= word >> 11;
        word ^= (word << 7) & 0x9d2c_5680;
        word ^= (word << 15) & 0xefc6_0000;
        word ^ (word >> 18)
    }

    pub(super) fn below(&mut self, upper: u32) -> usize {
        let shift = upper.leading_zeros();
        loop {
            let value = self.next() >> shift;
            if value < upper {
                return value as usize;
            }
        }
    }
}
